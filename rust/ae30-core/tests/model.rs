use std::time::{Duration, Instant};

use ae30_core::image::{name_bytes, Area, Image};
use ae30_core::params::{data, Format};
use ae30_core::session::{cmd, Event, Session};
use ae30_core::sysex::{self, addr, Addr};
use ae30_core::syx;

#[test]
fn data_loads_and_sizes_match_roland() {
    let d = data();
    let size = |t: &str| d.table(t).size;
    assert_eq!(size("Perf_common"), 0x5A);
    assert_eq!(size("Perf_part"), 0x65);
    assert_eq!(size("Perf_mfx"), 0x90); // 01 10
    assert_eq!(size("Perf_rev"), 0x53);
    assert_eq!(size("PCMEX_ptlLfo"), 0x9E); // 01 1E
    assert_eq!(size("stp"), 0x7A);
    assert_eq!(d.layouts.scene.len(), 56);
    // Part 3 sits at 00 32 00.
    let p3 = d
        .layouts
        .scene
        .iter()
        .find(|e| e.name == "Perf_part_3")
        .unwrap();
    assert_eq!(
        Area::TempScene.base().offset(Addr(p3.off)),
        addr(1, 0, 0x32, 0)
    );
    assert_eq!(d.tone_name(71, 72, 0), Some("Classic Saw !"));
}

#[test]
fn params_decode_encode_and_format() {
    let part = data().table("Perf_part");
    let pan = part.at(6).unwrap();
    let mut block = vec![0u8; 0x65];
    block[6] = 64;
    assert_eq!(pan.get(&block), Some(0));
    assert_eq!(pan.show(0), "Center");
    assert_eq!(pan.show(-10), "L10");
    assert_eq!(pan.set(&mut block, 63), [127]);
    assert_eq!(pan.set(&mut block, 99), [127], "clamped");

    let porta = part.at(40).unwrap(); // Portamento Time, 2 nibbles, 128 = TONE
    assert_eq!((porta.n, porta.bits), (2, 4));
    assert_eq!(porta.encode(128), [0x08, 0x00]);
    assert_eq!(porta.show(128), "TONE");
    assert_eq!(porta.show(20), "20");

    let tempo = data().table("Perf_common").at(16).unwrap();
    assert_eq!(tempo.encode(12000), sysex::pack_nibbles(12000, 4));
    assert_eq!(tempo.show(12000), "120.00");

    let eq = data().table("Perf_eq").at(1).unwrap();
    assert_eq!(eq.show(-3), "-3 dB");
    assert_eq!(eq.show(5), "+5 dB");
    assert_eq!(eq.encode(-24), [40]);
}

#[test]
fn every_format_renders() {
    for (name, t) in &data().tables {
        for p in &t.params {
            for v in [p.min, p.max, (p.min + p.max) / 2] {
                let s = p.show(v);
                assert!(!s.is_empty(), "{name} {} {v}", p.label());
            }
        }
    }
    let f = Format::DryWet;
    assert_eq!(f.show(30), "D70:30W");
}

#[test]
fn effect_parameters_follow_type() {
    let mfx = data().table("Perf_mfx");
    // Type 22 (VK Rotary) has its own named parameters.
    let rotary: Vec<_> = mfx
        .for_fx(22)
        .iter()
        .map(|p| p.label().to_string())
        .collect();
    assert!(rotary.contains(&"Speed".to_string()), "{rotary:?}");
    // Thru (0) falls back to the generic slots.
    assert!(mfx.for_fx(0).iter().all(|p| p.generic));
    assert!(mfx.fixed().any(|p| p.label() == "MFX Switch"));
}

