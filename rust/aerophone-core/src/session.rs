//! The conversation with the instrument, as a state machine with no I/O:
//! the caller feeds in received messages and the current time, and asks
//! for the next message to transmit. This keeps pacing, read timeouts and
//! the name-list protocol testable without hardware.
//!
//! - Output is paced like Roland's own app: after a message of `n` bytes,
//!   wait `n / 3.125 + 8` ms (the MIDI wire rate plus a margin).
//! - Edits to the same address that haven't gone out yet are merged, so
//!   dragging a slider sends only the latest value.
//! - One job (a read, a name list, an identity request) is in flight at a time.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crate::image::{ascii, Area};
use crate::sysex::{self, addr, Addr, Identity};

/// Undocumented command addresses (on the wire, 7-bit), from aerophoneCompanion.
pub mod cmd {
    use crate::sysex::{addr, Addr};
    /// Commit the System block to flash: data `[0]`.
    pub const SYSTEM_WRITE: Addr = addr(0x0F, 0x00, 0x10, 0x00);
    /// Write the Temporary Scene to a user scene: `[0, n>>8, n>>4, n]` nibbles.
    pub const SCENE_WRITE: Addr = addr(0x0F, 0x00, 0x10, 0x01);
    /// Write part 1's temporary tone to a user tone; part n is `+ 4*(n-1)`.
    pub const TONE_WRITE: Addr = addr(0x0F, 0x00, 0x10, 0x05);
    pub const SCENE_LIST: Addr = addr(0x0F, 0x00, 0x41, 0x00);
    pub const SCENE_LIST_ENTRY: Addr = addr(0x0F, 0x00, 0x41, 0x24);
    pub const TONE_LIST: Addr = addr(0x0F, 0x00, 0x42, 0x00);
    pub const TONE_LIST_ENTRY: Addr = addr(0x0F, 0x00, 0x42, 0x24);
}

pub const USER_TONES: u16 = 512;

/// How long to wait for the first reply to a request.
const FIRST_REPLY: Duration = Duration::from_millis(1500);
/// A streamed reply is finished when nothing more arrives for this long.
/// Stored scenes can pause mid-stream (read from flash), so allow a long gap.
const STREAM_GAP: Duration = Duration::from_millis(1500);
/// Name lists: first entry may take a while, then entries come quickly.
const LIST_FIRST: Duration = Duration::from_secs(5);
const LIST_GAP: Duration = Duration::from_millis(700);
const READ_RETRIES: u8 = 2;

/// A scene in a name list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneName {
    pub msb: u8,
    pub lsb: u8,
    pub pc: u8,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// Data from a DT1, whether we asked for it or not.
    Data {
        addr: Addr,
        data: Vec<u8>,
    },
    Identity(Identity),
    IdentityTimeout,
    /// A read finished. `complete` is false if it timed out short.
    ReadDone {
        tag: u64,
        addr: Addr,
        received: usize,
        complete: bool,
    },
    /// A scene name list finished (`user` banks U01-U12, else presets).
    SceneNames {
        user: bool,
        bank: u8,
        scenes: Vec<SceneName>,
    },
    /// The user tone names, index 0..512 (empty = unused slot).
    UserToneNames(Vec<String>),
}

#[derive(Debug, Clone)]
enum Job {
    Identify,
    /// RQ1 for `size` bytes at `addr`; counts data landing in
    /// `addr..addr+span`; done once `expect` bytes arrived.
    Read {
        tag: u64,
        addr: Addr,
        size: u32,
        span: u32,
        expect: usize,
    },
    /// Several whole-block RQ1s sent back to back (the unit answers them
    /// in order); done once every block's bytes arrived.
    Batch {
        tag: u64,
        blocks: Vec<(Addr, u32)>,
    },
    SceneList {
        user: bool,
        bank: u8,
    },
    ToneList,
}

#[derive(Debug)]
struct Running {
    job: Job,
    sent: Instant,
    last_data: Option<Instant>,
    received: usize,
    retries: u8,
    scenes: Vec<SceneName>,
    tones: Vec<String>,
}

