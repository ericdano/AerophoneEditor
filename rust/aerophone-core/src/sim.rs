//! A simulated AE-30 for demo mode: it takes the SysEx the editor would
//! send and answers the way the instrument does, from memory it keeps
//! itself. Nothing goes over MIDI.
//!
//! - Identity Request: an AE-30 Identity Reply.
//! - RQ1: DT1s of every block in the requested range (temporary and stored
//!   scenes, temporary tones, System, Setup).
//! - Scene/tone name lists: 600 user scenes (a few demo scenes, the rest
//!   "INIT SCENE"), preset scenes named after preset tones, user tones.
//! - DT1: placed in memory. A Setup change loads that scene into the
//!   Temporary Scene; a part's tone change loads that tone into its
//!   temporary tone; the store commands copy into user scenes/tones.

use std::collections::HashMap;

use crate::image::{name_bytes, Area, Image, WRITE_CHUNK};
use crate::params::{data, Param};
use crate::session::{cmd, USER_TONES};
use crate::sysex::{self, Addr, DEFAULT_DEVICE, MODEL_ID, ROLAND, RQ1};
use crate::syx;

/// Banks: ZEN-Core preset tones A/B/C (editable), SuperNATURAL, drum kits.
const BANK_A: (u8, u8) = (71, 72);
const BANK_B: (u8, u8) = (71, 73);
const BANK_SN: (u8, u8) = (89, 66);
const USER_SCENE_MSB: u8 = 85;
const PRESET_CATEGORIES: u8 = 12;
const PRESETS_PER_CATEGORY: usize = 10;

/// A demo scene: name, the tones of its parts (by name), and its effects.
struct Demo {
    name: &'static str,
    tones: &'static [&'static str],
    reverb: i32,
    chorus: bool,
    delay: bool,
    ifx: i32,
}

const DEMO_SCENES: &[Demo] = &[
    Demo {
        name: "Demo Alto Sax",
        tones: &["Pop Alto Sax"],
        reverb: 70,
        chorus: false,
        delay: true,
        ifx: 0,
    },
    Demo {
        name: "Demo Big Band",
        tones: &["AE Big Band Sax", "AE BigBrass Band"],
        reverb: 80,
        chorus: false,
        delay: false,
        ifx: 0,
    },
    Demo {
        name: "Demo Muted Tpt",
        tones: &["Mute Trumpet MT"],
        reverb: 60,
        chorus: false,
        delay: true,
        ifx: 0,
    },
    Demo {
        name: "Demo Clarinet",
        tones: &["Classic Clarinet", "AE Strings"],
        reverb: 90,
        chorus: false,
        delay: false,
        ifx: 0,
    },
    Demo {
        name: "Demo Saw Lead",
        tones: &["Saw Lead AP"],
        reverb: 50,
        chorus: true,
        delay: true,
        ifx: 5,
    },
    Demo {
        name: "Demo Jazz Organ",
        tones: &["Jazz Organ"],
        reverb: 40,
        chorus: true,
        delay: false,
        ifx: 0,
    },
    Demo {
        name: "Demo Pan Flute",
        tones: &["Pan Flute MT", "AE Choir"],
        reverb: 100,
        chorus: false,
        delay: true,
        ifx: 0,
    },
    Demo {
        name: "Demo Harmonica",
        tones: &["Blues Harmonica"],
        reverb: 60,
        chorus: false,
        delay: false,
        ifx: 3,
    },
];

struct Preset {
    lsb: u8,
    pc: u8,
    name: String,
    tone: (u8, u8, u8),
}

pub struct Sim {
    pub device: u8,
    images: HashMap<Area, Image>,
    presets: Vec<Preset>,
    user_tones: Vec<String>,
}

impl Default for Sim {
    fn default() -> Self {
        Self::new()
    }
}