#[test]
fn image_reads_applies_and_retargets() {
    let mut img = Image::new(Area::TempScene);
    let reads = img.missing_reads();
    // Whole blocks only: stored scenes ignore requests starting mid-block.
    assert_eq!(reads.len(), img.blocks.len());
    assert!(reads
        .iter()
        .zip(&img.blocks)
        .all(|(&(a, n), b)| a == b.addr && n as usize == b.data.len()));
    let total: u32 = reads.iter().map(|&(_, n)| n).sum();
    assert_eq!(total as usize, Area::TempScene.data_len());

    // Fill everything as the instrument would.
    for &(a, n) in &reads {
        img.apply(a, &vec![64; n as usize]);
    }
    assert!(img.is_loaded());
    let i = img.index("Perf_common").unwrap();
    img.set_bytes(i, 0, &name_bytes("My Sax"));
    assert_eq!(img.scene_name().as_deref(), Some("My Sax"));

    let u = img.retarget(Area::UserScene(599));
    assert_eq!(u.blocks[0].addr, addr(0x44, 0x57, 0, 0));
    assert_eq!(u.scene_name().as_deref(), Some("My Sax"));
}

#[test]
fn syx_round_trip() {
    let mut img = Image::new(Area::UserScene(7));
    for (a, n) in img.missing_reads() {
        img.apply(a, &vec![1; n as usize]);
    }
    let i = img.index("Perf_common").unwrap();
    img.set_bytes(i, 0, &name_bytes("Round Trip"));

    let file = syx::scene_to_syx(&img, 0x10);
    let back = syx::scene_from_syx(&file).unwrap();
    assert_eq!(back.area, Area::TempScene);
    assert!(back.is_loaded());
    assert_eq!(back.scene_name().as_deref(), Some("Round Trip"));

    let backup = syx::backup_to_syx(&[img.clone(), img.retarget(Area::UserScene(300))], 0x10);
    let scenes = syx::scenes_from_syx(&backup);
    assert_eq!(
        scenes.iter().map(|(a, _)| *a).collect::<Vec<_>>(),
        [Area::UserScene(7), Area::UserScene(300)]
    );
}

fn drain(s: &mut Session, now: Instant) -> Vec<Vec<u8>> {
    let mut v = Vec::new();
    while let Some(m) = s.poll(now) {
        v.push(m);
    }
    v
}

#[test]
fn session_paces_and_merges_edits() {
    let t0 = Instant::now();
    let mut s = Session::new(0x10);
    s.write(addr(1, 0, 0x30, 5), &[10], true);
    s.write(addr(1, 0, 0x30, 6), &[64], true);
    s.write(addr(1, 0, 0x30, 5), &[90], true); // replaces the first
    let first = drain(&mut s, t0);
    assert_eq!(first.len(), 1, "paced: one message per slot");
    assert_eq!(sysex::parse_dt1(&first[0]).unwrap().data, [90]);
    let later = drain(&mut s, t0 + Duration::from_millis(30));
    assert_eq!(later.len(), 1);
    assert!(s.is_idle());
}

#[test]
fn session_read_completes_and_retries() {
    let t0 = Instant::now();
    let mut s = Session::new(0x10);
    s.read(1, addr(1, 0, 0, 0), 0x5A);
    let sent = drain(&mut s, t0);
    assert_eq!(sent, [sysex::rq1(0x10, addr(1, 0, 0, 0), 0x5A)]);

    // No reply: it asks again.
    let t1 = t0 + Duration::from_secs(2);
    assert_eq!(drain(&mut s, t1).len(), 1);

    s.receive(&sysex::dt1(0x10, addr(1, 0, 0, 0), &[0x41; 0x5A]), t1);
    let ev = s.take_events();
    assert!(matches!(ev[0], Event::Data { .. }));
    assert!(matches!(
        ev[1],
        Event::ReadDone {
            tag: 1,
            complete: true,
            ..
        }
    ));
    assert!(s.is_idle());
}

