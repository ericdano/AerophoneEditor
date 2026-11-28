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

use aerophone_core::image::{name_bytes, Area, Image};
use aerophone_core::params::{data, Param};
use aerophone_core::scene::SceneRef;
use aerophone_core::session::{Event, SceneName, Session};
use aerophone_core::sysex::{Addr, Identity, Model};
use aerophone_core::syx;

use crate::midi::{self, Link, Waker};

mod playlist;
pub use playlist::{Entry, Playlist};

const TAG_SETUP: u64 = 1;
const TAG_SCENE: u64 = 2;
const TAG_SYSTEM: u64 = 3;
const TAG_TONE: u64 = 10; // + part
const TAG_BACKUP: u64 = 100;
const TAG_RESTORE: u64 = 200;

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

/// Writing scenes back to user slots, each checked by reading it back.
///
/// Per scene: its blocks go to the Temporary Scene (DT1), the scene-write
/// command stores it in the target slot, then the slot is read back block
/// by block and compared byte for byte with what was sent.
#[derive(Debug)]
pub struct Restore {
    /// (target user scene 0..600, scene data), in order.
    pub items: Vec<(u16, Image)>,
    pub next: usize,
    verify: Option<Image>,
    retried: bool,
    /// The read-back differed and the differences were written directly
    /// into the stored scene; this read-back is the second check.
    patched: bool,
    /// Bytes the store command changed and the patch put right.
    pub patched_bytes: usize,
    /// Values invalid in the file that the AE-30 replaced with valid ones.
    pub corrected: usize,
    /// Target slots whose read-back didn't match, with the first differing block.
    pub failed: Vec<(u16, String)>,
    pub started: Instant,
    /// The scene the AE-30 was on before, to return to afterwards.
    return_to: Option<(u8, u8, u8)>,
}

impl Restore {
    /// (scenes finished, scenes to write)
    pub fn progress(&self) -> (usize, usize) {
        (self.next, self.items.len())
    }

    pub fn eta(&self) -> Option<Duration> {
        let (done, total) = self.progress();
        (done >= 2).then(|| {
            self.started
                .elapsed()
                .mul_f64((total - done) as f64 / done as f64)
        })
    }
}

/// Where each scene in a `.syx` file goes: backed-up user scenes return to
/// their own slots, unless `start` (0-based) is given, which places the
/// scenes one after another from that slot. Single scene files (addressed to
/// the Temporary Scene) need `start`.
pub fn restore_plan(
    file: Vec<(Area, Image)>,
    start: Option<u16>,
) -> Result<Vec<(u16, Image)>, String> {
    let mut out = Vec::new();
    for (i, (area, img)) in file.into_iter().enumerate() {
        if !img.is_loaded() {
            return Err(format!("scene {} in the file is incomplete", i + 1));
        }
        let slot = match (start, area) {
            (Some(s), _) => s + i as u16,
            (None, Area::UserScene(n)) => n,
            (None, _) => {
                return Err("this file isn't a user-scene backup: choose a slot to start at".into())
            }
        };
        if slot >= 600 {
            return Err(format!(
                "not enough slots: scene {} would go past U12-50",
                i + 1
            ));
        }
        out.push((slot, img));
    }
    Ok(out)
}

/// Whether a byte of a written block is one the AE-30 may rewrite as it
/// takes the data, without changing the sound, so a read-back difference
/// there isn't an error:
/// - values outside their documented range, which it replaces with valid ones;
/// - parameters of an effect whose type is Thru/Off (type 0), which it
///   resets to its own defaults (seen in factory INIT scenes).
fn invalid_in_file(b: &aerophone_core::image::Block, off: usize) -> bool {
    let Some(p) = b
        .table
        .params
        .iter()
        .filter(|p| p.fx.is_none())
        .find(|p| (p.off as usize..p.off as usize + p.len()).contains(&off))
    else {
        return false;
    };
    let type_off = match b.entry.table.as_str() {
        "Perf_mfx" | "Perf_ifx" | "PCMEX_mfx" => Some(0),
        "Perf_cho" | "Perf_dly" | "Perf_rev" => Some(1),
        _ => None,
    };
    if p.generic && type_off.is_some_and(|t| b.data.get(t) == Some(&0)) {
        return true;
    }
    p.raw(&b.data)
        .is_some_and(|raw| raw < p.min + p.vofs || raw > p.max.max(p.min) + p.vofs)
}