impl Sim {
    pub fn new() -> Self {
        let mut sim = Sim {
            device: DEFAULT_DEVICE,
            images: HashMap::new(),
            presets: presets(),
            user_tones: vec![String::new(); usize::from(USER_TONES)],
        };
        // Start on the first demo scene.
        let setup = sim.image(Area::Setup);
        let b = setup.blocks[0].addr;
        setup.apply(b, &[USER_SCENE_MSB, 0, 0]);
        sim.load_temp_scene();
        sim
    }

    /// Put the user scenes from a backup `.syx` into memory (demo content).
    pub fn load_user_scenes(&mut self, bytes: &[u8]) {
        for (area, img) in syx::scenes_from_syx(bytes) {
            if let Area::UserScene(_) = area {
                if img.is_loaded() {
                    self.images.insert(area, img);
                }
            }
        }
        self.load_temp_scene();
    }

    /// The replies to one message the editor sent.
    pub fn handle(&mut self, msg: &[u8]) -> Vec<Vec<u8>> {
        if matches!(msg, [0xF0, 0x7E, _, 0x06, 0x01, 0xF7]) {
            return vec![vec![
                0xF0,
                0x7E,
                self.device,
                0x06,
                0x02,
                ROLAND,
                0x7D,
                0x03,
                0x00,
                0x00,
                0x00,
                0x03,
                0x02,
                0x00,
                0xF7,
            ]];
        }
        if let Some((at, size)) = parse_rq1(msg) {
            return self.read(at, size);
        }
        let Ok(set) = sysex::parse_dt1(msg) else {
            return Vec::new();
        };
        self.write(set.addr, &set.data)
    }

    fn read(&mut self, at: Addr, size: u32) -> Vec<Vec<u8>> {
        let Some(area) = area_of(at) else {
            return Vec::new();
        };
        let end = at.0 + size;
        let device = self.device;
        self.image(area)
            .blocks
            .iter()
            .filter(|b| b.addr.0 < end && b.addr.0 + b.data.len() as u32 > at.0)
            .flat_map(|b| sysex::dt1_packets(device, b.addr, &b.data, WRITE_CHUNK))
            .collect()
    }

    fn write(&mut self, at: Addr, d: &[u8]) -> Vec<Vec<u8>> {
        if at == cmd::SCENE_LIST {
            let (user, bank) = (d.first() == Some(&1), d.get(1).copied().unwrap_or(0));
            return self.scene_list(user, bank);
        }
        if at == cmd::TONE_LIST {
            return self.tone_list();
        }
        if at == cmd::SCENE_WRITE {
            let n = sysex::unpack_nibbles(d.get(1..4).unwrap_or(&[])) as u16;
            if n < 600 {
                let img = self.image(Area::TempScene).retarget(Area::UserScene(n));
                self.images.insert(Area::UserScene(n), img);
            }
            return Vec::new();
        }
        if (cmd::TONE_WRITE.0..cmd::TONE_WRITE.0 + 16).contains(&at.0) {
            let part = ((at.0 - cmd::TONE_WRITE.0) / 4 + 1) as u8;
            let idx = sysex::unpack_nibbles(d.get(1..4).unwrap_or(&[])) as usize;
            let name = self
                .image(Area::TempTone(part))
                .scene_name()
                .unwrap_or_default();
            if let Some(slot) = self.user_tones.get_mut(idx) {
                *slot = name;
            }
            return Vec::new();
        }
        if at == cmd::SYSTEM_WRITE {
            return Vec::new();
        }
        let Some(area) = area_of(at) else {
            return Vec::new();
        };
        match area {
            Area::Setup => {
                let before = self.scene_bytes();
                self.image(Area::Setup).apply(at, d);
                if self.scene_bytes() != before {
                    self.load_temp_scene();
                }
            }
            Area::TempScene => {
                let before: Vec<_> = (1..=4).map(|p| self.part_tone(p)).collect();
                self.image(Area::TempScene).apply(at, d);
                for p in 1..=4u8 {
                    if self.part_tone(p) != before[usize::from(p - 1)] {
                        self.load_temp_tone(p);
                    }
                }
            }
            _ => {
                self.image(area).apply(at, d);
            }
        }
        Vec::new()
    }

