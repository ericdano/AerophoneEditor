//! A local copy of one area of the instrument's memory (the temporary scene,
//! a user scene, a temporary tone, …), as named blocks of bytes.
//!
//! Incoming DT1 data is placed by address, so the same code handles replies
//! to our reads and edits made on the instrument's own panel.

use crate::params::{data, LayoutEntry, Param, Table};
use crate::sysex::{self, addr, Addr};

/// Largest RQ1 the editor asks for in one go. Every block is smaller, so
/// blocks are always requested whole: the AE-30 ignores requests that start
/// inside a block of a stored (user/preset) scene.
pub const READ_CHUNK: u32 = 256;
/// DT1 packet size when writing.
pub const WRITE_CHUNK: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Area {
    System,
    Setup,
    TempScene,
    /// Temporary tone of part 1..=4.
    TempTone(u8),
    /// User scene 0..600.
    UserScene(u16),
    /// Preset scene 0..480.
    PresetScene(u16),
}

impl Area {
    pub fn base(self) -> Addr {
        match self {
            Area::System => addr(0, 0, 0, 0),
            Area::Setup => addr(0x00, 0x10, 0, 0),
            Area::TempScene => addr(0x01, 0, 0, 0),
            Area::TempTone(p) => addr(0x02, p.saturating_sub(1), 0, 0),
            Area::UserScene(n) => Addr(addr(0x40, 0, 0, 0).0 + (u32::from(n) << 14)),
            Area::PresetScene(n) => Addr(addr(0x50, 0, 0, 0).0 + (u32::from(n) << 14)),
        }
    }

    pub fn layout(self) -> &'static [LayoutEntry] {
        let l = &data().layouts;
        match self {
            Area::System => &l.system,
            Area::Setup => &l.setup,
            Area::TempScene | Area::UserScene(_) | Area::PresetScene(_) => &l.scene,
            Area::TempTone(_) => &l.tone,
        }
    }

    pub fn is_scene(self) -> bool {
        matches!(
            self,
            Area::TempScene | Area::UserScene(_) | Area::PresetScene(_)
        )
    }

    /// Bytes from the base address to the end of the last block.
    pub fn span(self) -> u32 {
        self.layout()
            .iter()
            .map(|e| e.off + data().table(&e.table).size)
            .max()
            .unwrap_or(0)
    }

    /// Bytes of real data in the area (the blocks, without gaps).
    pub fn data_len(self) -> usize {
        self.layout()
            .iter()
            .map(|e| data().table(&e.table).size as usize)
            .sum()
    }
}

#[derive(Debug, Clone)]
pub struct Block {
    pub entry: &'static LayoutEntry,
    pub table: &'static Table,
    pub addr: Addr,
    pub data: Vec<u8>,
    filled: Vec<bool>,
}

impl Block {
    pub fn is_loaded(&self) -> bool {
        self.filled.iter().all(|&f| f)
    }

    /// Bytes not received yet.
    pub fn missing(&self) -> usize {
        self.filled.iter().filter(|&&f| !f).count()
    }

    pub fn name(&self) -> &'static str {
        &self.entry.name
    }

    /// Value of the parameter at `off`, if loaded.
    pub fn get(&self, off: u32) -> Option<i32> {
        let p = self.table.at(off)?;
        self.value(p)
    }

    pub fn value(&self, p: &Param) -> Option<i32> {
        if !self.is_loaded() {
            return None;
        }
        p.get(&self.data)
    }

    pub fn raw(&self, off: u32) -> Option<u8> {
        self.data.get(off as usize).copied()
    }
}

/// One area's blocks.
#[derive(Debug, Clone)]
pub struct Image {
    pub area: Area,
    pub blocks: Vec<Block>,
}

impl Image {
    pub fn new(area: Area) -> Self {
        let base = area.base();
        let blocks = area
            .layout()
            .iter()
            .map(|e| {
                let table = data().table(&e.table);
                Block {
                    entry: e,
                    table,
                    addr: Addr(base.0 + e.off),
                    data: vec![0; table.size as usize],
                    filled: vec![false; table.size as usize],
                }
            })
            .collect();
        Image { area, blocks }
    }

    pub fn index(&self, name: &str) -> Option<usize> {
        self.blocks.iter().position(|b| b.entry.name == name)
    }