#[derive(Debug)]
struct Outgoing {
    bytes: Vec<u8>,
    /// Edits with the same key replace each other while queued.
    key: Option<Addr>,
}

#[derive(Debug)]
pub struct Session {
    pub device: u8,
    out: VecDeque<Outgoing>,
    jobs: VecDeque<Job>,
    running: Option<Running>,
    busy_until: Option<Instant>,
    events: Vec<Event>,
    /// Total messages sent / received, for activity lights.
    pub sent_count: u64,
    pub received_count: u64,
}

impl Default for Session {
    fn default() -> Self {
        Self::new(sysex::DEFAULT_DEVICE)
    }
}

/// Time the wire needs for `n` bytes plus Roland's margin.
pub fn pace(n: usize) -> Duration {
    Duration::from_micros(n as u64 * 320 + 8000)
}

impl Session {
    pub fn new(device: u8) -> Self {
        Session {
            device,
            out: VecDeque::new(),
            jobs: VecDeque::new(),
            running: None,
            busy_until: None,
            events: Vec::new(),
            sent_count: 0,
            received_count: 0,
        }
    }

    // ---- requests ----------------------------------------------------------

    pub fn identify(&mut self) {
        self.jobs.push_back(Job::Identify);
    }

    /// Read `size` bytes at `addr` (one block chunk; see `Image::missing_reads`).
    pub fn read(&mut self, tag: u64, addr: Addr, size: u32) {
        self.jobs.push_back(Job::Read {
            tag,
            addr,
            size,
            span: size,
            expect: size as usize,
        });
    }

    /// One RQ1 for a whole temporary area (scene, tone), which the unit
    /// answers with a stream of DT1s for every block in it.
    ///
    /// The unit doesn't treat the size as an address range: for stored
    /// scenes, a scene's span makes it stream on through several following
    /// scenes, and its byte count stops short. For the temporary areas the
    /// span works; any overrun lands in the next temporary tone, which is
    /// harmless. Stored scenes are read with [`Session::read_raw`] (see the
    /// editor's backup).
    pub fn read_area(&mut self, tag: u64, area: Area) {
        let n = area.data_len();
        self.jobs.push_back(Job::Read {
            tag,
            addr: area.base(),
            size: area.span(),
            span: area.span(),
            expect: n,
        });
    }

    /// Read these blocks (exact block addresses and sizes) with requests
    /// sent back to back instead of one round trip each. This is how stored
    /// scenes are read: unlike a streamed request it can't run past them.
    pub fn read_blocks(&mut self, tag: u64, blocks: Vec<(Addr, u32)>) {
        if !blocks.is_empty() {
            self.jobs.push_back(Job::Batch { tag, blocks });
        }
    }

    /// A raw RQ1 of `size` (as sent) whose replies are counted inside
    /// `addr..addr+span`; for experiments.
    pub fn read_raw(&mut self, tag: u64, addr: Addr, size: u32, span: u32, expect: usize) {
        self.jobs.push_back(Job::Read {
            tag,
            addr,
            size,
            span,
            expect,
        });
    }

    pub fn list_scenes(&mut self, user: bool, bank: u8) {
        self.jobs.push_back(Job::SceneList { user, bank });
    }

    pub fn list_user_tones(&mut self) {
        self.jobs.push_back(Job::ToneList);
    }

    /// Drop queued jobs (not the one running) whose read tag matches.
    pub fn cancel_reads(&mut self, keep: impl Fn(u64) -> bool) {
        self.jobs.retain(|j| match j {
            Job::Read { tag, .. } | Job::Batch { tag, .. } => keep(*tag),
            _ => true,
        });
    }

    /// Stop waiting for the running read with `tag` (replies still on
    /// their way are handled as unsolicited data).
    pub fn abort(&mut self, tag: u64) {
        let running = match &self.running {
            Some(Running {
                job: Job::Read { tag: t, .. } | Job::Batch { tag: t, .. },
                ..
            }) => *t == tag,
            _ => false,
        };
        if running {
            self.running = None;
            // Drop its requests not sent yet.
            self.out.retain(|o| o.bytes.get(7) != Some(&sysex::RQ1));
        }
    }