    // ---- memory ------------------------------------------------------------

    fn image(&mut self, area: Area) -> &mut Image {
        if !self.images.contains_key(&area) {
            let img = self.fresh(area);
            self.images.insert(area, img);
        }
        self.images.get_mut(&area).unwrap()
    }

    /// What an area holds before anything was written to it.
    fn fresh(&self, area: Area) -> Image {
        match area {
            Area::UserScene(n) => match DEMO_SCENES.get(usize::from(n)) {
                Some(d) => demo_scene(area, d),
                None => scene(
                    area,
                    "INIT SCENE",
                    &[(BANK_A.0, BANK_A.1, 0)],
                    0,
                    false,
                    false,
                    0,
                ),
            },
            Area::PresetScene(n) => match self.presets.get(usize::from(n)) {
                Some(p) => scene(area, &p.name, &[p.tone], 70, false, false, 0),
                None => scene(
                    area,
                    "INIT SCENE",
                    &[(BANK_A.0, BANK_A.1, 0)],
                    0,
                    false,
                    false,
                    0,
                ),
            },
            Area::TempScene => scene(
                area,
                "INIT SCENE",
                &[(BANK_A.0, BANK_A.1, 0)],
                0,
                false,
                false,
                0,
            ),
            _ => defaults(area),
        }
    }

    fn scene_bytes(&mut self) -> (u8, u8, u8) {
        let b = &self.image(Area::Setup).blocks[0];
        (b.data[0], b.data[1], b.data[2])
    }

    fn part_tone(&mut self, part: u8) -> (u8, u8, u8) {
        let img = self.image(Area::TempScene);
        let b = img.block(&format!("Perf_part_{part}")).unwrap();
        (b.data[2], b.data[3], b.data[4])
    }

    /// Copy the scene the Setup block points at into the Temporary Scene,
    /// and its parts' tones into the temporary tones.
    fn load_temp_scene(&mut self) {
        let (msb, lsb, pc) = self.scene_bytes();
        let src = match (msb, lsb) {
            (USER_SCENE_MSB, 0..=11) if pc < 50 => {
                Some(Area::UserScene(u16::from(lsb) * 50 + u16::from(pc)))
            }
            (USER_SCENE_MSB, 64..=75) => self
                .presets
                .iter()
                .position(|p| p.lsb == lsb && p.pc == pc)
                .map(|i| Area::PresetScene(i as u16)),
            _ => None,
        };
        if let Some(src) = src {
            let img = self.image(src).retarget(Area::TempScene);
            self.images.insert(Area::TempScene, img);
        }
        for p in 1..=4 {
            self.load_temp_tone(p);
        }
    }

    /// A part's temporary tone, named after the tone the part plays.
    fn load_temp_tone(&mut self, part: u8) {
        let (msb, lsb, pc) = self.part_tone(part);
        let name = match (msb, lsb) {
            (71, 8..=11) => {
                let i = usize::from(lsb - 8) * 128 + usize::from(pc);
                self.user_tones
                    .get(i)
                    .filter(|s| !s.is_empty())
                    .cloned()
                    .unwrap_or_else(|| format!("User Tone {}", i + 1))
            }
            _ => data()
                .tone_name(msb, lsb, pc)
                .unwrap_or("INIT TONE")
                .to_string(),
        };
        let mut img = defaults(Area::TempTone(part));
        let b = img.blocks[0].addr;
        img.apply(b, &name_bytes(&name));
        self.images.insert(Area::TempTone(part), img);
    }

    // ---- name lists --------------------------------------------------------

