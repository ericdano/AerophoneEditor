//! MIDI ports and a blocking request/reply link to the AE-30, via midir
//! (CoreMIDI on macOS, WinMM on Windows, ALSA on Linux).
//!
//! Port handling follows EWITool2: ports are identified by label, with
//! " #2", " #3", … added when several ports share a name.

use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

pub type Waker = Box<dyn Fn() + Send + Sync>;

use aerophone_core::session::{Event, Session};
use midir::{
    Ignore, MidiInput, MidiInputConnection, MidiInputPort, MidiOutput, MidiOutputConnection,
    MidiOutputPort,
};

const CLIENT: &str = "AerophoneEditor";

/// Names the AE-30 shows up under, depending on OS and driver.
pub const DEFAULT_PORT_HINTS: [&str; 3] = ["AE-30", "Aerophone", "AE-20"];

fn unique_labels(names: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen: HashMap<String, usize> = HashMap::new();
    names
        .into_iter()
        .map(|name| {
            let n = seen.entry(name.clone()).or_default();
            *n += 1;
            if *n == 1 {
                name
            } else {
                format!("{name} #{n}")
            }
        })
        .collect()
}

fn input_ports(input: &MidiInput) -> Vec<(String, MidiInputPort)> {
    let ports = input.ports();
    let names = ports
        .iter()
        .map(|p| input.port_name(p).unwrap_or_else(|_| "(unnamed)".into()));
    unique_labels(names).into_iter().zip(ports).collect()
}

fn output_ports(output: &MidiOutput) -> Vec<(String, MidiOutputPort)> {
    let ports = output.ports();
    let names = ports
        .iter()
        .map(|p| output.port_name(p).unwrap_or_else(|_| "(unnamed)".into()));
    unique_labels(names).into_iter().zip(ports).collect()
}

/// Labels of the MIDI input and output ports currently available.
pub fn list_ports() -> Result<(Vec<String>, Vec<String>), String> {
    let input = MidiInput::new(CLIENT).map_err(|e| e.to_string())?;
    let output = MidiOutput::new(CLIENT).map_err(|e| e.to_string())?;
    let ins = input_ports(&input).into_iter().map(|(l, _)| l).collect();
    let outs = output_ports(&output).into_iter().map(|(l, _)| l).collect();
    Ok((ins, outs))
}

/// The best label for `hints` (case-insensitive): the first hint that
/// matches anything wins; among its matches, the shortest name (so "AE-30"
/// beats "AE-30 DAW CTRL", which is the unit's DAW-control port), then the
/// first alphabetically ("AE-30 MIDI OUT1" before OUT2, as the Generic USB driver names them on some systems).
fn pick<'a>(labels: &'a [String], hints: &[&str]) -> Option<&'a String> {
    hints.iter().find_map(|h| {
        let h = h.to_lowercase();
        labels
            .iter()
            .filter(|l| l.to_lowercase().contains(&h))
            .min_by_key(|l| (l.to_lowercase().contains("daw"), l.len(), l.as_str()))
    })
}

/// A connection to the AE-30 on one In/Out port pair. Received SysEx is
/// queued; [`Link::pump`] moves it into a [`Session`] and sends whatever the
/// session has due.
pub struct Link {
    _input: MidiInputConnection<()>,
    output: MidiOutputConnection,
    rx: Receiver<Vec<u8>>,
    pub in_label: String,
    pub out_label: String,
}

