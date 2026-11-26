//! Decoding the Setup block and the parts of a Scene that the first
//! milestones need. Offsets are from the MIDI Implementation PDF.

use crate::sysex::unpack_nibbles;

fn name(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|&b| {
            if (32..127).contains(&b) {
                b as char
            } else {
                '?'
            }
        })
        .collect::<String>()
        .trim_end()
        .to_string()
}

/// A value stored with 64 meaning zero.
fn centered(b: u8) -> i8 {
    b as i8 - 64
}

/// Which scene a Bank Select MSB/LSB + Program Change points at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SceneRef {
    /// User scene 1..=600 (bank U01..U12, 50 per bank).
    User(u16),
    /// Preset category P01..P12, program 1-based.
    Preset {
        category: u8,
        program: u8,
    },
    Unknown {
        msb: u8,
        lsb: u8,
        pc: u8,
    },
}

impl SceneRef {
    pub fn from_bank(msb: u8, lsb: u8, pc: u8) -> Self {
        match (msb, lsb) {
            (85, 0..=11) if pc < 50 => SceneRef::User(lsb as u16 * 50 + pc as u16 + 1),
            (85, 64..=75) => SceneRef::Preset {
                category: lsb - 63,
                program: pc + 1,
            },
            _ => SceneRef::Unknown { msb, lsb, pc },
        }
    }
}

impl std::fmt::Display for SceneRef {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match *self {
            SceneRef::User(n) => {
                let (bank, slot) = ((n - 1) / 50 + 1, (n - 1) % 50 + 1);
                write!(f, "User {n:03} (U{bank:02}-{slot:02})")
            }
            SceneRef::Preset { category, program } => {
                write!(f, "Preset P{category:02}-{program:02}")
            }
            SceneRef::Unknown { msb, lsb, pc } => {
                write!(f, "MSB {msb} LSB {lsb} PC {pc}")
            }
        }
    }
}

/// The Setup block (`00 10 00 00`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Setup {
    pub scene_msb: u8,
    pub scene_lsb: u8,
    pub scene_pc: u8,
    pub favorite: u8,
    pub battery: u8,
    pub transmit_edit_data: bool,
    pub drone: bool,
    pub harmony: bool,
}

impl Setup {
    pub fn decode(b: &[u8]) -> Option<Self> {
        (b.len() >= 0x13).then(|| Setup {
            scene_msb: b[0x00],
            scene_lsb: b[0x01],
            scene_pc: b[0x02],
            favorite: b[0x03],
            battery: b[0x06],
            transmit_edit_data: b[0x07] != 0,
            drone: b[0x11] != 0,
            harmony: b[0x12] != 0,
        })
    }

    pub fn scene(&self) -> SceneRef {
        SceneRef::from_bank(self.scene_msb, self.scene_lsb, self.scene_pc)
    }
}

/// Scene Common (size `5A`).
#[derive(Debug, Clone, PartialEq)]
pub struct SceneCommon {
    pub name: String,
    /// Beats per minute.
    pub tempo: f32,
    pub transpose: i8,
    pub octave: i8,
    pub volume: u8,
}

impl SceneCommon {
    pub fn decode(b: &[u8]) -> Option<Self> {
        (b.len() >= 0x19).then(|| SceneCommon {
            name: name(&b[0x00..0x10]),
            tempo: unpack_nibbles(&b[0x10..0x14]) as f32 / 100.0,
            transpose: centered(b[0x14]),
            octave: centered(b[0x15]),
            volume: b[0x18],
        })
    }
}

/// Which bank a part's tone comes from.
pub fn tone_bank_name(msb: u8, lsb: u8) -> String {
    match (msb, lsb) {
        (71, 8..=11) => format!("User Tone {}", lsb - 7),
        (71, 72) => "ZEN-Core PR-A".into(),
        (71, 73) => "ZEN-Core PR-B".into(),
        (71, 74) => "ZEN-Core PR-C".into(),
        (89, 66) => "SuperNATURAL Acoustic".into(),
        (70, 66) => "Drum Kit".into(),
        _ => format!("Bank {msb}/{lsb}"),
    }
}

/// Scene Part (size `65`): one layer of a scene.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScenePart {
    pub drone: bool,
    pub on: bool,
    pub tone_msb: u8,
    pub tone_lsb: u8,
    pub tone_pc: u8,
    pub level: u8,
    /// -64..=63, 0 = centre.
    pub pan: i8,
    pub reverb_send: u8,
    pub chorus_send: u8,
    pub delay_send: u8,
    pub octave: i8,
}

impl ScenePart {
    pub fn decode(b: &[u8]) -> Option<Self> {
        (b.len() >= 0x19).then(|| ScenePart {
            drone: b[0x00] == 1,
            on: b[0x01] != 0,
            tone_msb: b[0x02],
            tone_lsb: b[0x03],
            tone_pc: b[0x04],
            level: b[0x05],
            pan: centered(b[0x06]),
            reverb_send: b[0x07],
            chorus_send: b[0x08],
            delay_send: b[0x09],
            octave: centered(b[0x18]),
        })
    }

    pub fn tone_bank(&self) -> String {
        tone_bank_name(self.tone_msb, self.tone_lsb)
    }
}
