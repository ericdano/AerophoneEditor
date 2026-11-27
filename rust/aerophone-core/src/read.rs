//! Collecting the DT1 replies to one RQ1 into a single buffer.
//!
//! The unit may answer a request with several DT1 packets; each one says
//! where its data goes, so they are placed by address rather than by order.

use crate::map::Block;
use crate::sysex::{self, DataSet};

#[derive(Debug, Clone)]
pub struct BlockRead {
    block: Block,
    device: u8,
    data: Vec<u8>,
    filled: Vec<bool>,
}

impl BlockRead {
    pub fn new(device: u8, block: Block) -> Self {
        let n = block.size as usize;
        BlockRead {
            block,
            device,
            data: vec![0; n],
            filled: vec![false; n],
        }
    }

    /// The RQ1 message to send.
    pub fn request(&self) -> Vec<u8> {
        sysex::rq1(self.device, self.block.addr, self.block.size)
    }

    /// Offer a received message. Returns true if any of its data belonged
    /// to this block. Other messages are ignored.
    pub fn receive(&mut self, msg: &[u8]) -> bool {
        match sysex::parse_dt1(msg) {
            Ok(set) => self.accept(&set),
            Err(_) => false,
        }
    }

    pub fn accept(&mut self, set: &DataSet) -> bool {
        let Some(start) = set.addr.since(self.block.addr) else {
            return false;
        };
        let mut used = false;
        for (i, &b) in set.data.iter().enumerate() {
            let at = start as usize + i;
            if at < self.data.len() {
                self.data[at] = b;
                self.filled[at] = true;
                used = true;
            }
        }
        used
    }

    pub fn received(&self) -> usize {
        self.filled.iter().filter(|&&f| f).count()
    }

    pub fn is_complete(&self) -> bool {
        self.filled.iter().all(|&f| f)
    }

    /// The data, once complete.
    pub fn into_data(self) -> Option<Vec<u8>> {
        self.is_complete().then_some(self.data)
    }
}
