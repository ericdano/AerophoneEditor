//! The editor's state and its conversation with the instrument, without any
//! GUI code: the window calls [`Editor::tick`] every frame and the
//! `set_*`/`select_*`/`save_*` methods on user actions.
//!
//! Editing model: every change is written straight into the instrument's
//! Temporary Scene / Temporary Tone, so you hear it at once. Nothing is
//! stored until "Save to user scene", which sends Roland's write command.

use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, Instant};

use ae30_core::image::{name_bytes, Area, Image};
use ae30_core::params::{data, Param};
use ae30_core::scene::SceneRef;
use ae30_core::session::{Event, SceneName, Session};
use ae30_core::sysex::{Addr, Identity, Model};
use ae30_core::syx;

use crate::midi::{self, Link, Waker};

const TAG_SETUP: u64 = 1;
const TAG_SCENE: u64 = 2;
const TAG_SYSTEM: u64 = 3;
const TAG_TONE: u64 = 10; // + part
const TAG_BACKUP: u64 = 100;

/// Edits to the same setting this close together undo as one step.
const UNDO_MERGE: Duration = Duration::from_millis(1200);
/// The instrument needs a moment to load a newly chosen scene or tone.
const SETTLE: Duration = Duration::from_millis(300);

/// Which local copy an edit applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Target {
    Scene,
    Tone(u8),
    System,
}