    /// Send a DT1. With `merge`, a queued DT1 to the same address is replaced.
    pub fn write(&mut self, at: Addr, data: &[u8], merge: bool) {
        for (i, chunk) in data.chunks(128).enumerate() {
            let a = Addr(at.0 + (i * 128) as u32);
            let bytes = sysex::dt1(self.device, a, chunk);
            if merge {
                if let Some(q) = self.out.iter_mut().find(|q| q.key == Some(a)) {
                    q.bytes = bytes;
                    continue;
                }
            }
            self.out.push_back(Outgoing {
                bytes,
                key: merge.then_some(a),
            });
        }
    }

    /// Switch the instrument to a scene (Setup block).
    pub fn select_scene(&mut self, msb: u8, lsb: u8, pc: u8) {
        self.write(addr(0x00, 0x10, 0x00, 0x00), &[msb, lsb, pc], true);
    }

    /// Store the Temporary Scene into user scene `n` (0..600).
    pub fn write_scene(&mut self, n: u16) {
        self.write(cmd::SCENE_WRITE, &nibbles3(n), false);
    }

    /// Store part `part`'s temporary tone (1..=4) into user tone `idx` (0..512).
    pub fn write_tone(&mut self, part: u8, idx: u16) {
        let at = Addr(cmd::TONE_WRITE.0 + 4 * u32::from(part.saturating_sub(1)));
        self.write(at, &nibbles3(idx), false);
    }

    /// Commit the System block to flash.
    pub fn write_system(&mut self) {
        self.write(cmd::SYSTEM_WRITE, &[0], false);
    }

    // ---- driving -----------------------------------------------------------

    pub fn is_idle(&self) -> bool {
        self.out.is_empty() && self.jobs.is_empty() && self.running.is_none()
    }

    /// Roughly how long the messages queued now take to go out.
    pub fn queued_time(&self) -> Duration {
        self.out.iter().map(|o| pace(o.bytes.len())).sum()
    }

    pub fn pending_jobs(&self) -> usize {
        self.jobs.len() + usize::from(self.running.is_some())
    }