#[test]
fn session_collects_scene_names() {
    let t0 = Instant::now();
    let mut s = Session::new(0x10);
    s.list_scenes(true, 3);
    let sent = drain(&mut s, t0);
    let set = sysex::parse_dt1(&sent[0]).unwrap();
    assert_eq!(set.addr, cmd::SCENE_LIST);
    assert_eq!(set.data[..2], [1, 3]);

    let entry = |pc: u8, name: &str| {
        let mut d = vec![1, 3, 0, 85, 3, pc, 0, 0, 0, 0];
        d.extend(name_bytes(name));
        sysex::dt1(0x10, cmd::SCENE_LIST_ENTRY, &d)
    };
    s.receive(&entry(0, "Tenor"), t0);
    s.receive(&entry(1, "Alto"), t0);
    let mut end = vec![1, 3, 0, 0, 0, 0, 0, 0, 0, 0];
    end.extend([b' '; 16]);
    s.receive(&sysex::dt1(0x10, cmd::SCENE_LIST_ENTRY, &end), t0);
    let ev = s.take_events();
    let Event::SceneNames { user, bank, scenes } = &ev[0] else {
        panic!("{ev:?}")
    };
    assert!(*user);
    assert_eq!(*bank, 3);
    assert_eq!(scenes.len(), 2);
    assert_eq!(scenes[1].name, "Alto");
}

#[test]
fn write_commands() {
    let mut s = Session::new(0x10);
    s.write_scene(599);
    s.write_tone(2, 300);
    let t0 = Instant::now();
    let a = sysex::parse_dt1(&drain(&mut s, t0)[0]).unwrap();
    assert_eq!(a.addr, addr(0x0F, 0, 0x10, 0x01));
    assert_eq!(a.data, [0, 2, 5, 7]); // 599 = 0x257
    let b = sysex::parse_dt1(&drain(&mut s, t0 + Duration::from_secs(1))[0]).unwrap();
    assert_eq!(b.addr, addr(0x0F, 0, 0x10, 0x09));
    assert_eq!(b.data, [0, 1, 2, 12]); // 300 = 0x12C
}

#[test]
fn identity_is_retried() {
    let t0 = Instant::now();
    let mut s = Session::new(0x10);
    s.identify();
    assert_eq!(drain(&mut s, t0), [sysex::IDENTITY_REQUEST.to_vec()]);
    let t1 = t0 + Duration::from_secs(2);
    assert_eq!(
        drain(&mut s, t1),
        [sysex::IDENTITY_REQUEST.to_vec()],
        "asked again"
    );
    assert!(s.take_events().is_empty());
    let reply = [
        0xF0, 0x7E, 0x7F, 0x06, 0x02, 0x41, 0x7D, 0x03, 0x00, 0x00, 0, 1, 0, 0, 0xF7,
    ];
    s.receive(&reply, t1);
    assert!(matches!(s.take_events()[0], Event::Identity(_)));
}

#[test]
fn batch_reads_go_out_back_to_back_and_complete() {
    let t0 = Instant::now();
    let mut s = Session::new(0x10);
    let img = Image::new(Area::UserScene(4));
    let reads = img.missing_reads();
    let total = reads.len();
    s.read_blocks(9, reads.clone());
    // All requests leave without waiting for replies (only paced).
    let mut sent = Vec::new();
    for ms in 0..2000 {
        sent.extend(drain(&mut s, t0 + Duration::from_millis(ms)));
    }
    assert_eq!(sent.len(), total);
    let t1 = t0 + Duration::from_secs(2);
    for (i, &(a, n)) in reads.iter().enumerate() {
        s.receive(&sysex::dt1(0x10, a, &vec![1; n as usize]), t1);
        let done = s.take_events().iter().any(|e| {
            matches!(
                e,
                Event::ReadDone {
                    tag: 9,
                    complete: true,
                    ..
                }
            )
        });
        assert_eq!(done, i == total - 1);
    }
    assert!(s.is_idle());
}

#[test]
fn abort_drops_unsent_requests() {
    let t0 = Instant::now();
    let mut s = Session::new(0x10);
    s.read_blocks(9, Image::new(Area::UserScene(0)).missing_reads());
    assert_eq!(drain(&mut s, t0).len(), 1);
    s.abort(9);
    assert!(s.is_idle());
}
