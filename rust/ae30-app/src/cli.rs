//! Command-line tools (`ae30edit <command>`), for checking the connection and
//! backing up without the GUI. Only `backup` and the read commands exist
//! here; nothing in this file writes to the instrument's memory.

use std::time::{Duration, Instant};

use ae30_core::image::{Area, Image};
use ae30_core::params::data;
use ae30_core::session::{Event, Session};
use ae30_core::sysex::{addr, Addr, Identity};

use crate::midi::{self, Link};

const LIMIT: Duration = Duration::from_secs(6);

pub const USAGE: &str = "\
ae30edit — Roland Aerophone AE-30 editor

USAGE:
  ae30edit                               open the editor
  ae30edit ports                         list MIDI ports
  ae30edit probe                         identify the AE-30, show the current scene
  ae30edit scene                         read the whole Temporary Scene, list its settings
  ae30edit names [user|preset]           list scene names
  ae30edit read AA BB CC DD SIZE         hex-dump bytes (address and plain size in hex)
  ae30edit backup FILE.syx [FIRST LAST]  save user scenes FIRST..LAST (1-600) to a .syx file
  ae30edit restore FILE.syx [SLOT] [--yes]
                                         write a backup's scenes back to their user slots (or
                                         one after another from SLOT, 1-600) and check each;
                                         without --yes, only shows what would be written
  ae30edit stream-test                   time reading one scene with a single request

Option: --port TEXT  use the MIDI port whose name contains TEXT
(default: tries \"AE-30\", \"Aerophone\", \"AE-20\").";

pub fn run(cmd: &str, args: &[String], port: Option<&str>) -> Result<(), String> {
    let hints: Vec<&str> = match port {
        Some(p) => vec![p],
        None => midi::DEFAULT_PORT_HINTS.to_vec(),
    };
    match cmd {
        "ports" => ports(),
        "probe" => probe(&hints),
        "scene" => scene(&hints),
        "names" => names(&hints, args.first().map(String::as_str)),
        "read" => read(&hints, args),
        "backup" => backup(port, args),
        "stream-test" => stream_test(&hints, args),
        "editor-test" => editor_test(port),
        "edit-test" => edit_test(&hints),
        "playlist-test" => playlist_test(port, args),
        "stream-log" => stream_log(&hints, args),
        "listen" => listen(&hints),
        "restore" => restore(port, args),
        "poke-temp" => poke_temp(&hints, args),
        "poke-user" => poke_user(&hints, args),
        "tone-names" => tone_names(&hints),
        "system-set" => system_set(port, args),
        _ => Err(format!("unknown command \"{cmd}\"\n\n{USAGE}")),
    }
}

fn ports() -> Result<(), String> {
    let (ins, outs) = midi::list_ports()?;
    println!("MIDI In:");
    ins.iter().for_each(|l| println!("  {l}"));
    println!("MIDI Out:");
    outs.iter().for_each(|l| println!("  {l}"));
    Ok(())
}

/// Open the ports and confirm an Aerophone answers.
pub fn connect(hints: &[&str]) -> Result<(Link, Session, Identity), String> {
    let mut link = Link::open(hints, None)?;
    let mut session = Session::default();
    session.identify();
    let mut id = None;
    link.run(&mut session, LIMIT, |_, ev| {
        if let Event::Identity(i) = ev {
            id = Some(i);
        }
    })?;
    let id = id.ok_or("no answer to the identity request (is the AE-30 on, USB mode Generic?)")?;
    session.device = id.device;
    eprintln!(
        "{:?} on \"{}\" / \"{}\" (device {:02X})",
        id.model, link.in_label, link.out_label, id.device
    );
    Ok((link, session, id))
}

/// Read every missing block of `img`.
pub fn load(link: &mut Link, s: &mut Session, img: &mut Image) -> Result<(), String> {
    for (i, (a, n)) in img.missing_reads().into_iter().enumerate() {
        s.read(i as u64, a, n);
    }
    let mut failed = Vec::new();
    link.run(s, LIMIT, |_, ev| match ev {
        Event::Data { addr, data } => {
            img.apply(addr, &data);
        }
        Event::ReadDone {
            complete: false,
            addr,
            ..
        } => failed.push(addr),
        _ => {}
    })?;
    if failed.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "no reply for {} reads, first {}",
            failed.len(),
            failed[0]
        ))
    }
}