    fn scene_list(&mut self, user: bool, bank: u8) -> Vec<Vec<u8>> {
        let mut entries: Vec<(u8, u8, u8, String)> = Vec::new();
        if user {
            for pc in 0..50u8 {
                let n = u16::from(bank) * 50 + u16::from(pc);
                if n >= 600 {
                    break;
                }
                let name = self
                    .image(Area::UserScene(n))
                    .scene_name()
                    .unwrap_or_default();
                entries.push((USER_SCENE_MSB, bank, pc, name));
            }
        } else if bank < PRESET_CATEGORIES {
            for p in self.presets.iter().filter(|p| p.lsb == 64 + bank) {
                entries.push((USER_SCENE_MSB, p.lsb, p.pc, p.name.clone()));
            }
        }
        let entry = |msb: u8, lsb: u8, pc: u8, name: &str| {
            let mut d = vec![u8::from(user), bank, 0, msb, lsb, pc, 0, 0, 0, 0];
            d.extend_from_slice(&name_bytes(name));
            sysex::dt1(self.device, cmd::SCENE_LIST_ENTRY, &d)
        };
        let mut out: Vec<Vec<u8>> = entries
            .iter()
            .map(|(m, l, p, n)| entry(*m, *l, *p, n))
            .collect();
        if out.len() < 50 || !user {
            out.push(entry(0, 0, 0, ""));
        }
        out
    }

    fn tone_list(&self) -> Vec<Vec<u8>> {
        let entry = |msb: u8, lsb: u8, pc: u8, name: &str| {
            let mut d = vec![1, 0, 0, msb, lsb, pc];
            d.extend_from_slice(&name_bytes(name));
            sysex::dt1(self.device, cmd::TONE_LIST_ENTRY, &d)
        };
        let mut out: Vec<Vec<u8>> = self
            .user_tones
            .iter()
            .enumerate()
            .filter(|(_, n)| !n.is_empty())
            .map(|(i, n)| entry(71, 8 + (i / 128) as u8, (i % 128) as u8, n))
            .collect();
        out.push(entry(0, 0, 0, ""));
        out
    }
}

/// Which area an address falls in.
fn area_of(a: Addr) -> Option<Area> {
    if let Some(area) = syx::scene_area_of(a) {
        return Some(area);
    }
    let [hi, mid, ..] = a.to_bytes();
    match (hi, mid) {
        (0x00, 0x10..) => Some(Area::Setup),
        (0x00, _) => Some(Area::System),
        (0x02, 0..=3) => Some(Area::TempTone(mid + 1)),
        _ => None,
    }
}

/// `F0 41 dev 00 00 00 7D 11 aaaa ssss sum F7` → (address, size).
fn parse_rq1(msg: &[u8]) -> Option<(Addr, u32)> {
    if msg.len() != 18 || msg[1] != ROLAND || msg[3..7] != MODEL_ID || msg[7] != RQ1 {
        return None;
    }
    let a = Addr::from_bytes(msg[8..12].try_into().ok()?);
    let n = Addr::from_bytes(msg[12..16].try_into().ok()?).0;
    Some((a, n))
}

/// A plausible starting value: 0 where the range allows it, levels and
/// volumes at 100, tempo 120, bend range 2, upper limits at the top,
/// everything else at its minimum.
fn default_value(p: &Param) -> i32 {
    let name = p.name.to_lowercase();
    if (name.contains("level") || name.contains("volume")) && p.max >= 100 && !name.contains("send")
    {
        100
    } else if name.contains("tempo") {
        12000.clamp(p.min, p.max) // 120.00 BPM
    } else if name.contains("bend range") {
        2.clamp(p.min, p.max)
    } else if name.contains("upper") {
        p.max
    } else {
        0.clamp(p.min, p.max)
    }
}

/// An area with every block filled with default values.
fn defaults(area: Area) -> Image {
    let mut img = Image::new(area);
    for b in &mut img.blocks {
        for p in &b.table.params {
            p.set(&mut b.data, default_value(p));
        }
    }
    let fills: Vec<(Addr, Vec<u8>)> = img
        .blocks
        .iter()
        .map(|b| (b.addr, b.data.clone()))
        .collect();
    for (a, d) in fills {
        img.apply(a, &d);
    }
    img
}