impl Link {
    /// Open the In and Out ports that best match `hints`. `waker` is called
    /// from the MIDI thread whenever a message arrives (e.g. to repaint).
    pub fn open(hints: &[&str], waker: Option<Waker>) -> Result<Self, String> {
        let mut input = MidiInput::new(CLIENT).map_err(|e| e.to_string())?;
        // SysEx is filtered by default on some backends.
        input.ignore(Ignore::None);
        let output = MidiOutput::new(CLIENT).map_err(|e| e.to_string())?;

        let ins = input_ports(&input);
        let outs = output_ports(&output);
        let in_labels: Vec<String> = ins.iter().map(|(l, _)| l.clone()).collect();
        let out_labels: Vec<String> = outs.iter().map(|(l, _)| l.clone()).collect();
        let not_found = |dir: &str, labels: &[String]| {
            format!("no MIDI {dir} port matching {hints:?} (available: {labels:?})")
        };
        let in_label = pick(&in_labels, hints)
            .ok_or_else(|| not_found("In", &in_labels))?
            .clone();
        let out_label = pick(&out_labels, hints)
            .ok_or_else(|| not_found("Out", &out_labels))?
            .clone();
        let in_port = ins.into_iter().find(|(l, _)| *l == in_label).unwrap().1;
        let out_port = outs.into_iter().find(|(l, _)| *l == out_label).unwrap().1;

        let (tx, rx) = mpsc::channel();
        let input = input
            .connect(
                &in_port,
                "AerophoneEditor In",
                move |_t, msg, _| {
                    let _ = tx.send(msg.to_vec());
                    if let Some(w) = &waker {
                        w();
                    }
                },
                (),
            )
            .map_err(|e| format!("MIDI In \"{in_label}\": {e}"))?;
        let output = output
            .connect(&out_port, "AerophoneEditor Out")
            .map_err(|e| format!("MIDI Out \"{out_label}\": {e}"))?;

        Ok(Link {
            _input: input,
            output,
            rx,
            in_label,
            out_label,
        })
    }

    pub fn send(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.output.send(bytes).map_err(|e| e.to_string())
    }

    /// Feed received SysEx into `session` and send what is due. Returns
    /// the non-SysEx messages received (notes, controllers) for meters.
    pub fn pump(&mut self, session: &mut Session) -> Result<Vec<Vec<u8>>, String> {
        let mut other = Vec::new();
        while let Ok(msg) = self.rx.try_recv() {
            if msg.first() == Some(&0xF0) {
                session.receive(&msg, Instant::now());
            } else {
                other.push(msg);
            }
        }
        while let Some(bytes) = session.poll(Instant::now()) {
            self.send(&bytes)?;
        }
        Ok(other)
    }

    /// Block until the session is idle, handing each event to `on_event`.
    /// Gives up after `limit` without any progress.
    pub fn run(
        &mut self,
        session: &mut Session,
        limit: Duration,
        mut on_event: impl FnMut(&mut Session, Event),
    ) -> Result<(), String> {
        let mut last_progress = Instant::now();
        loop {
            self.pump(session)?;
            let events = session.take_events();
            if !events.is_empty() {
                last_progress = Instant::now();
            }
            for ev in events {
                on_event(session, ev);
            }
            if session.is_idle() {
                return Ok(());
            }
            if last_progress.elapsed() > limit {
                return Err("the instrument stopped answering".into());
            }
            let wait = session
                .next_wake(Instant::now())
                .unwrap_or(Duration::from_millis(5))
                .clamp(Duration::from_millis(1), Duration::from_millis(20));
            if let Ok(msg) = self.rx.recv_timeout(wait) {
                if msg.first() == Some(&0xF0) {
                    session.receive(&msg, Instant::now());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{pick, unique_labels};

    #[test]
    fn duplicate_port_names_get_numbered() {
        let names = ["AE-30", "IAC Bus", "AE-30"];
        assert_eq!(
            unique_labels(names.map(String::from)),
            ["AE-30", "IAC Bus", "AE-30 #2"]
        );
    }

    #[test]
    fn picks_port_by_hint_case_insensitively() {
        let labels = ["IAC Driver Bus 1", "MIDIIN2 (AE-30)"].map(String::from);
        assert_eq!(pick(&labels, &["aerophone", "ae-30"]), Some(&labels[1]));
        assert_eq!(pick(&labels, &["EWI"]), None);
        let ae = ["AE-30 AE-30 DAW CTRL", "AE-30"].map(String::from);
        assert_eq!(pick(&ae, &["AE-30"]), Some(&ae[1]));
        let generic = ["Network Session 1", "AE-30 MIDI OUT2", "AE-30 MIDI OUT1"].map(String::from);
        assert_eq!(pick(&generic, &["AE-30"]), Some(&generic[2]));
    }
}