fn probe(hints: &[&str]) -> Result<(), String> {
    let (mut link, mut s, _) = connect(hints)?;
    let mut setup = Image::new(Area::Setup);
    load(&mut link, &mut s, &mut setup)?;
    let st = &setup.blocks[0];
    let scene = ae30_core::scene::SceneRef::from_bank(
        st.raw(0).unwrap_or(0),
        st.raw(1).unwrap_or(0),
        st.raw(2).unwrap_or(0),
    );
    println!(
        "Current scene: {scene}   battery {}/4",
        st.raw(6).unwrap_or(0)
    );
    let mut img = Image::new(Area::TempScene);
    load(&mut link, &mut s, &mut img)?;
    print_parts(&img);
    Ok(())
}

fn print_parts(img: &Image) {
    println!("\"{}\"", img.scene_name().unwrap_or_default());
    for n in 1..=5 {
        let b = img.block(&format!("Perf_part_{n}")).unwrap();
        let (msb, lsb, pc) = (b.raw(2).unwrap(), b.raw(3).unwrap(), b.raw(4).unwrap());
        let tone = data()
            .tone_name(msb, lsb, pc)
            .map(String::from)
            .unwrap_or_else(|| ae30_core::scene::tone_bank_name(msb, lsb));
        println!(
            "  Part {n} {:<3} {:<24} level {:>3}",
            if b.raw(1) == Some(1) { "on" } else { "-" },
            tone,
            b.raw(5).unwrap()
        );
    }
}

fn scene(hints: &[&str]) -> Result<(), String> {
    let (mut link, mut s, _) = connect(hints)?;
    let mut img = Image::new(Area::TempScene);
    let t = Instant::now();
    load(&mut link, &mut s, &mut img)?;
    println!(
        "Read {} blocks ({} bytes) in {:.2}s",
        img.blocks.len(),
        img.loaded_bytes(),
        t.elapsed().as_secs_f32()
    );
    print_parts(&img);
    for name in ["Perf_common", "Perf_part_1", "Perf_mfx_1", "Perf_rev"] {
        let b = img.block(name).unwrap();
        println!("\n[{name}]");
        let fx = b.get(0).or_else(|| b.get(1)).unwrap_or(0) as u16;
        let fx_type = if name.contains("mfx") {
            b.get(0)
        } else if name.contains("rev") {
            b.get(1)
        } else {
            None
        };
        let mut params: Vec<_> = b.table.fixed().collect();
        if let Some(t) = fx_type {
            params.extend(b.table.for_fx(t as u16));
        }
        let _ = fx;
        for p in params.iter().take(40) {
            let v = b.value(p).unwrap();
            println!("  {:<28} {}", p.label(), p.show(v));
        }
    }
    Ok(())
}

fn names(hints: &[&str], which: Option<&str>) -> Result<(), String> {
    let user = which != Some("preset");
    let (mut link, mut s, _) = connect(hints)?;
    for bank in 0..12 {
        s.list_scenes(user, bank);
    }
    let t = Instant::now();
    let mut count = 0;
    link.run(&mut s, Duration::from_secs(10), |_, ev| {
        if let Event::SceneNames { bank, scenes, .. } = ev {
            println!("Bank {:02}: {} scenes", bank + 1, scenes.len());
            for sc in scenes.iter().take(5) {
                println!("   {:>3}/{:>3}/{:>2}  {}", sc.msb, sc.lsb, sc.pc, sc.name);
            }
            count += scenes.len();
        }
    })?;
    println!("{count} names in {:.1}s", t.elapsed().as_secs_f32());
    Ok(())
}