fn set(img: &mut Image, block: &str, param: &str, value: i32) {
    let Some(i) = img.index(block) else { return };
    let Some(p) = img.blocks[i].table.params.iter().find(|p| p.name == param) else {
        return;
    };
    img.set(i, p, value);
}

/// A scene called `name` playing `tones` (parts 1, 2, …) with these effects.
fn scene(
    area: Area,
    name: &str,
    tones: &[(u8, u8, u8)],
    reverb: i32,
    chorus: bool,
    delay: bool,
    ifx: i32,
) -> Image {
    let mut img = defaults(area);
    img.set_bytes(0, 0, &name_bytes(name));
    for (k, &(msb, lsb, pc)) in tones.iter().enumerate().take(4) {
        let part = format!("Perf_part_{}", k + 1);
        set(&mut img, &part, "Part Switch", 1);
        set(
            &mut img,
            &part,
            "Tone Bank Select MSB (CC#0)",
            i32::from(msb),
        );
        set(
            &mut img,
            &part,
            "Tone Bank Select LSB (CC#32)",
            i32::from(lsb),
        );
        set(&mut img, &part, "Tone Program Change (PC)", i32::from(pc));
        set(
            &mut img,
            &part,
            "Part Level (CC#7)",
            if k == 0 { 110 } else { 80 },
        );
        set(&mut img, &part, "Part Reverb Send Level (CC#91)", 40);
        set(
            &mut img,
            &part,
            "Part Chorus Send Level (CC#93)",
            if chorus { 40 } else { 0 },
        );
        set(
            &mut img,
            &part,
            "Part Delay Send Level",
            if delay { 30 } else { 0 },
        );
    }
    if reverb > 0 {
        set(&mut img, "Perf_rev", "Switch", 1);
        set(&mut img, "Perf_rev", "Reverb Type", 1);
        set(&mut img, "Perf_rev", "Reverb Level", reverb);
    }
    if chorus {
        set(&mut img, "Perf_cho", "Switch", 1);
        set(&mut img, "Perf_cho", "Chorus Type", 1);
        set(&mut img, "Perf_cho", "Chorus Level", 90);
    }
    if delay {
        set(&mut img, "Perf_dly", "Switch", 1);
        set(&mut img, "Perf_dly", "Chorus Type", 1);
        set(&mut img, "Perf_dly", "Chorus Level", 70);
    }
    if ifx > 0 {
        set(&mut img, "Perf_ifx", "MFX Switch", 1);
        set(&mut img, "Perf_ifx", "MFX Type", ifx);
    }
    img
}

/// A setlist for demo mode: some of the demo user scenes, as
/// (bank select, Temporary Scene image) pairs.
pub fn demo_setlist() -> Vec<((u8, u8, u8), Image)> {
    [0u8, 2, 4, 1, 6]
        .iter()
        .map(|&i| {
            let img = demo_scene(Area::TempScene, &DEMO_SCENES[usize::from(i)]);
            ((USER_SCENE_MSB, 0, i), img)
        })
        .collect()
}

fn demo_scene(area: Area, d: &Demo) -> Image {
    let tones: Vec<(u8, u8, u8)> = d.tones.iter().filter_map(|n| find_tone(n)).collect();
    scene(area, d.name, &tones, d.reverb, d.chorus, d.delay, d.ifx)
}

/// (MSB, LSB, PC) of a preset tone by name.
fn find_tone(name: &str) -> Option<(u8, u8, u8)> {
    [("A", BANK_A), ("B", BANK_B), ("SN", BANK_SN)]
        .iter()
        .find_map(|(bank, (msb, lsb))| {
            let pc = data().tones.get(*bank)?.iter().position(|t| t == name)?;
            Some((*msb, *lsb, pc as u8))
        })
}