/// Bytes where a read-back differs from what was written, as
/// (block index, offset), leaving out values that were invalid in the file.
pub fn differences(written: &Image, read: &Image) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    for (k, (w, r)) in written.blocks.iter().zip(&read.blocks).enumerate() {
        if !w.is_loaded() {
            continue;
        }
        for (i, (a, b)) in w.data.iter().zip(&r.data).enumerate() {
            if (!r.is_loaded() || a != b) && !invalid_in_file(w, i) {
                out.push((k, i));
            }
        }
    }
    out
}

/// How many bytes differ only because the file held invalid values.
pub fn corrected_by_unit(written: &Image, read: &Image) -> usize {
    written
        .blocks
        .iter()
        .zip(&read.blocks)
        .filter(|(w, r)| w.is_loaded() && r.is_loaded())
        .map(|(w, r)| {
            w.data
                .iter()
                .zip(&r.data)
                .enumerate()
                .filter(|&(i, (a, b))| a != b && invalid_in_file(w, i))
                .count()
        })
        .sum()
}

/// A readable description of the first real difference, if any.
pub fn first_difference(written: &Image, read: &Image) -> Option<String> {
    let (k, i) = *differences(written, read).first()?;
    let (w, r) = (&written.blocks[k], &read.blocks[k]);
    Some(if !r.is_loaded() {
        format!("{} not read back", w.name())
    } else {
        format!(
            "{} byte {i:#04x}: wrote {:02X}, read {:02X}",
            w.name(),
            w.data[i],
            r.data[i]
        )
    })
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
    pub restore: Option<Restore>,
    /// Breath controller value and when it last moved (from the unit's MIDI out).
    pub breath: (u8, Instant),
    pub note: Option<u8>,
    /// Pitch bend from the unit (-8192..8191) and when it last moved.
    pub bend: (i16, Instant),
    pub last_rx: Option<Instant>,
    /// The Live screen's playlist.
    pub playlist: Playlist,
    /// While in demo mode: the real setlists and cached preset names.
    saved_playlist: Option<Playlist>,
    /// Port checks in a row that didn't find the link's ports.
    port_misses: u8,
    saved_presets: Option<Vec<SceneName>>,
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
            restore: None,
            breath: (0, now),
            note: None,
            bend: (0, now),
            last_rx: None,
            playlist: Playlist::load(),
            saved_playlist: None,
            port_misses: 0,
            saved_presets: None,
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
                // Developer aid: `AE30_PACE=200` doubles the gap between
                // messages (for testing slow links).
                if let Some(p) = std::env::var("AE30_PACE").ok().and_then(|v| v.parse().ok()) {
                    self.session.gap_percent = p;
                }
                log(format_args!(
                    "connect {} (pace {}%)",
                    link.in_label, self.session.gap_percent
                ));
                self.link = Some(link);
                self.conn = Conn::Connecting;
                self.session.identify();
            }
            Err(e) => self.conn = Conn::Failed(e),
        }
    }

    /// Demo mode: connect to a simulated AE-30. Everything works on its
    /// memory and nothing is sent over MIDI. Setlists live in memory only.
    pub fn connect_demo(&mut self, waker: Option<Waker>) {
        self.disconnect();
        self.link = Some(Link::demo());
        // A ready-made setlist (never saved: it has no folder).
        let mut demo = Playlist::default();
        demo.lists[0].name = "Demo Gig".into();
        demo.lists[0].entries = aerophone_core::sim::demo_setlist()
            .into_iter()
            .map(|(source, scene)| playlist::Entry {
                scene,
                source: Some(source),
                slot: None,
                changed: false,
                stored: false,
            })
            .collect();
        self.saved_playlist = Some(std::mem::replace(&mut self.playlist, demo));
        self.saved_presets = Some(std::mem::take(&mut self.preset_names));
        self.conn = Conn::Connecting;
        self.session.identify();
        if let Some(w) = waker {
            w();
        }
    }

    pub fn is_demo(&self) -> bool {
        self.link.as_ref().is_some_and(Link::is_demo)
    }

    pub fn disconnect(&mut self) {
        self.port_misses = 0;
        if let Some(p) = self.saved_playlist.take() {
            self.playlist = p;
        }
        if let Some(p) = self.saved_presets.take() {
            // Leaving demo mode: forget the simulated unit's names.
            self.preset_names = p;
            self.user_names.iter_mut().for_each(String::clear);
            self.user_tones = None;
        }
        self.link = None;
        self.session = Session::default();
        self.pending.clear();
        self.conn = Conn::Offline;
        self.backup = None;
        self.restore = None;
    }

    pub fn is_online(&self) -> bool {
        matches!(self.conn, Conn::Online(_))
    }

    /// Drop the link when its MIDI ports are gone (USB unplugged, Bluetooth
    /// disconnected). Nothing else notices while the unit is idle. The
    /// Failed state makes the app look for the AE-30 again.
    pub fn check_ports(&mut self) {
        let Some(link) = self.link.as_ref().filter(|l| !l.is_demo()) else {
            return;
        };
        let Ok((ins, outs)) = midi::list_ports() else {
            return;
        };
        if ins.contains(&link.in_label) && outs.contains(&link.out_label) {
            self.port_misses = 0;
            return;
        }
        // Missing once can be a glitch (Bluetooth MIDI); letting
        // go of the port then made the system close the connection for real.
        self.port_misses += 1;
        log(format_args!(
            "port \"{}\" missing ({}x): in {ins:?}",
            link.in_label, self.port_misses
        ));
        if self.port_misses < 2 {
            return;
        }
        let e = format!("no MIDI port \"{}\" any more (disconnected)", link.in_label);
        self.disconnect();
        self.conn = Conn::Failed(e);
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

    /// The connected model's name for messages ("AE-30" until known).
    pub fn device_name(&self) -> &'static str {
        match self.model() {
            Some(Model::Ae20) => "AE-20",
            _ => "AE-30",
        }
    }

    /// Settings for the AE-30's Thumb Pad and Motion controller, which the
    /// AE-20 doesn't have: hidden while an AE-20 is connected. (It still
    /// keeps them in memory, so reads and backups are unchanged.)
    pub fn hides(&self, p: &Param) -> bool {
        self.model() == Some(Model::Ae20) && ae30_only(p.label())
    }

    /// Controllers-tab entries the connected model doesn't have.
    pub fn hides_controller(&self, key: &str) -> bool {
        self.model() == Some(Model::Ae20) && matches!(key, "pad_1" | "motion_1")
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
        if let Some(d) = self.playlist_tick(now) {
            wake = Some(wake.map_or(d, |w| w.min(d)));
        }
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
        match &ev {
            Event::Data { .. } => {}
            Event::SceneNames { user, bank, scenes } => log(format_args!(
                "names {} bank {bank}: {}",
                if *user { "user" } else { "preset" },
                scenes.len()
            )),
            Event::UserToneNames(n) => log(format_args!("user tone names: {}", n.len())),
            e => log(format_args!("{e:?}")),
        }
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
                // Let go of the port, so the app keeps looking and tries
                // again (a Bluetooth instrument may need a moment after
                // pairing). Holding on to it stopped all retries.
                self.disconnect();
                self.conn = Conn::Failed(
                    "The Aerophone didn't answer. Is it on, with its USB driver mode set to Generic?"
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
                    TAG_RESTORE => self.restore_check(),
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
                    if bank == 11 && !self.is_demo() {
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
        // While restoring, the Temporary Scene is a staging area: don't
        // reload it (that would interleave reads with the writes).
        let restoring = self.restore.is_some();
        if let Some(r) = &mut self.restore {
            if let Some(v) = &mut r.verify {
                v.apply(addr, bytes);
            }
        }
        let before = self.current_scene();
        self.setup.apply(addr, bytes);
        if self.setup.is_loaded() && !restoring {
            let after = self.current_scene();
            if before.is_some() && after != before {
                // The scene was changed on the instrument (or by us).
                self.current = after;
                self.scene_reload = Some(now + SETTLE);
            }
        }
        let tone_before: Vec<_> = (1..=4).map(|p| self.part_tone(p)).collect();
        if !self.scene.apply(addr, bytes).is_empty() && self.scene.is_loaded() && !restoring {
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
        // Over Bluetooth, ask block by block: a streamed area comes back as
        // one fast burst, which can make an AE-20 drop its Bluetooth link.
        // Paced requests also pace the replies.
        let gentle = self
            .link
            .as_ref()
            .is_some_and(|l| l.in_label.contains("Bluetooth"));
        let img = self.image_mut(tag);
        img.invalidate();
        let area = img.area;
        if gentle {
            let blocks = img.missing_reads();
            self.session.read_blocks(tag, blocks);
        } else {
            self.session.read_area(tag, area);
        }
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
                self.playlist_loaded();
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
        // Keep edits made to a playlist entry before they're replaced.
        self.playlist_capture(false);
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
        format!(
            "{} #{}",
            aerophone_core::scene::tone_bank_name(msb, lsb),
            pc + 1
        )
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
            return self.warn("That file has no Aerophone scene in it.");
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

    // ---- restore -----------------------------------------------------------

    /// Write `items` (target user scene 0..600, scene) to the AE-30, checking
    /// each by reading it back. Uses the Temporary Scene as a staging area,
    /// so unsaved edits there are lost.
    pub fn start_restore(&mut self, items: Vec<(u16, Image)>) {
        if items.is_empty() {
            return self.warn("No scenes to restore.");
        }
        if self.backup.is_some() {
            return self.warn("Wait for the backup to finish first.");
        }
        let return_to = self
            .setup
            .blocks
            .first()
            .filter(|b| b.is_loaded())
            .map(|b| (b.data[0], b.data[1], b.data[2]));
        self.scene_reload = None;
        self.tone_reload = [None; 4];
        self.restore = Some(Restore {
            items,
            next: 0,
            verify: None,
            retried: false,
            patched: false,
            patched_bytes: 0,
            corrected: 0,
            failed: Vec::new(),
            started: Instant::now(),
            return_to,
        });
        self.restore_next();
    }

    pub fn cancel_restore(&mut self) {
        if let Some(r) = &self.restore {
            let done = r.next;
            self.session.cancel_reads(|t| t != TAG_RESTORE);
            self.session.abort(TAG_RESTORE);
            self.pending.remove(&TAG_RESTORE);
            self.finish_restore();
            self.warn(format!("Restore stopped after {done} scenes"));
        }
    }

    fn restore_next(&mut self) {
        let Some(r) = &mut self.restore else { return };
        let Some((slot, img)) = r.items.get(r.next) else {
            return self.finish_restore();
        };
        let slot = *slot;
        // 1. Stage the scene in the Temporary Scene.
        let staged = img.retarget(Area::TempScene);
        for b in staged.blocks.iter().filter(|b| b.is_loaded()) {
            self.session.write(b.addr, &b.data, false);
        }
        // 2. Store it.
        self.session.write_scene(slot);
        // 3. Read the slot back (sent after the writes; the unit answers in order).
        let verify = Image::new(Area::UserScene(slot));
        let reads = verify.missing_reads();
        r.verify = Some(verify);
        r.retried = false;
        r.patched = false;
        self.session.read_blocks(TAG_RESTORE, reads);
        self.pending.insert(TAG_RESTORE, 1);
    }

    fn restore_check(&mut self) {
        let Some(r) = &mut self.restore else { return };
        let (slot, written) = &r.items[r.next];
        let slot = *slot;
        let v = r.verify.as_ref().unwrap();
        if !v.is_loaded() && !r.retried {
            r.retried = true;
            let reads = v.missing_reads();
            self.session.read_blocks(TAG_RESTORE, reads);
            self.pending.insert(TAG_RESTORE, 1);
            return;
        }
        if let Some(diff) = first_difference(written, v) {
            if !r.patched && v.is_loaded() {
                // The AE-30's store command changes a few values (Mono/Poly
                // "Tone" becomes "Poly" in parts 3-5). Write the differing
                // bytes straight into the stored scene (Roland's map allows
                // DT1 to user scenes; tested to survive power-off), then
                // check again.
                let patches: Vec<(Addr, u8)> = differences(written, v)
                    .into_iter()
                    .map(|(k, i)| {
                        (
                            Addr(v.blocks[k].addr.0 + i as u32),
                            written.blocks[k].data[i],
                        )
                    })
                    .collect();
                r.patched = true;
                r.patched_bytes += patches.len();
                for (a, b) in patches {
                    self.session.write(a, &[b], false);
                }
                let again = Image::new(Area::UserScene(slot));
                let reads = again.missing_reads();
                r.verify = Some(again);
                self.session.read_blocks(TAG_RESTORE, reads);
                self.pending.insert(TAG_RESTORE, 1);
                return;
            }
            r.failed.push((slot, diff));
        }
        r.corrected += corrected_by_unit(written, r.verify.as_ref().unwrap());
        if let Some(name) = written.scene_name() {
            self.user_names[usize::from(slot)] = name;
        }
        r.next += 1;
        r.verify = None;
        self.restore_next();
    }

    fn finish_restore(&mut self) {
        let Some(r) = self.restore.take() else { return };
        let secs = r.started.elapsed().as_secs();
        if r.failed.is_empty() {
            let mut notes = Vec::new();
            if r.patched_bytes > 0 {
                notes.push(format!(
                    "{} values the {} changed on saving were put back",
                    r.patched_bytes,
                    self.device_name()
                ));
            }
            if r.corrected > 0 {
                notes.push(format!(
                    "{} settings of unused effects or out-of-range values were set by the {} itself",
                    r.corrected,
                    self.device_name()
                ));
            }
            let fixed = if notes.is_empty() {
                String::new()
            } else {
                format!(" ({})", notes.join("; "))
            };
            self.tell(format!(
                "Restored and checked {} scenes in {}:{:02}{fixed}",
                r.next,
                secs / 60,
                secs % 60
            ));
        } else {
            let (slot, why) = &r.failed[0];
            self.warn(format!(
                "Restored {} scenes; {} didn't read back the same (first: {:03}, {why})",
                r.next,
                r.failed.len(),
                slot + 1
            ));
        }
        // Back to the scene the AE-30 was on (this also reloads the editor).
        match r.return_to {
            Some((m, l, p)) => {
                self.session.select_scene(m, l, p);
                self.scene_reload = Some(Instant::now() + SETTLE);
            }
            None => self.load_scene(),
        }
        self.dirty = false;
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
    let dirs = crate::paths::dirs()?;
    Some(dirs.cache_dir().join("preset-scenes.txt"))
}

/// Labels of settings for the Thumb Pad and Motion controller (AE-30 only).
fn ae30_only(label: &str) -> bool {
    let l = label.to_lowercase();
    l.contains("motion") || l.contains("thumb") || l.ends_with("select pad")
}

/// Developer aid: `AE30_LOG=1` prints connection steps with the seconds
/// since start to stderr.
pub fn log(args: std::fmt::Arguments) {
    use std::sync::OnceLock;
    static ON: OnceLock<Option<Instant>> = OnceLock::new();
    if let Some(t0) = ON.get_or_init(|| std::env::var_os("AE30_LOG").map(|_| Instant::now())) {
        eprintln!("[ae30 {:7.2}] {args}", t0.elapsed().as_secs_f32());
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn filled(area: Area, v: u8) -> Image {
        let mut img = Image::new(area);
        for (a, n) in img.missing_reads() {
            img.apply(a, &vec![v; n as usize]);
        }
        img
    }

    #[test]
    fn restore_plan_places_scenes() {
        let file = vec![
            (Area::UserScene(7), filled(Area::UserScene(7), 64)),
            (Area::UserScene(9), filled(Area::UserScene(9), 64)),
        ];
        let own: Vec<u16> = restore_plan(file.clone(), None)
            .unwrap()
            .iter()
            .map(|x| x.0)
            .collect();
        assert_eq!(own, [7, 9]);
        let moved: Vec<u16> = restore_plan(file, Some(100))
            .unwrap()
            .iter()
            .map(|x| x.0)
            .collect();
        assert_eq!(moved, [100, 101]);
        let single = vec![(Area::TempScene, filled(Area::TempScene, 64))];
        assert!(restore_plan(single.clone(), None).is_err());
        assert!(restore_plan(single, Some(599)).is_ok());
        let two = vec![
            (Area::TempScene, filled(Area::TempScene, 64)),
            (Area::TempScene, filled(Area::TempScene, 64)),
        ];
        assert!(restore_plan(two, Some(599)).is_err(), "runs past U12-50");
    }

    #[test]
    fn checking_ignores_what_the_unit_sets_itself() {
        let mut written = filled(Area::UserScene(0), 64);
        let i = written.index("Perf_mfx_1").unwrap();
        written.blocks[i].data[0] = 0; // MFX type Thru
        let mut read = written.clone();
        // Unused effect parameters rewritten by the unit: not a difference.
        read.blocks[i].data[0x16] = 0x0F;
        assert!(differences(&written, &read).is_empty());
        assert_eq!(corrected_by_unit(&written, &read), 1);
        // A real change elsewhere is.
        let p = read.index("Perf_part_3").unwrap();
        read.blocks[p].data[0x24] = 1;
        written.blocks[p].data[0x24] = 2;
        assert_eq!(differences(&written, &read), [(p, 0x24)]);
        assert!(first_difference(&written, &read)
            .unwrap()
            .contains("Perf_part_3"));
    }
}