fn read(hints: &[&str], args: &[String]) -> Result<(), String> {
    let nums: Vec<u32> = args
        .iter()
        .map(|a| u32::from_str_radix(a.trim_start_matches("0x"), 16))
        .collect::<Result<_, _>>()
        .map_err(|e| format!("bad hex number: {e}"))?;
    let [a, b, c, d, size] = nums[..] else {
        return Err("usage: ae30edit read AA BB CC DD SIZE".into());
    };
    if [a, b, c, d].iter().any(|&x| x > 0x7F) || size == 0 {
        return Err("address bytes must be 00-7F and SIZE non-zero".into());
    }
    let start = addr(a as u8, b as u8, c as u8, d as u8);
    let (mut link, mut s, _) = connect(hints)?;
    s.read(0, start, size);
    let mut buf = vec![None; size as usize];
    link.run(&mut s, LIMIT, |_, ev| {
        if let Event::Data { addr, data } = ev {
            for (i, v) in data.iter().enumerate() {
                if let Some(o) = (addr.0 + i as u32).checked_sub(start.0) {
                    if let Some(slot) = buf.get_mut(o as usize) {
                        *slot = Some(*v);
                    }
                }
            }
        }
    })?;
    for (i, row) in buf.chunks(16).enumerate() {
        let hex: Vec<String> = row
            .iter()
            .map(|b| b.map_or("..".into(), |v| format!("{v:02X}")))
            .collect();
        let text: String = row
            .iter()
            .map(|b| match b {
                Some(v) if (32..127).contains(v) => *v as char,
                _ => '.',
            })
            .collect();
        println!("{:04X}  {:<47}  {text}", i * 16, hex.join(" "));
    }
    Ok(())
}

fn stream_test(hints: &[&str], args: &[String]) -> Result<(), String> {
    let (mut link, mut s, _) = connect(hints)?;
    let areas = match args.first() {
        Some(n) => vec![Area::UserScene(
            n.parse::<u16>().map_err(|e| e.to_string())? - 1,
        )],
        None => vec![Area::TempScene, Area::TempTone(1), Area::System],
    };
    for area in areas {
        let mut img = Image::new(area);
        let t = Instant::now();
        let mut packets = 0;
        s.read_area(0, area);
        link.run(&mut s, LIMIT, |_, ev| {
            if let Event::Data { addr, data } = ev {
                packets += 1;
                img.apply(addr, &data);
            }
        })?;
        println!(
        "{area:?}: one request: {packets} packets, {} of {} bytes, all blocks loaded: {}, {:.2}s",
        img.loaded_bytes(),
        area.data_len(),
        img.is_loaded(),
        t.elapsed().as_secs_f32()
    );
        let missing: Vec<_> = img
            .blocks
            .iter()
            .filter(|b| !b.is_loaded())
            .map(|b| b.name())
            .collect();
        if !missing.is_empty() {
            println!("Missing: {missing:?}");
        }
    }
    Ok(())
}

/// Drive the GUI's editor model without a window until everything loaded.
fn editor_test(port: Option<&str>) -> Result<(), String> {
    use crate::editor::{Conn, Editor};
    let mut ed = Editor::new();
    ed.connect(port, None);
    let t = Instant::now();
    let mut scene_at = None;
    while t.elapsed() < Duration::from_secs(30) {
        let wait = ed.tick().unwrap_or(Duration::from_millis(5));
        std::thread::sleep(wait.min(Duration::from_millis(5)));
        if let Conn::Failed(e) = &ed.conn {
            return Err(e.clone());
        }
        if scene_at.is_none() && ed.scene.is_loaded() {
            scene_at = Some(t.elapsed());
        }
        if ed.is_online() && !ed.is_loading() && ed.session.is_idle() && scene_at.is_some() {
            break;
        }
    }
    println!(
        "scene loaded after {:.2}s, all done after {:.2}s",
        scene_at.unwrap_or_default().as_secs_f32(),
        t.elapsed().as_secs_f32()
    );
    println!("current {:?}  \"{}\"", ed.current_scene(), ed.scene_name());
    for p in 1..=5 {
        let tone_loaded = p <= 4 && ed.tones[p as usize - 1].is_loaded();
        println!(
            "  part {p}: {}  (tone image loaded: {tone_loaded})",
            ed.tone_label(p)
        );
    }
    println!("system loaded: {}", ed.system.is_loaded());
    println!(
        "user names known: {}, presets: {}",
        ed.user_names.iter().filter(|n| !n.is_empty()).count(),
        ed.preset_names.len()
    );
    Ok(())
}