impl Target {
    pub fn area(self) -> Area {
        match self {
            Target::Scene => Area::TempScene,
            Target::Tone(p) => Area::TempTone(p),
            Target::System => Area::System,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Edit {
    pub target: Target,
    pub addr: Addr,
    pub old: Vec<u8>,
    pub new: Vec<u8>,
    pub label: String,
    at: Instant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Conn {
    Offline,
    Connecting,
    Online(Identity),
    Failed(String),
}

/// A backup of user scenes in progress.
///
/// Reading stored scenes: one RQ1 starting at a user scene makes the AE-30
/// stream that scene and many following ones (in bursts, ~1.3 s per scene),
/// so the backup keeps everything that arrives and, when the stream ends,
/// starts again at the first scene still missing. A scene the stream won't
/// deliver is read block by block, and skipped if that fails too.
#[derive(Debug)]
pub struct Backup {
    pub path: std::path::PathBuf,
    /// Scenes to save (0-based), in order.
    pub want: Vec<u16>,
    images: Vec<Option<Image>>,
    start: Option<u16>,
    block_mode: bool,
    gave_up: Vec<u16>,
    pub started: Instant,
}

impl Backup {
    fn loaded(&self, n: u16) -> bool {
        self.images[usize::from(n)]
            .as_ref()
            .is_some_and(Image::is_loaded)
    }

    /// (scenes finished, scenes wanted)
    pub fn progress(&self) -> (usize, usize) {
        let done = self
            .want
            .iter()
            .filter(|&&n| self.loaded(n) || self.gave_up.contains(&n))
            .count();
        (done, self.want.len())
    }

    /// Estimated time left, once a few scenes are in.
    pub fn eta(&self) -> Option<Duration> {
        let (done, total) = self.progress();
        (done >= 3).then(|| {
            self.started
                .elapsed()
                .mul_f64((total - done) as f64 / done as f64)
        })
    }

    fn next_missing(&self) -> Option<u16> {
        self.want
            .iter()
            .copied()
            .find(|&n| !self.loaded(n) && !self.gave_up.contains(&n))
    }
}

pub struct Editor {
    link: Option<Link>,
    pub session: Session,
    pub conn: Conn,
    pub setup: Image,
    pub scene: Image,
    pub tones: Vec<Image>,
    pub system: Image,
    /// User scene names, 600 slots ("" = not known yet).
    pub user_names: Vec<String>,
    /// Preset scenes in panel order.
    pub preset_names: Vec<SceneName>,
    /// User tone names, 512 slots, once listed.
    pub user_tones: Option<Vec<String>>,
    pub undo: Vec<Edit>,
    pub redo: Vec<Edit>,
    /// The Temporary Scene has changes not saved to a user scene.
    pub dirty: bool,
    pub status: Option<(String, Instant, bool)>,
    pub backup: Option<Backup>,
    /// Breath controller value and when it last moved (from the unit's MIDI out).
    pub breath: (u8, Instant),
    pub note: Option<u8>,
    /// Pitch bend from the unit (-8192..8191) and when it last moved.
    pub bend: (i16, Instant),
    pub last_rx: Option<Instant>,
    pending: HashMap<u64, usize>,
    scene_reload: Option<Instant>,
    tone_reload: [Option<Instant>; 4],
    current: Option<SceneRef>,
    loading_names: bool,
}

impl Default for Editor {
    fn default() -> Self {
        Self::new()
    }
}

impl Editor {
    pub fn new() -> Self {
        let now = Instant::now();
        Editor {
            link: None,
            session: Session::default(),
            conn: Conn::Offline,
            setup: Image::new(Area::Setup),
            scene: Image::new(Area::TempScene),
            tones: (1..=4).map(|p| Image::new(Area::TempTone(p))).collect(),
            system: Image::new(Area::System),
            user_names: vec![String::new(); 600],
            preset_names: load_preset_cache(),
            user_tones: None,
            undo: Vec::new(),
            redo: Vec::new(),
            dirty: false,
            status: None,
            backup: None,
            breath: (0, now),
            note: None,
            bend: (0, now),
            last_rx: None,
            pending: HashMap::new(),
            scene_reload: None,
            tone_reload: [None; 4],
            current: None,
            loading_names: false,
        }
    }

    // ---- connection --------------------------------------------------------

    pub fn connect(&mut self, port: Option<&str>, waker: Option<Waker>) {
        self.disconnect();
        let hints: Vec<&str> = match port {
            Some(p) => vec![p],
            None => midi::DEFAULT_PORT_HINTS.to_vec(),
        };
        match Link::open(&hints, waker) {
            Ok(link) => {
                self.link = Some(link);
                self.conn = Conn::Connecting;
                self.session.identify();
            }
            Err(e) => self.conn = Conn::Failed(e),
        }
    }

    pub fn disconnect(&mut self) {
        self.link = None;
        self.session = Session::default();
        self.pending.clear();
        self.conn = Conn::Offline;
        self.backup = None;
    }

    pub fn is_online(&self) -> bool {
        matches!(self.conn, Conn::Online(_))
    }

    pub fn port_name(&self) -> Option<&str> {
        self.link.as_ref().map(|l| l.in_label.as_str())
    }

    pub fn model(&self) -> Option<Model> {
        match &self.conn {
            Conn::Online(id) => Some(id.model),
            _ => None,
        }
    }

    pub fn is_loading(&self) -> bool {
        self.pending.values().any(|&n| n > 0) || self.loading_names
    }

    pub fn tell(&mut self, msg: impl Into<String>) {
        self.status = Some((msg.into(), Instant::now(), false));
    }

    pub fn warn(&mut self, msg: impl Into<String>) {
        self.status = Some((msg.into(), Instant::now(), true));
    }

    /// Do pending MIDI work. Returns how soon to call again.
    pub fn tick(&mut self) -> Option<Duration> {
        let now = Instant::now();
        if let Some(link) = &mut self.link {
            match link.pump(&mut self.session) {
                Ok(other) => {
                    for m in other {
                        self.channel_message(&m, now);
                    }
                }
                Err(e) => {
                    self.disconnect();
                    self.conn = Conn::Failed(e);
                    return None;
                }
            }
        }
        if self.session.received_count > 0 {
            self.last_rx = Some(now);
        }
        for ev in self.session.take_events() {
            self.event(ev, now);
        }
        self.check_backup();
        // Flush what the events queued.
        if let Some(link) = &mut self.link {
            let _ = link.pump(&mut self.session);
        }
        if self.scene_reload.is_some_and(|t| now >= t) {
            self.scene_reload = None;
            self.load_scene();
        }
        for p in 0..4 {
            if self.tone_reload[p].is_some_and(|t| now >= t) {
                self.tone_reload[p] = None;
                self.load_tone(p as u8 + 1);
            }
        }
        let mut wake = self.session.next_wake(now);
        for t in self
            .tone_reload
            .iter()
            .chain([&self.scene_reload])
            .flatten()
        {
            let d = t.saturating_duration_since(now);
            wake = Some(wake.map_or(d, |w| w.min(d)));
        }
        wake
    }

    fn channel_message(&mut self, m: &[u8], now: Instant) {
        match (m.first().map(|s| s & 0xF0), m.get(1), m.get(2)) {
            (Some(0xB0), Some(&2 | &11), Some(&v)) => self.breath = (v, now),
            (Some(0xE0), Some(&lsb), Some(&msb)) => {
                self.bend = (((i16::from(msb) << 7) | i16::from(lsb)) - 8192, now)
            }
            (Some(0x90), Some(&n), Some(&v)) if v > 0 => self.note = Some(n),
            (Some(0x80), Some(&n), _) | (Some(0x90), Some(&n), Some(0)) if self.note == Some(n) => {
                self.note = None;
            }
            _ => {}
        }
    }

    fn done(&mut self, tag: u64) -> bool {
        match self.pending.get_mut(&tag) {
            Some(n) if *n > 0 => {
                *n -= 1;
                *n == 0
            }
            _ => false,
        }
    }

    fn event(&mut self, ev: Event, now: Instant) {
        match ev {
            Event::Identity(id) => {
                self.session.device = id.device;
                let first = !self.is_online();
                self.conn = Conn::Online(id);
                if first {
                    self.load_everything();
                }
            }
            Event::IdentityTimeout => {
                self.conn = Conn::Failed(
                    "The AE-30 didn't answer. Is it on, with its USB driver mode set to Generic?"
                        .into(),
                );
            }
            Event::Data { addr, data } => self.data(addr, &data, now),
            Event::ReadDone { tag, .. } => {
                if !self.done(tag) {
                    return;
                }
                match tag {
                    TAG_SCENE => self.finish_image(TAG_SCENE),
                    TAG_SETUP => self.finish_image(TAG_SETUP),
                    TAG_SYSTEM => self.finish_image(TAG_SYSTEM),
                    t if (TAG_TONE + 1..=TAG_TONE + 4).contains(&t) => self.finish_image(t),
                    TAG_BACKUP => self.backup_next(),
                    _ => {}
                }
            }
            Event::SceneNames { user, bank, scenes } => {
                if user {
                    for s in scenes {
                        let n = usize::from(s.lsb) * 50 + usize::from(s.pc);
                        if let Some(slot) = self.user_names.get_mut(n) {
                            slot.clone_from(&s.name);
                        }
                    }
                } else {
                    if bank == 0 {
                        self.preset_names.clear();
                    }
                    self.preset_names.extend(scenes);
                    if bank == 11 {
                        save_preset_cache(&self.preset_names);
                    }
                }
                if !user && bank == 11 || user && bank == 11 && !self.preset_names.is_empty() {
                    self.loading_names = false;
                }
            }
            Event::UserToneNames(names) => self.user_tones = Some(names),
        }
    }

    /// Place incoming data, and react to changes made on the instrument.
    fn data(&mut self, addr: Addr, bytes: &[u8], now: Instant) {
        let before = self.current_scene();
        self.setup.apply(addr, bytes);
        if self.setup.is_loaded() {
            let after = self.current_scene();
            if before.is_some() && after != before {
                // The scene was changed on the instrument (or by us).
                self.current = after;
                self.scene_reload = Some(now + SETTLE);
            }
        }
        let tone_before: Vec<_> = (1..=4).map(|p| self.part_tone(p)).collect();
        if !self.scene.apply(addr, bytes).is_empty() && self.scene.is_loaded() {
            for p in 1..=4u8 {
                if self.part_tone(p) != tone_before[usize::from(p - 1)] {
                    self.tone_reload[usize::from(p - 1)] = Some(now + SETTLE);
                }
            }
        }
        for t in &mut self.tones {
            t.apply(addr, bytes);
        }
        self.system.apply(addr, bytes);
        if let Some(b) = &mut self.backup {
            if let Some(Area::UserScene(n)) = syx::scene_area_of(addr) {
                b.images[usize::from(n)]
                    .get_or_insert_with(|| Image::new(Area::UserScene(n)))
                    .apply(addr, bytes);
            }
        }
    }

    // ---- loading -----------------------------------------------------------

    fn image_mut(&mut self, tag: u64) -> &mut Image {
        match tag {
            TAG_SETUP => &mut self.setup,
            TAG_SCENE => &mut self.scene,
            TAG_SYSTEM => &mut self.system,
            t => &mut self.tones[(t - TAG_TONE - 1) as usize],
        }
    }

    /// Read an area with one streamed request; `finish_image` follows up.
    fn stream(&mut self, tag: u64) {
        self.session.cancel_reads(|t| t != tag);
        let img = self.image_mut(tag);
        img.invalidate();
        let area = img.area;
        self.session.read_area(tag, area);
        self.pending.insert(tag, 1);
    }

    /// After a streamed read: fetch any blocks that didn't arrive.
    fn finish_image(&mut self, tag: u64) {
        let reads = self.image_mut(tag).missing_reads();
        if !reads.is_empty() && !self.pending.contains_key(&(tag | 1 << 32)) {
            self.pending.insert(tag | 1 << 32, 1);
            self.pending.insert(tag, reads.len());
            for (a, n) in reads {
                self.session.read(tag, a, n);
            }
            return;
        }
        self.pending.remove(&(tag | 1 << 32));
        match tag {
            TAG_SETUP => {
                self.current = self.current_scene();
            }
            TAG_SCENE => {
                self.dirty = false;
                self.undo.clear();
                self.redo.clear();
                for p in 1..=4 {
                    self.load_tone(p);
                }
            }
            _ => {}
        }
    }

    fn load_everything(&mut self) {
        self.stream(TAG_SETUP);
        self.load_scene();
        self.stream(TAG_SYSTEM);
        self.load_names();
    }

    pub fn load_scene(&mut self) {
        self.stream(TAG_SCENE);
        for t in &mut self.tones {
            t.invalidate();
        }
    }

    /// Read a part's temporary tone, if it is a ZEN-Core tone (the only
    /// kind whose insides are editable).
    pub fn load_tone(&mut self, part: u8) {
        let tag = TAG_TONE + u64::from(part);
        if self.tone_editable(part) {
            self.stream(tag);
        } else {
            self.tones[usize::from(part - 1)].invalidate();
            self.pending.remove(&tag);
        }
    }

    pub fn load_names(&mut self) {
        self.loading_names = true;
        for bank in 0..12 {
            self.session.list_scenes(true, bank);
        }
        if self.preset_names.is_empty() {
            for bank in 0..12 {
                self.session.list_scenes(false, bank);
            }
        }
    }

    pub fn load_user_tones(&mut self) {
        self.session.list_user_tones();
    }

    // ---- reading values ----------------------------------------------------

    pub fn image(&self, t: Target) -> &Image {
        match t {
            Target::Scene => &self.scene,
            Target::Tone(p) => &self.tones[usize::from(p - 1)],
            Target::System => &self.system,
        }
    }

    fn image_of(&mut self, t: Target) -> &mut Image {
        match t {
            Target::Scene => &mut self.scene,
            Target::Tone(p) => &mut self.tones[usize::from(p - 1)],
            Target::System => &mut self.system,
        }
    }

    pub fn current_scene(&self) -> Option<SceneRef> {
        let b = self.setup.blocks.first()?;
        if !b.is_loaded() {
            return None;
        }
        Some(SceneRef::from_bank(b.raw(0)?, b.raw(1)?, b.raw(2)?))
    }

    pub fn scene_name(&self) -> String {
        self.scene.scene_name().unwrap_or_default()
    }

    /// (MSB, LSB, PC) of a part's tone.
    pub fn part_tone(&self, part: u8) -> Option<(u8, u8, u8)> {
        let b = self.scene.block(&format!("Perf_part_{part}"))?;
        b.is_loaded()
            .then(|| (b.raw(2).unwrap(), b.raw(3).unwrap(), b.raw(4).unwrap()))
    }

    pub fn tone_editable(&self, part: u8) -> bool {
        (1..=4).contains(&part) && matches!(self.part_tone(part), Some((71, _, _)))
    }

    /// A readable name for a part's tone.
    pub fn tone_label(&self, part: u8) -> String {
        let Some((msb, lsb, pc)) = self.part_tone(part) else {
            return "…".into();
        };
        if msb == 71 && (8..=11).contains(&lsb) {
            let idx = usize::from(lsb - 8) * 128 + usize::from(pc);
            if let Some(n) = self.user_tones.as_ref().and_then(|t| t.get(idx)) {
                if !n.is_empty() {
                    return n.clone();
                }
            }
            let t = &self.tones[usize::from(part - 1)];
            if let Some(n) = t.scene_name() {
                return n;
            }
            return format!("User Tone {}", idx + 1);
        }
        if let Some(n) = data().tone_name(msb, lsb, pc) {
            return n.to_string();
        }
        format!("{} #{}", ae30_core::scene::tone_bank_name(msb, lsb), pc + 1)
    }

    pub fn value(&self, t: Target, block: &str, p: &Param) -> Option<i32> {
        self.image(t).block(block)?.value(p)
    }

    // ---- editing -----------------------------------------------------------

    fn record(&mut self, target: Target, addr: Addr, old: Vec<u8>, new: Vec<u8>, label: &str) {
        let now = Instant::now();
        self.redo.clear();
        if let Some(last) = self.undo.last_mut() {
            if last.target == target
                && last.addr == addr
                && last.new.len() == new.len()
                && now.duration_since(last.at) < UNDO_MERGE
            {
                last.new = new;
                last.at = now;
                return;
            }
        }
        self.undo.push(Edit {
            target,
            addr,
            old,
            new,
            label: label.to_string(),
            at: now,
        });
        if self.undo.len() > 500 {
            self.undo.remove(0);
        }
    }

    /// Change a parameter: locally, on the instrument, and in the undo list.
    pub fn set(&mut self, t: Target, block: &str, p: &Param, value: i32) {
        let img = self.image_of(t);
        let Some(i) = img.index(block) else { return };
        let start = p.off as usize;
        let old = img.blocks[i].data[start..start + p.len()].to_vec();
        let (addr, bytes) = img.set(i, p, value);
        if old == bytes {
            return;
        }
        self.session.write(addr, &bytes, true);
        if t != Target::System {
            self.dirty = true;
        }
        self.record(t, addr, old, bytes, p.label());
    }

    /// Write raw bytes into a block (names, tone selection).
    pub fn set_bytes(&mut self, t: Target, block: &str, off: u32, bytes: &[u8], label: &str) {
        let img = self.image_of(t);
        let Some(i) = img.index(block) else { return };
        let o = off as usize;
        let old = img.blocks[i].data[o..o + bytes.len()].to_vec();
        if old == bytes {
            return;
        }
        let addr = img.set_bytes(i, off, bytes);
        self.session.write(addr, bytes, true);
        self.dirty = true;
        self.record(t, addr, old, bytes.to_vec(), label);
    }

    pub fn rename_scene(&mut self, name: &str) {
        self.set_bytes(
            Target::Scene,
            "Perf_common",
            0,
            &name_bytes(name),
            "Scene name",
        );
    }

    pub fn rename_tone(&mut self, part: u8, name: &str) {
        self.set_bytes(
            Target::Tone(part),
            "PCMEX_common",
            0,
            &name_bytes(name),
            "Tone name",
        );
    }

    /// Point a part at another tone.
    pub fn choose_tone(&mut self, part: u8, msb: u8, lsb: u8, pc: u8) {
        self.set_bytes(
            Target::Scene,
            &format!("Perf_part_{part}"),
            2,
            &[msb, lsb, pc],
            "Tone",
        );
        if (1..=4).contains(&part) {
            self.tone_reload[usize::from(part - 1)] = Some(Instant::now() + SETTLE);
        }
    }

    fn apply_edit(&mut self, e: &Edit, bytes: &[u8]) {
        let img = self.image_of(e.target);
        img.apply(e.addr, bytes);
        self.session.write(e.addr, bytes, true);
    }

    pub fn undo(&mut self) {
        if let Some(e) = self.undo.pop() {
            let old = e.old.clone();
            self.apply_edit(&e, &old);
            self.tell(format!("Undid {}", e.label));
            self.redo.push(e);
        }
    }

    pub fn redo(&mut self) {
        if let Some(e) = self.redo.pop() {
            let new = e.new.clone();
            self.apply_edit(&e, &new);
            self.tell(format!("Redid {}", e.label));
            self.undo.push(e);
        }
    }

    // ---- scenes ------------------------------------------------------------

    pub fn select_scene(&mut self, msb: u8, lsb: u8, pc: u8) {
        self.session.select_scene(msb, lsb, pc);
        // Reflect it locally; the reload follows from the Setup change.
        let before = self.current_scene();
        self.setup.apply(Area::Setup.base(), &[msb, lsb, pc]);
        if self.current_scene() != before {
            self.current = self.current_scene();
            self.scene_reload = Some(Instant::now() + SETTLE);
        }
    }

    pub fn select_user(&mut self, n: u16) {
        self.select_scene(85, (n / 50) as u8, (n % 50) as u8);
    }

    /// Step through the current list (user or preset) by `dir`.
    pub fn step_scene(&mut self, dir: i32) {
        match self.current_scene() {
            Some(SceneRef::User(n)) => {
                let n = (i32::from(n) - 1 + dir).rem_euclid(600) as u16;
                self.select_user(n);
            }
            Some(SceneRef::Preset { category, program }) => {
                let lsb = category + 63;
                let i = self
                    .preset_names
                    .iter()
                    .position(|s| s.lsb == lsb && s.pc == program - 1);
                if let Some(i) = i {
                    let len = self.preset_names.len() as i32;
                    let s = &self.preset_names[(i as i32 + dir).rem_euclid(len) as usize];
                    let (m, l, p) = (s.msb, s.lsb, s.pc);
                    self.select_scene(m, l, p);
                }
            }
            _ => {}
        }
    }

    /// Store the Temporary Scene (with `name`) into user scene `n` (0..600).
    pub fn save_scene(&mut self, n: u16, name: &str) {
        if name != self.scene_name() {
            self.rename_scene(name);
        }
        self.session.write_scene(n);
        self.user_names[usize::from(n)] = name.trim_end().to_string();
        self.dirty = false;
        self.tell(format!(
            "Saved \"{}\" to user scene {:03} (U{:02}-{:02})",
            name.trim_end(),
            n + 1,
            n / 50 + 1,
            n % 50 + 1
        ));
        // Re-read the setup to see where the unit now is.
        self.scene_reload = None;
        self.stream(TAG_SETUP);
    }

    /// Store part `part`'s tone into user tone `idx` (0..512) and point the part at it.
    pub fn save_tone(&mut self, part: u8, idx: u16, name: &str) {
        self.rename_tone(part, name);
        self.session.write_tone(part, idx);
        if let Some(t) = &mut self.user_tones {
            t[usize::from(idx)] = name.trim_end().to_string();
        }
        let (lsb, pc) = (8 + (idx / 128) as u8, (idx % 128) as u8);
        self.scene.apply(
            Addr(
                self.scene
                    .block(&format!("Perf_part_{part}"))
                    .unwrap()
                    .addr
                    .0
                    + 2,
            ),
            &[71, lsb, pc],
        );
        self.tell(format!(
            "Saved tone \"{}\" to user tone {}",
            name.trim_end(),
            idx + 1
        ));
    }

    pub fn export_scene(&mut self, path: &Path) {
        if !self.scene.is_loaded() {
            return self.warn("The scene hasn't finished loading.");
        }
        match std::fs::write(path, syx::scene_to_syx(&self.scene, self.session.device)) {
            Ok(()) => self.tell(format!("Exported to {}", path.display())),
            Err(e) => self.warn(format!("Couldn't write {}: {e}", path.display())),
        }
    }

    /// Load a `.syx` scene into the Temporary Scene (not stored until saved).
    pub fn import_scene(&mut self, path: &Path) {
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) => return self.warn(format!("Couldn't read {}: {e}", path.display())),
        };
        let Some(img) = syx::scene_from_syx(&bytes) else {
            return self.warn("That file has no AE-30 scene in it.");
        };
        for b in img.blocks.iter().filter(|b| b.is_loaded()) {
            self.scene.apply(b.addr, &b.data);
            self.session.write(b.addr, &b.data, false);
        }
        self.dirty = true;
        self.undo.clear();
        self.redo.clear();
        for t in &mut self.tone_reload {
            *t = Some(Instant::now() + SETTLE);
        }
        self.tell(format!(
            "Loaded \"{}\" — save it to a user scene to keep it",
            img.scene_name().unwrap_or_default()
        ));
    }

    // ---- backup ------------------------------------------------------------

    /// Back up user scenes `want` (0-based) to `path`.
    pub fn start_backup(&mut self, path: std::path::PathBuf, want: Vec<u16>) {
        if want.is_empty() {
            return self.warn("No scenes to back up.");
        }
        self.backup = Some(Backup {
            path,
            want,
            images: vec![None; 600],
            start: None,
            block_mode: false,
            gave_up: Vec::new(),
            started: Instant::now(),
        });
        self.backup_next();
    }

    /// The user scenes that have names other than INIT SCENE.
    pub fn named_scenes(&self) -> Vec<u16> {
        (0..600u16)
            .filter(|&n| {
                let name = &self.user_names[usize::from(n)];
                !name.is_empty() && name != "INIT SCENE"
            })
            .collect()
    }

    pub fn cancel_backup(&mut self) {
        self.backup = None;
        self.session.cancel_reads(|t| t != TAG_BACKUP);
        self.session.abort(TAG_BACKUP);
        self.pending.remove(&TAG_BACKUP);
        self.tell("Backup cancelled");
    }

    /// Read the next scene still missing, block by block (requests sent
    /// back to back); a scene that stays incomplete after a second try is
    /// skipped and reported.
    fn backup_next(&mut self) {
        let Some(b) = &mut self.backup else { return };
        let Some(n) = b.next_missing() else {
            return self.finish_backup();
        };
        if b.start == Some(n) {
            if b.block_mode {
                b.gave_up.push(n);
                b.block_mode = false;
                return self.backup_next();
            }
            b.block_mode = true; // second try: only what's missing
        } else {
            b.start = Some(n);
            b.block_mode = false;
        }
        let img = b.images[usize::from(n)].get_or_insert_with(|| Image::new(Area::UserScene(n)));
        let reads = img.missing_reads();
        self.session.read_blocks(TAG_BACKUP, reads);
        self.pending.insert(TAG_BACKUP, 1);
    }

    /// Called every tick: finish as soon as every wanted scene is in.
    fn check_backup(&mut self) {
        if self
            .backup
            .as_ref()
            .is_some_and(|b| b.next_missing().is_none())
        {
            self.session.abort(TAG_BACKUP);
            self.pending.remove(&TAG_BACKUP);
            self.finish_backup();
        }
    }

    fn finish_backup(&mut self) {
        let Some(b) = self.backup.take() else { return };
        let mut scenes = Vec::new();
        for &n in &b.want {
            if let Some(img) = &b.images[usize::from(n)] {
                if let Some(name) = img.scene_name() {
                    self.user_names[usize::from(n)] = name;
                }
                scenes.push(img.clone());
            }
        }
        let bytes = syx::backup_to_syx(&scenes, self.session.device);
        let secs = b.started.elapsed().as_secs();
        match std::fs::write(&b.path, bytes) {
            Ok(()) if b.gave_up.is_empty() => self.tell(format!(
                "Backed up {} user scenes to {} in {}:{:02}",
                scenes.len(),
                b.path.display(),
                secs / 60,
                secs % 60
            )),
            Ok(()) => self.warn(format!(
                "Backed up to {}, but {} scenes couldn't be read completely (first: {:03})",
                b.path.display(),
                b.gave_up.len(),
                b.gave_up[0] + 1
            )),
            Err(e) => self.warn(format!("Couldn't write {}: {e}", b.path.display())),
        }
    }
}

// ---- preset name cache (presets never change, and listing them is slow) ----

fn cache_path() -> Option<std::path::PathBuf> {
    let dirs = directories::ProjectDirs::from("com.github", "ericdano", "AE30PatchEditor")?;
    Some(dirs.cache_dir().join("preset-scenes.txt"))
}

fn load_preset_cache() -> Vec<SceneName> {
    let Some(text) = cache_path().and_then(|p| std::fs::read_to_string(p).ok()) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|l| {
            let mut it = l.splitn(4, '\t');
            Some(SceneName {
                msb: it.next()?.parse().ok()?,
                lsb: it.next()?.parse().ok()?,
                pc: it.next()?.parse().ok()?,
                name: it.next()?.to_string(),
            })
        })
        .collect()
}

fn save_preset_cache(list: &[SceneName]) {
    let Some(p) = cache_path() else { return };
    let _ = std::fs::create_dir_all(p.parent().unwrap());
    let text: String = list
        .iter()
        .map(|s| format!("{}\t{}\t{}\t{}\n", s.msb, s.lsb, s.pc, s.name))
        .collect();
    let _ = std::fs::write(p, text);
}