    pub fn block(&self, name: &str) -> Option<&Block> {
        self.blocks.iter().find(|b| b.entry.name == name)
    }

    pub fn block_mut(&mut self, name: &str) -> Option<&mut Block> {
        self.blocks.iter_mut().find(|b| b.entry.name == name)
    }

    pub fn is_loaded(&self) -> bool {
        self.blocks.iter().all(Block::is_loaded)
    }

    pub fn loaded_bytes(&self) -> usize {
        self.blocks
            .iter()
            .map(|b| b.filled.iter().filter(|&&f| f).count())
            .sum()
    }

    /// Forget the contents (e.g. when the instrument switches scene).
    pub fn invalidate(&mut self) {
        for b in &mut self.blocks {
            b.filled.iter_mut().for_each(|f| *f = false);
        }
    }

    /// Place DT1 data. Returns the indices of the blocks it touched.
    pub fn apply(&mut self, at: Addr, bytes: &[u8]) -> Vec<usize> {
        let end = at.0 + bytes.len() as u32;
        let mut touched = Vec::new();
        for (i, b) in self.blocks.iter_mut().enumerate() {
            let (lo, hi) = (b.addr.0, b.addr.0 + b.data.len() as u32);
            if end <= lo || at.0 >= hi {
                continue;
            }
            for (k, &v) in bytes.iter().enumerate() {
                let a = at.0 + k as u32;
                if (lo..hi).contains(&a) {
                    let o = (a - lo) as usize;
                    b.data[o] = v;
                    b.filled[o] = true;
                }
            }
            touched.push(i);
        }
        touched
    }

    /// RQ1 requests (address, size) for every block not fully loaded,
    /// split into chunks of at most [`READ_CHUNK`] bytes.
    pub fn missing_reads(&self) -> Vec<(Addr, u32)> {
        let mut out = Vec::new();
        for b in self.blocks.iter().filter(|b| !b.is_loaded()) {
            let size = b.data.len() as u32;
            let mut off = 0;
            while off < size {
                let n = READ_CHUNK.min(size - off);
                out.push((Addr(b.addr.0 + off), n));
                off += n;
            }
        }
        out
    }

    /// Set a parameter in block `index`; returns the DT1 address and bytes.
    pub fn set(&mut self, index: usize, p: &Param, value: i32) -> (Addr, Vec<u8>) {
        let b = &mut self.blocks[index];
        let bytes = p.set(&mut b.data, value);
        (Addr(b.addr.0 + p.off), bytes)
    }

    /// Write raw bytes at `off` in block `index`; returns the DT1 address.
    pub fn set_bytes(&mut self, index: usize, off: u32, bytes: &[u8]) -> Addr {
        let b = &mut self.blocks[index];
        let o = off as usize;
        b.data[o..o + bytes.len()].copy_from_slice(bytes);
        Addr(b.addr.0 + off)
    }

    /// The same contents, addressed as another area of the same kind.
    pub fn retarget(&self, area: Area) -> Image {
        let mut img = Image::new(area);
        for (dst, src) in img.blocks.iter_mut().zip(&self.blocks) {
            dst.data.clone_from(&src.data);
            dst.filled.clone_from(&src.filled);
        }
        img
    }

    /// DT1 messages that write the loaded blocks back.
    pub fn to_dt1s(&self, device: u8) -> Vec<Vec<u8>> {
        self.blocks
            .iter()
            .filter(|b| b.is_loaded())
            .flat_map(|b| sysex::dt1_packets(device, b.addr, &b.data, WRITE_CHUNK))
            .collect()
    }

    /// The scene's name (for scene areas).
    pub fn scene_name(&self) -> Option<String> {
        let b = self.blocks.first().filter(|b| b.is_loaded())?;
        Some(ascii(&b.data[..16]))
    }
}

/// Bytes as printable ASCII, trailing spaces removed.
pub fn ascii(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|&b| {
            if (32..127).contains(&b) {
                b as char
            } else {
                ' '
            }
        })
        .collect::<String>()
        .trim_end()
        .to_string()
}

/// A 16-character name, padded with spaces, printable ASCII only.
pub fn name_bytes(name: &str) -> [u8; 16] {
    let mut out = [b' '; 16];
    for (o, c) in out.iter_mut().zip(name.chars()) {
        *o = if (' '..='~').contains(&c) {
            c as u8
        } else {
            b'?'
        };
    }
    out
}