/// Live playlist check on the AE-30. Writes only to U12-50, and only if it
/// is empty (INIT SCENE): backs it up to `FILE.syx`, then adds the current
/// scene to an in-memory playlist, changes its reverb level, picks the entry
/// again (which stores it in U12-50 and selects that), reads it back, picks
/// it once more (now just a scene change), restores U12-50 from the backup
/// and returns to the original scene.
fn playlist_test(port: Option<&str>, args: &[String]) -> Result<(), String> {
    use crate::editor::{restore_plan, Editor, Playlist};
    use ae30_core::scene::SceneRef;
    let file = args.first().ok_or("usage: playlist-test BACKUP.syx")?;
    let slot = 599u16;
    let mut ed = Editor::new();
    ed.playlist = Playlist::default(); // not the saved one
    ed.connect(port, None);
    let run = |ed: &mut Editor, what: &str, secs: u64, done: &dyn Fn(&Editor) -> bool| {
        let t = Instant::now();
        while t.elapsed() < Duration::from_secs(secs) {
            let wait = ed.tick().unwrap_or(Duration::from_millis(5));
            std::thread::sleep(wait.min(Duration::from_millis(5)));
            if done(ed) {
                println!("  {what}: {:.2}s", t.elapsed().as_secs_f32());
                return Ok(());
            }
        }
        Err(format!("timed out: {what}"))
    };
    let idle = |ed: &Editor| {
        ed.is_online()
            && !ed.is_loading()
            && ed.session.is_idle()
            && ed.scene.is_loaded()
            && ed.backup.is_none()
            && ed.restore.is_none()
    };
    run(&mut ed, "loaded", 40, &|ed: &Editor| {
        idle(ed) && ed.user_names.iter().all(|n| !n.is_empty())
    })?;
    if ed.user_names[usize::from(slot)] != "INIT SCENE" {
        return Err(format!(
            "U12-50 isn't empty (\"{}\"); not touching it",
            ed.user_names[usize::from(slot)]
        ));
    }
    let original = ed.setup.blocks[0].data[..3].to_vec();
    let home = ed.current_scene();
    println!("current {:?} \"{}\"", home, ed.scene_name());

    ed.start_backup(file.into(), vec![slot]);
    run(&mut ed, "backed up U12-50", 30, &idle)?;

    ed.playlist_add();
    let rev = data().table("Perf_rev").at(2).unwrap();
    let before = ed
        .value(crate::editor::Target::Scene, "Perf_rev", rev)
        .unwrap();
    let want = if before < 127 { before + 1 } else { before - 1 };
    ed.set(crate::editor::Target::Scene, "Perf_rev", rev, want);
    println!("reverb level {before} -> {want}");
    run(&mut ed, "edit copied into the entry", 5, &|ed: &Editor| {
        ed.playlist.entries[0].changed
    })?;

    // Pick the entry: stores it in U12-50 and selects it.
    ed.playlist_play(0);
    let e = &ed.playlist.entries[0];
    println!("entry: slot {:?} stored {}", e.slot, e.stored);
    run(&mut ed, "stored and selected", 30, &idle)?;
    std::thread::sleep(Duration::from_millis(500));
    ed.load_scene();
    run(&mut ed, "read back", 15, &idle)?;
    let now = ed.current_scene();
    let got = ed.value(crate::editor::Target::Scene, "Perf_rev", rev);
    println!("unit now on {now:?}, reverb level {got:?}");
    let ok_store = now == Some(SceneRef::User(slot + 1)) && got == Some(want);
    ed.load_names();
    run(&mut ed, "names", 30, &idle)?;
    println!("U12-50 is now \"{}\"", ed.user_names[usize::from(slot)]);

    // Pick it again: just a scene change now.
    let t = Instant::now();
    ed.select_scene(original[0], original[1], original[2]);
    run(&mut ed, "back to the original", 15, &idle)?;
    ed.playlist_play(0);
    let queued = ed.session.queued_time();
    println!(
        "second pick queued {:.0} ms of MIDI ({:.2}s after leaving)",
        queued.as_secs_f32() * 1000.0,
        t.elapsed().as_secs_f32()
    );
    run(&mut ed, "selected again", 15, &idle)?;
    let ok_again =
        ed.current_scene() == Some(SceneRef::User(slot + 1)) && queued < Duration::from_millis(100);

    // Put U12-50 back and go home.
    let bytes = std::fs::read(file).map_err(|e| e.to_string())?;
    let items = restore_plan(ae30_core::syx::scenes_from_syx(&bytes), None)?;
    ed.start_restore(items);
    run(&mut ed, "U12-50 restored", 60, &idle)?;
    ed.select_scene(original[0], original[1], original[2]);
    run(&mut ed, "home", 15, &idle)?;
    ed.load_names();
    run(&mut ed, "names", 30, &idle)?;
    println!(
        "U12-50 is \"{}\" again; on {:?}",
        ed.user_names[usize::from(slot)],
        ed.current_scene()
    );
    if ok_store && ok_again && ed.current_scene() == home {
        println!("OK");
        Ok(())
    } else {
        Err(format!("failed: stored {ok_store}, reselect {ok_again}"))
    }
}

