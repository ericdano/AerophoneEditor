//! Scene files and backups as standard `.syx` files: a sequence of the DT1
//! messages that would set the data, so other SysEx tools can send them too.
//!
//! - A single scene is stored addressed to the Temporary Scene, so sending
//!   the file to the instrument loads it for playing (not stored).
//! - A backup stores each user scene at its own user-scene address.

use crate::image::{Area, Image};
use crate::sysex::{self, Addr};

/// Split a byte stream into SysEx messages (`F0 … F7`), ignoring anything else.
pub fn split(bytes: &[u8]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut cur: Option<Vec<u8>> = None;
    for &b in bytes {
        match b {
            0xF0 => cur = Some(vec![b]),
            0xF7 => {
                if let Some(mut m) = cur.take() {
                    m.push(b);
                    out.push(m);
                }
            }
            _ => {
                if let Some(m) = &mut cur {
                    m.push(b);
                }
            }
        }
    }
    out
}

/// Which scene area an address falls in.
pub fn scene_area_of(a: Addr) -> Option<Area> {
    let [hi, mid, ..] = a.to_bytes();
    let n = (u16::from(hi & 0x0F) << 7) | u16::from(mid);
    match hi {
        0x01 if mid == 0 => Some(Area::TempScene),
        0x40..=0x44 => (n < 600).then_some(Area::UserScene(n)),
        0x50..=0x53 => (n < 480).then_some(Area::PresetScene(n)),
        _ => None,
    }
}

/// One scene as `.syx` bytes, addressed to the Temporary Scene.
pub fn scene_to_syx(scene: &Image, device: u8) -> Vec<u8> {
    scene.retarget(Area::TempScene).to_dt1s(device).concat()
}

/// All the scenes in a `.syx` file, each with the area it was addressed to.
pub fn scenes_from_syx(bytes: &[u8]) -> Vec<(Area, Image)> {
    let mut scenes: Vec<(Area, Image)> = Vec::new();
    for msg in split(bytes) {
        let Ok(set) = sysex::parse_dt1(&msg) else {
            continue;
        };
        let Some(area) = scene_area_of(set.addr) else {
            continue;
        };
        let i = match scenes.iter().position(|(a, _)| *a == area) {
            Some(i) => i,
            None => {
                scenes.push((area, Image::new(area)));
                scenes.len() - 1
            }
        };
        scenes[i].1.apply(set.addr, &set.data);
    }
    scenes
}

/// The first scene in a `.syx` file, as a Temporary Scene image.
pub fn scene_from_syx(bytes: &[u8]) -> Option<Image> {
    let (_, img) = scenes_from_syx(bytes).into_iter().next()?;
    Some(img.retarget(Area::TempScene))
}

/// A backup of several scenes, each at its own address.
pub fn backup_to_syx(scenes: &[Image], device: u8) -> Vec<u8> {
    scenes
        .iter()
        .flat_map(|s| s.to_dt1s(device))
        .flatten()
        .collect()
}