    pub fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }

    /// The next message to transmit now, if any. Call repeatedly until None.
    pub fn poll(&mut self, now: Instant) -> Option<Vec<u8>> {
        self.check_timeouts(now);
        if self.busy_until.is_some_and(|t| now < t) {
            return None;
        }
        let bytes = if let Some(o) = self.out.pop_front() {
            o.bytes
        } else if self.running.is_none() {
            let job = self.jobs.pop_front()?;
            let bytes = self.request_bytes(&job);
            let sent = self.queue_batch_rest(&job, now);
            self.running = Some(Running {
                job,
                sent,
                last_data: None,
                received: 0,
                retries: 0,
                scenes: Vec::new(),
                tones: vec![String::new(); USER_TONES as usize],
            });
            bytes
        } else {
            return None;
        };
        self.busy_until = Some(now + pace(bytes.len()));
        self.sent_count += 1;
        Some(bytes)
    }

    /// When `poll` might next have something to do.
    pub fn next_wake(&self, now: Instant) -> Option<Duration> {
        let mut wake: Option<Instant> = None;
        let mut at = |t: Instant| wake = Some(wake.map_or(t, |w| w.min(t)));
        if !self.out.is_empty() || (self.running.is_none() && !self.jobs.is_empty()) {
            at(self.busy_until.unwrap_or(now));
        }
        if let Some(r) = &self.running {
            at(self.deadline(r));
        }
        wake.map(|w| w.saturating_duration_since(now))
    }

    fn request_bytes(&self, job: &Job) -> Vec<u8> {
        let d = self.device;
        match job {
            Job::Identify => sysex::IDENTITY_REQUEST.to_vec(),
            Job::Read { addr, size, .. } => sysex::rq1(d, *addr, *size),
            Job::Batch { blocks, .. } => sysex::rq1(d, blocks[0].0, blocks[0].1),
            Job::SceneList { user, bank } => sysex::dt1(
                d,
                cmd::SCENE_LIST,
                &[
                    u8::from(*user),
                    *bank,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0x0F,
                    0x0F,
                    0x0F,
                    0x0F,
                ],
            ),
            Job::ToneList => sysex::dt1(
                d,
                cmd::TONE_LIST,
                &[1, 0, 0, 0, 0, 0, 0, 0x0F, 0x0F, 0x0F, 0x0F],
            ),
        }
    }

    /// For a batch, queue its other requests; returns when the last of
    /// them will have gone out (the reply deadline counts from there).
    fn queue_batch_rest(&mut self, job: &Job, now: Instant) -> Instant {
        let Job::Batch { blocks, .. } = job else {
            return now;
        };
        let mut t = now;
        for &(a, n) in &blocks[1..] {
            let bytes = sysex::rq1(self.device, a, n);
            t += pace(bytes.len());
            self.out.push_back(Outgoing { bytes, key: None });
        }
        t
    }

    fn deadline(&self, r: &Running) -> Instant {
        match (&r.job, r.last_data) {
            (Job::SceneList { .. } | Job::ToneList, None) => r.sent + LIST_FIRST,
            (Job::SceneList { .. } | Job::ToneList, Some(t)) => t + LIST_GAP,
            (_, None) => r.sent + FIRST_REPLY,
            (_, Some(t)) => t + STREAM_GAP,
        }
    }

    fn check_timeouts(&mut self, now: Instant) {
        let Some(r) = &self.running else { return };
        if now < self.deadline(r) {
            return;
        }
        let mut r = self.running.take().unwrap();
        match &r.job {
            Job::Identify if r.retries < READ_RETRIES => {
                // The port may not have been ready: ask again.
                r.retries += 1;
                r.sent = now;
                let bytes = self.request_bytes(&r.job);
                self.out.push_front(Outgoing { bytes, key: None });
                self.running = Some(r);
            }
            Job::Identify => self.events.push(Event::IdentityTimeout),
            Job::Read { tag, addr, .. } => {
                if r.received == 0 && r.retries < READ_RETRIES {
                    // Nothing came back: ask again.
                    r.retries += 1;
                    r.sent = now;
                    let bytes = self.request_bytes(&r.job);
                    self.out.push_front(Outgoing { bytes, key: None });
                    self.running = Some(r);
                    return;
                }
                self.events.push(Event::ReadDone {
                    tag: *tag,
                    addr: *addr,
                    received: r.received,
                    complete: false,
                });
            }
            Job::Batch { tag, blocks } => {
                if r.received == 0 && r.retries < READ_RETRIES {
                    r.retries += 1;
                    let first = self.request_bytes(&r.job);
                    let job = r.job.clone();
                    r.sent = self.queue_batch_rest(&job, now);
                    self.out.push_front(Outgoing {
                        bytes: first,
                        key: None,
                    });
                    self.running = Some(r);
                    return;
                }
                self.events.push(Event::ReadDone {
                    tag: *tag,
                    addr: blocks[0].0,
                    received: r.received,
                    complete: false,
                });
            }
            Job::SceneList { user, bank } => self.events.push(Event::SceneNames {
                user: *user,
                bank: *bank,
                scenes: std::mem::take(&mut r.scenes),
            }),
            Job::ToneList => self
                .events
                .push(Event::UserToneNames(std::mem::take(&mut r.tones))),
        }
    }

    /// Hand in a message received from the instrument.
    pub fn receive(&mut self, msg: &[u8], now: Instant) {
        self.received_count += 1;
        if let Some(id) = sysex::parse_identity(msg) {
            if matches!(self.running.as_ref().map(|r| &r.job), Some(Job::Identify)) {
                self.running = None;
            }
            self.events.push(Event::Identity(id));
            return;
        }
        let Ok(set) = sysex::parse_dt1(msg) else {
            return;
        };
        if set.addr == cmd::SCENE_LIST_ENTRY || set.addr == cmd::TONE_LIST_ENTRY {
            self.list_entry(set.addr, &set.data, now);
            return;
        }
        if let Some(r) = &mut self.running {
            if let Job::Batch { tag, blocks } = &r.job {
                let s0 = set.addr.0;
                let e0 = s0 + set.data.len() as u32;
                let got: u32 = blocks
                    .iter()
                    .map(|&(a, n)| e0.min(a.0 + n).saturating_sub(s0.max(a.0)))
                    .sum();
                if got > 0 {
                    r.received += got as usize;
                    r.last_data = Some(now);
                    let expect: usize = blocks.iter().map(|&(_, n)| n as usize).sum();
                    if r.received >= expect {
                        let (tag, addr, received) = (*tag, blocks[0].0, r.received);
                        self.running = None;
                        self.events.push(Event::Data {
                            addr: set.addr,
                            data: set.data,
                        });
                        self.events.push(Event::ReadDone {
                            tag,
                            addr,
                            received,
                            complete: true,
                        });
                        return;
                    }
                }
            }
            if let Job::Read {
                tag,
                addr,
                span,
                expect,
                ..
            } = r.job
            {
                let (lo, hi) = (addr.0, addr.0 + span);
                let s = set.addr.0;
                let e = s + set.data.len() as u32;
                if e > lo && s < hi {
                    r.received += (e.min(hi) - s.max(lo)) as usize;
                    r.last_data = Some(now);
                    if r.received >= expect {
                        let received = r.received;
                        self.running = None;
                        self.events.push(Event::Data {
                            addr: set.addr,
                            data: set.data,
                        });
                        self.events.push(Event::ReadDone {
                            tag,
                            addr,
                            received,
                            complete: true,
                        });
                        return;
                    }
                }
            }
        }
        self.events.push(Event::Data {
            addr: set.addr,
            data: set.data,
        });
    }

    fn list_entry(&mut self, at: Addr, d: &[u8], now: Instant) {
        let Some(r) = &mut self.running else { return };
        r.last_data = Some(now);
        let finished = match (&r.job, at == cmd::SCENE_LIST_ENTRY) {
            // gg bb dd mm ll pp ii ii ii ii name[16]
            (Job::SceneList { user, bank }, true) => {
                if d.len() < 26 || d[0] != u8::from(*user) || d[1] != *bank {
                    return;
                }
                let (msb, lsb, pc) = (d[3], d[4], d[5]);
                if msb == 0 && lsb == 0 && pc == 0 {
                    true
                } else {
                    r.scenes.push(SceneName {
                        msb,
                        lsb,
                        pc,
                        name: ascii(&d[10..26]),
                    });
                    // User banks always hold 50 scenes.
                    *user && r.scenes.len() >= 50
                }
            }
            // gg ee dd mm ll pp name[16]
            (Job::ToneList, false) => {
                if d.len() < 22 || d[0] != 1 {
                    return;
                }
                let (msb, lsb, pc) = (d[3], d[4], d[5]);
                if msb == 0 && lsb == 0 && pc == 0 {
                    true
                } else {
                    if msb == 71 && (8..=11).contains(&lsb) {
                        let i = usize::from(lsb - 8) * 128 + usize::from(pc);
                        r.tones[i] = ascii(&d[6..22]);
                    }
                    false
                }
            }
            _ => return,
        };
        if finished {
            // Reuse the timeout path to emit the result.
            let r = self.running.as_mut().unwrap();
            r.last_data = Some(now - LIST_GAP);
            self.check_timeouts(now);
        }
    }
}

/// A slot number as the four bytes the write commands take: `0, n>>8, n>>4, n` (nibbles).
fn nibbles3(n: u16) -> [u8; 4] {
    [
        0,
        ((n >> 8) & 0x0F) as u8,
        ((n >> 4) & 0x0F) as u8,
        (n & 0x0F) as u8,
    ]
}