/// Live-edit check on the Temporary Scene only: lower Part 1's level by
/// one, read it back from the unit, then put it back. Nothing is stored.
fn edit_test(hints: &[&str]) -> Result<(), String> {
    let (mut link, mut s, _) = connect(hints)?;
    let mut img = Image::new(Area::TempScene);
    load(&mut link, &mut s, &mut img)?;
    let i = img.index("Perf_part_1").unwrap();
    let level = data().table("Perf_part").at(5).unwrap();
    let before = img.blocks[i].value(level).unwrap();
    let probe_value = if before > 0 { before - 1 } else { 1 };
    let (a, bytes) = img.set(i, level, probe_value);
    s.write(a, &bytes, false);
    link.run(&mut s, LIMIT, |_, _| {})?;
    let mut check = Image::new(Area::TempScene);
    load(&mut link, &mut s, &mut check)?;
    let after = check.block("Perf_part_1").unwrap().value(level).unwrap();
    let (a, bytes) = img.set(i, level, before);
    s.write(a, &bytes, false);
    link.run(&mut s, LIMIT, |_, _| {})?;
    println!("Part 1 level: was {before}, wrote {probe_value}, unit reports {after}, restored to {before}");
    if after == probe_value {
        Ok(())
    } else {
        Err("the unit did not take the edit".into())
    }
}

