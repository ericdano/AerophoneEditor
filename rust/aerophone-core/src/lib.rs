//! Roland Aerophone Pro AE-30 (and AE-20) SysEx protocol: message framing,
//! the full parameter map, local images of scenes and tones, the paced
//! request/reply session, and `.syx` scene files. No MIDI I/O here, so
//! everything can be unit-tested without the instrument.

pub mod image;
pub mod map;
pub mod params;
pub mod read;
pub mod scene;
pub mod session;
pub mod sim;
pub mod sysex;
pub mod syx;

pub use image::{Area, Image};
pub use params::{data, Param};
pub use session::{Event, Session};