/// Preset scenes: ten per category, each named after a preset tone it plays.
fn presets() -> Vec<Preset> {
    let tones: Vec<((u8, u8, u8), String)> = [("A", BANK_A), ("B", BANK_B)]
        .iter()
        .flat_map(|(bank, (msb, lsb))| {
            data()
                .tones
                .get(*bank)
                .into_iter()
                .flatten()
                .enumerate()
                .filter(|(_, n)| !n.is_empty())
                .map(move |(pc, n)| ((*msb, *lsb, pc as u8), n.clone()))
        })
        .collect();
    let step = (tones.len() / (usize::from(PRESET_CATEGORIES) * PRESETS_PER_CATEGORY)).max(1);
    let mut out = Vec::new();
    for c in 0..PRESET_CATEGORIES {
        for k in 0..PRESETS_PER_CATEGORY {
            let i = (usize::from(c) * PRESETS_PER_CATEGORY + k) * step;
            let Some((tone, name)) = tones.get(i) else {
                break;
            };
            out.push(Preset {
                lsb: 64 + c,
                pc: k as u8,
                name: name.clone(),
                tone: *tone,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{Event, Session};
    use std::time::{Duration, Instant};

    /// Run a session against the simulator until it is idle.
    fn run(sim: &mut Sim, s: &mut Session) -> Vec<Event> {
        let mut events = Vec::new();
        let mut now = Instant::now();
        for _ in 0..100_000 {
            while let Some(msg) = s.poll(now) {
                for r in sim.handle(&msg) {
                    s.receive(&r, now);
                }
            }
            events.extend(s.take_events());
            if s.is_idle() {
                break;
            }
            now += Duration::from_millis(5);
        }
        events
    }

    #[test]
    fn answers_identity_reads_and_lists() {
        let mut sim = Sim::new();
        let mut s = Session::new(0x7F);
        s.identify();
        let ev = run(&mut sim, &mut s);
        assert!(ev.iter().any(|e| matches!(e, Event::Identity(_))));

        s.read_area(1, Area::TempScene);
        let ev = run(&mut sim, &mut s);
        assert!(ev
            .iter()
            .any(|e| matches!(e, Event::ReadDone { complete: true, .. })));
        let mut img = Image::new(Area::TempScene);
        for e in &ev {
            if let Event::Data { addr, data } = e {
                img.apply(*addr, data);
            }
        }
        assert!(img.is_loaded());
        assert_eq!(img.scene_name().as_deref(), Some("Demo Alto Sax"));

        s.list_scenes(true, 0);
        s.list_scenes(false, 0);
        let ev = run(&mut sim, &mut s);
        let lists: Vec<_> = ev
            .iter()
            .filter_map(|e| match e {
                Event::SceneNames { user, scenes, .. } => Some((*user, scenes.len())),
                _ => None,
            })
            .collect();
        assert_eq!(lists, vec![(true, 50), (false, PRESETS_PER_CATEGORY)]);
    }

    #[test]
    fn selecting_and_storing_scenes() {
        let mut sim = Sim::new();
        let mut s = Session::new(DEFAULT_DEVICE);
        s.select_scene(USER_SCENE_MSB, 0, 2);
        run(&mut sim, &mut s);
        assert_eq!(
            sim.image(Area::TempScene).scene_name().as_deref(),
            Some("Demo Muted Tpt")
        );
        assert_eq!(
            sim.image(Area::TempTone(1)).scene_name().as_deref(),
            Some("Mute Trumpet MT")
        );
        // Rename the temporary scene and store it in U12-50.
        s.write(Area::TempScene.base(), &name_bytes("Stored"), false);
        s.write_scene(599);
        run(&mut sim, &mut s);
        assert_eq!(
            sim.image(Area::UserScene(599)).scene_name().as_deref(),
            Some("Stored")
        );
    }

    #[test]
    fn every_demo_tone_exists() {
        for d in DEMO_SCENES {
            for t in d.tones {
                assert!(find_tone(t).is_some(), "{t}");
            }
        }
    }
}