/// Back up user scenes FIRST..=LAST (1-based) with the editor's backup code.
fn backup(port: Option<&str>, args: &[String]) -> Result<(), String> {
    use crate::editor::{Conn, Editor};
    let file = args
        .first()
        .ok_or("usage: ae30edit backup FILE.syx [FIRST LAST]")?;
    let num = |i: usize, d: u16| -> Result<u16, String> {
        args.get(i)
            .map_or(Ok(d), |a| a.parse().map_err(|_| format!("bad number {a}")))
    };
    let (first, last) = (num(1, 1)?.clamp(1, 600), num(2, 600)?.clamp(1, 600));
    let mut ed = Editor::new();
    ed.connect(port, None);
    let t = Instant::now();
    let mut started = false;
    let mut last_print = 0;
    loop {
        let wait = ed.tick().unwrap_or(Duration::from_millis(5));
        std::thread::sleep(wait.min(Duration::from_millis(2)));
        if let Conn::Failed(e) = &ed.conn {
            return Err(e.clone());
        }
        if !started && ed.is_online() && ed.scene.is_loaded() {
            ed.start_backup(file.into(), (first - 1..last).collect());
            started = true;
        }
        if let Some(b) = &ed.backup {
            let (done, total) = b.progress();
            if done >= last_print + 10 || done == total {
                last_print = done;
                println!(
                    "{done}/{total} after {:.0}s",
                    b.started.elapsed().as_secs_f32()
                );
            }
        }
        if started && ed.backup.is_none() {
            break;
        }
        if t.elapsed() > Duration::from_secs(3600) {
            return Err("gave up after an hour".into());
        }
    }
    if let Some((msg, _, warn)) = &ed.status {
        println!("{msg}");
        if *warn {
            return Err("backup incomplete".into());
        }
    }
    Ok(())
}

/// Log every DT1 the unit sends for one streamed user-scene request.
fn stream_log(hints: &[&str], args: &[String]) -> Result<(), String> {
    let n: u16 = args.first().and_then(|a| a.parse().ok()).unwrap_or(1);
    let size: Option<u32> = args.get(1).and_then(|a| u32::from_str_radix(a, 16).ok());
    let (mut link, mut s, _) = connect(hints)?;
    let area = Area::UserScene(n - 1);
    let img = Image::new(area);
    let t = Instant::now();
    match size {
        Some(sz) => s.read_raw(0, area.base(), sz, 1 << 20, usize::MAX),
        None => s.read_area(0, area),
    }
    let mut seen = std::collections::BTreeMap::new();
    let mut last = t;
    link.run(&mut s, Duration::from_secs(12), |_, ev| {
        if let Event::Data { addr, data } = ev {
            let now = Instant::now();
            let off = addr.0 - area.base().0;
            let name = img
                .blocks
                .iter()
                .find(|b| (b.addr.0..b.addr.0 + b.data.len() as u32).contains(&addr.0))
                .map_or("?", |b| b.name());
            println!(
                "{:7.1} ms (+{:5.1})  off {:05X} {:3} bytes  {name}",
                t.elapsed().as_secs_f32() * 1000.0,
                now.duration_since(last).as_secs_f32() * 1000.0,
                off,
                data.len()
            );
            last = now;
            *seen.entry(name).or_insert(0) += data.len();
        }
    })?;
    let missing: Vec<_> = img
        .blocks
        .iter()
        .filter(|b| seen.get(b.name()).copied().unwrap_or(0) < b.data.len())
        .map(|b| b.name())
        .collect();
    println!("missing or short: {missing:?}");
    Ok(())
}

