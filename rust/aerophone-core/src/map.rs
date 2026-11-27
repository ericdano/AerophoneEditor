//! Block addresses and sizes from Roland's *AE-30/AE-20 MIDI Implementation*
//! (v2.01). Offsets are written exactly as in the PDF so they can be checked
//! against it by eye.

use crate::sysex::{addr, Addr};

/// A block of parameters: where it starts and how many bytes it holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Block {
    pub addr: Addr,
    pub size: u32,
}

impl Block {
    pub const fn new(addr: Addr, size: u32) -> Self {
        Block { addr, size }
    }
}

// ---- Top level -----------------------------------------------------------

pub const SYSTEM: Addr = addr(0x00, 0x00, 0x00, 0x00);
pub const SETUP: Block = Block::new(addr(0x00, 0x10, 0x00, 0x00), 0x7A);
pub const TEMP_SCENE: Addr = addr(0x01, 0x00, 0x00, 0x00);
pub const USER_SCENE_BASE: Addr = addr(0x40, 0x00, 0x00, 0x00);
pub const PRESET_SCENE_BASE: Addr = addr(0x50, 0x00, 0x00, 0x00);

pub const USER_SCENES: u32 = 600;
pub const PRESET_SCENES: u32 = 480;
pub const PARTS: u8 = 5;
pub const TEMP_TONES: u8 = 4;

/// Base address of user scene `n` (0-based, 0..600).
pub fn user_scene(n: u32) -> Option<Addr> {
    (n < USER_SCENES).then(|| Addr(USER_SCENE_BASE.0 + (n << 14)))
}

/// Base address of preset scene `n` (0-based, 0..480).
pub fn preset_scene(n: u32) -> Option<Addr> {
    (n < PRESET_SCENES).then(|| Addr(PRESET_SCENE_BASE.0 + (n << 14)))
}

/// Base address of the temporary tone for part `part` (1..=4).
pub fn temp_tone(part: u8) -> Option<Addr> {
    (1..=TEMP_TONES)
        .contains(&part)
        .then(|| addr(0x02, part - 1, 0x00, 0x00))
}

// ---- Within a scene (relative to the scene's base) ------------------------

pub const SCENE_COMMON: Block = Block::new(addr(0, 0x00, 0x00, 0x00), 0x5A);
pub const SCENE_PART_SIZE: u32 = 0x65;
pub const SCENE_EQ_SIZE: u32 = 0x09;

/// Scene Part `part` (1..=5), relative to the scene base.
pub fn scene_part(part: u8) -> Option<Block> {
    (1..=PARTS)
        .contains(&part)
        .then(|| Block::new(addr(0, 0x00, 0x30 + part - 1, 0x00), SCENE_PART_SIZE))
}

/// Scene EQ for part `part` (1..=5), relative to the scene base.
pub fn scene_eq(part: u8) -> Option<Block> {
    (1..=PARTS)
        .contains(&part)
        .then(|| Block::new(addr(0, 0x00, 0x40 + part - 1, 0x00), SCENE_EQ_SIZE))
}

/// A block inside the scene at `scene`.
pub fn in_scene(scene: Addr, block: Block) -> Block {
    Block::new(scene.offset(block.addr), block.size)
}