/// Count what the unit sends for 5 seconds, without asking anything.
fn listen(hints: &[&str]) -> Result<(), String> {
    let mut link = Link::open(hints, None)?;
    let mut s = Session::default();
    let t = Instant::now();
    let mut n = 0;
    let mut last = None;
    while t.elapsed() < Duration::from_secs(5) {
        link.pump(&mut s)?;
        for ev in s.take_events() {
            if let Event::Data { addr, .. } = ev {
                n += 1;
                last = Some(addr);
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    println!("{n} DT1 messages in 5 s, last at {last:?}");
    Ok(())
}

/// Write scenes from a `.syx` file to user slots, checking each by reading it
/// back. Without `--yes`, only lists what would happen.
fn restore(port: Option<&str>, args: &[String]) -> Result<(), String> {
    use crate::editor::{restore_plan, Conn, Editor};
    let yes = args.iter().any(|a| a == "--yes");
    let args: Vec<&String> = args.iter().filter(|a| *a != "--yes").collect();
    let file = args
        .first()
        .ok_or("usage: ae30edit restore FILE.syx [SLOT] [--yes]")?;
    let start = match args.get(1) {
        Some(a) => Some(
            a.parse::<u16>()
                .ok()
                .filter(|n| (1..=600).contains(n))
                .ok_or(format!("bad slot {a} (1-600)"))?
                - 1,
        ),
        None => None,
    };
    let bytes = std::fs::read(file.as_str()).map_err(|e| format!("{file}: {e}"))?;
    let plan = restore_plan(ae30_core::syx::scenes_from_syx(&bytes), start)?;

    let mut ed = Editor::new();
    ed.connect(port, None);
    let t = Instant::now();
    while !(ed.is_online() && ed.scene.is_loaded() && !ed.is_loading()) {
        let wait = ed.tick().unwrap_or(Duration::from_millis(5));
        std::thread::sleep(wait.min(Duration::from_millis(2)));
        if let Conn::Failed(e) = &ed.conn {
            return Err(e.clone());
        }
        if t.elapsed() > Duration::from_secs(30) {
            return Err("the AE-30 didn't finish loading".into());
        }
    }
    let slot = |n: u16| format!("U{:02}-{:02}", n / 50 + 1, n % 50 + 1);
    for (n, img) in &plan {
        println!(
            "{}  \"{}\"  replaces  \"{}\"",
            slot(*n),
            img.scene_name().unwrap_or_default(),
            ed.user_names[usize::from(*n)]
        );
    }
    if !yes {
        println!(
            "\n{} scenes would be written. Add --yes to write them.",
            plan.len()
        );
        return Ok(());
    }
    ed.start_restore(plan);
    let mut last = usize::MAX;
    while ed.restore.is_some() {
        let wait = ed.tick().unwrap_or(Duration::from_millis(5));
        std::thread::sleep(wait.min(Duration::from_millis(2)));
        if let Some(r) = &ed.restore {
            let (done, total) = r.progress();
            if done != last {
                last = done;
                println!(
                    "{done}/{total} after {:.1}s",
                    r.started.elapsed().as_secs_f32()
                );
            }
        }
        if let Conn::Failed(e) = &ed.conn {
            return Err(e.clone());
        }
    }
    // Let the return to the previous scene go out.
    let t = Instant::now();
    while t.elapsed() < Duration::from_secs(2) {
        ed.tick();
        std::thread::sleep(Duration::from_millis(5));
    }
    match &ed.status {
        Some((msg, _, false)) => {
            println!("{msg}");
            Ok(())
        }
        Some((msg, _, true)) => Err(msg.clone()),
        None => Ok(()),
    }
}

/// Experiment: write one byte into the Temporary Scene (`01 00 BB CC`) and
/// read it back. Not stored. `poke-temp BB CC VALUE` (hex).
fn poke_temp(hints: &[&str], args: &[String]) -> Result<(), String> {
    let n: Vec<u8> = args
        .iter()
        .map(|a| u8::from_str_radix(a, 16))
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;
    let [b, c, v] = n[..] else {
        return Err("usage: poke-temp BB CC VALUE (hex)".into());
    };
    let (mut link, mut s, _) = connect(hints)?;
    let mut img = Image::new(Area::TempScene);
    load(&mut link, &mut s, &mut img)?;
    let at = addr(0x01, 0x00, b, c);
    let before = img.blocks.iter().find_map(|bl| {
        at.since(bl.addr)
            .and_then(|o| bl.data.get(o as usize).copied())
    });
    s.write(at, &[v], false);
    link.run(&mut s, LIMIT, |_, _| {})?;
    let mut check = Image::new(Area::TempScene);
    load(&mut link, &mut s, &mut check)?;
    let after = check.blocks.iter().find_map(|bl| {
        at.since(bl.addr)
            .and_then(|o| bl.data.get(o as usize).copied())
    });
    println!("{at}: was {before:02X?}, wrote {v:02X}, reads {after:02X?}");
    Ok(())
}

/// Experiment: DT1 one byte straight into stored user scene SLOT (1-600) at
/// block offset `BB CC`, then read the whole block back. `poke-user SLOT BB CC VALUE`.
fn poke_user(hints: &[&str], args: &[String]) -> Result<(), String> {
    let slot: u16 = args
        .first()
        .and_then(|a| a.parse().ok())
        .filter(|n| (1..=600).contains(n))
        .ok_or("bad slot")?;
    let n: Vec<u8> = args[1..]
        .iter()
        .map(|a| u8::from_str_radix(a, 16))
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;
    let [b, c, v] = n[..] else {
        return Err("usage: poke-user SLOT BB CC VALUE (hex)".into());
    };
    let area = Area::UserScene(slot - 1);
    let (mut link, mut s, _) = connect(hints)?;
    let at = Addr(area.base().0 + ((b as u32) << 7) + c as u32);
    let mut img = Image::new(area);
    s.read_blocks(0, img.missing_reads());
    link.run(&mut s, LIMIT, |_, ev| {
        if let Event::Data { addr, data } = ev {
            img.apply(addr, &data);
        }
    })?;
    let get = |img: &Image| {
        img.blocks.iter().find_map(|bl| {
            at.since(bl.addr)
                .and_then(|o| bl.data.get(o as usize).copied())
        })
    };
    let before = get(&img);
    s.write(at, &[v], false);
    link.run(&mut s, LIMIT, |_, _| {})?;
    std::thread::sleep(Duration::from_millis(500));
    let mut check = Image::new(area);
    s.read_blocks(0, check.missing_reads());
    link.run(&mut s, LIMIT, |_, ev| {
        if let Event::Data { addr, data } = ev {
            check.apply(addr, &data);
        }
    })?;
    println!(
        "{at}: was {before:02X?}, wrote {v:02X}, reads {:02X?}",
        get(&check)
    );
    Ok(())
}

/// List the user tone names (non-empty ones).
fn tone_names(hints: &[&str]) -> Result<(), String> {
    let (mut link, mut s, _) = connect(hints)?;
    s.list_user_tones();
    let mut names = None;
    link.run(&mut s, Duration::from_secs(10), |_, ev| {
        if let Event::UserToneNames(n) = ev {
            names = Some(n);
        }
    })?;
    let names = names.ok_or("no tone list")?;
    for (i, n) in names.iter().enumerate() {
        if !n.is_empty() && n != "INIT TONE" {
            println!("{:3} {n}", i + 1);
        }
    }
    Ok(())
}

/// Set one System setting by its label and store System settings:
/// `system-set "Display Contrast" 3`.
fn system_set(port: Option<&str>, args: &[String]) -> Result<(), String> {
    use crate::editor::{Conn, Editor, Target};
    let [label, value] = args else {
        return Err("usage: system-set LABEL VALUE".into());
    };
    let value: i32 = value.parse().map_err(|_| "bad value")?;
    let p = ae30_core::data()
        .table("Sys_common")
        .named(label)
        .ok_or(format!("no System setting called {label}"))?;
    let mut ed = Editor::new();
    ed.connect(port, None);
    let t = Instant::now();
    while ed.value(Target::System, "Sys_common", p).is_none() {
        let wait = ed.tick().unwrap_or(Duration::from_millis(5));
        std::thread::sleep(wait.min(Duration::from_millis(2)));
        if let Conn::Failed(e) = &ed.conn {
            return Err(e.clone());
        }
        if t.elapsed() > Duration::from_secs(30) {
            return Err("System settings didn't load".into());
        }
    }
    let before = ed.value(Target::System, "Sys_common", p).unwrap();
    ed.set(Target::System, "Sys_common", p, value);
    ed.session.write_system();
    let t = Instant::now();
    while t.elapsed() < Duration::from_secs(2) {
        ed.tick();
        std::thread::sleep(Duration::from_millis(5));
    }
    println!("{label}: {} -> {} (stored)", p.show(before), p.show(value));
    Ok(())
}
