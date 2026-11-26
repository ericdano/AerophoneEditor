use ae30_core::map::{self, Block};
use ae30_core::read::BlockRead;
use ae30_core::scene::{SceneCommon, ScenePart, SceneRef, Setup};
use ae30_core::sysex::*;

#[test]
fn checksum_matches_roland_worked_example() {
    // MIDI Implementation: Temp Scene Part level, address 01 00 00 18, value 4A.
    assert_eq!(checksum(&[0x01, 0x00, 0x00, 0x18, 0x4A]), 0x1D);
    assert_eq!(
        dt1(0x10, addr(0x01, 0x00, 0x00, 0x18), &[0x4A]),
        [
            0xF0, 0x41, 0x10, 0x00, 0x00, 0x00, 0x7D, 0x12, 0x01, 0x00, 0x00, 0x18, 0x4A, 0x1D,
            0xF7
        ]
    );
}

#[test]
fn rq1_for_setup_block() {
    let msg = rq1(0x10, map::SETUP.addr, map::SETUP.size);
    // 00+10+00+00+00+00+00+7A = 0x8A -> 138 % 128 = 10 -> 118 = 0x76
    assert_eq!(
        hex(&msg),
        "F0 41 10 00 00 00 7D 11 00 10 00 00 00 00 00 7A 76 F7"
    );
}

#[test]
fn addresses_carry_in_7_bits() {
    assert_eq!(addr(0, 0, 0x7F, 0x7F).offset(Addr(1)), addr(0, 1, 0, 0));
    assert_eq!(map::user_scene(0), Some(addr(0x40, 0x00, 0, 0)));
    assert_eq!(map::user_scene(128), Some(addr(0x41, 0x00, 0, 0)));
    assert_eq!(map::user_scene(599), Some(addr(0x44, 0x57, 0, 0)));
    assert_eq!(map::user_scene(600), None);
    assert_eq!(map::preset_scene(479), Some(addr(0x53, 0x5F, 0, 0)));
    assert_eq!(map::temp_tone(4), Some(addr(0x02, 0x03, 0, 0)));
    let part1 = map::in_scene(map::TEMP_SCENE, map::scene_part(1).unwrap());
    assert_eq!(part1.addr, addr(0x01, 0x00, 0x30, 0x00));
    assert_eq!(map::scene_part(5).unwrap().addr, addr(0, 0, 0x34, 0));
}

#[test]
fn nibbles_round_trip() {
    assert_eq!(unpack_nibbles(&[0x0A, 0x0B, 0x0C, 0x0D]), 0xABCD);
    assert_eq!(pack_nibbles(12000, 4), [0x02, 0x0E, 0x0E, 0x00]);
    assert_eq!(unpack_nibbles(&pack_nibbles(12000, 4)), 12000);
}

#[test]
fn parses_dt1_and_rejects_bad_checksum() {
    let msg = dt1(0x10, addr(0x01, 0x00, 0x00, 0x18), &[0x4A]);
    let set = parse_dt1(&msg).unwrap();
    assert_eq!(set.addr, addr(0x01, 0x00, 0x00, 0x18));
    assert_eq!(set.data, [0x4A]);

    let mut bad = msg.clone();
    bad[12] = 0x4B;
    assert!(matches!(
        parse_dt1(&bad),
        Err(ParseError::BadChecksum { .. })
    ));
    assert_eq!(parse_dt1(&IDENTITY_REQUEST), Err(ParseError::TooShort));
}

#[test]
fn identity_reply() {
    let reply = [
        0xF0, 0x7E, 0x10, 0x06, 0x02, 0x41, 0x7D, 0x03, 0x00, 0x00, 0x01, 0x01, 0x00, 0x01, 0xF7,
    ];
    let id = parse_identity(&reply).unwrap();
    assert_eq!(id.model, Model::Ae30);
    assert_eq!(id.device, 0x10);
    let mut ae20 = reply;
    ae20[8] = 0x01;
    assert_eq!(parse_identity(&ae20).unwrap().model, Model::Ae20);
    assert!(parse_identity(&IDENTITY_REQUEST).is_none());
}

#[test]
fn block_read_assembles_packets_in_any_order() {
    let block = Block::new(addr(0x01, 0x00, 0x00, 0x00), 300);
    let data: Vec<u8> = (0..300).map(|i| (i % 128) as u8).collect();
    let mut read = BlockRead::new(0x10, block);
    let mut packets = dt1_packets(0x10, block.addr, &data, 128);
    packets.reverse();
    // Unrelated traffic is ignored.
    assert!(!read.receive(&dt1(0x10, addr(0x00, 0x10, 0, 0), &[1, 2, 3])));
    for (i, p) in packets.iter().enumerate() {
        assert!(!read.is_complete(), "complete after {i} packets");
        assert!(read.receive(p));
    }
    // Packet 2 starts at 01 00 01 00 (128 in 7-bit addressing).
    assert_eq!(
        parse_dt1(&packets[1]).unwrap().addr,
        addr(0x01, 0x00, 0x01, 0x00)
    );
    assert_eq!(read.into_data().unwrap(), data);
}

fn ascii16(s: &str) -> Vec<u8> {
    format!("{s:<16}").into_bytes()
}

#[test]
fn decodes_scene_common_and_part() {
    let mut common = ascii16("Alto Sax");
    common.extend(pack_nibbles(12000, 4)); // 120.00 BPM
    common.extend([64 + 2, 64 - 1, 0, 0, 100]);
    common.resize(0x5A, 0);
    let c = SceneCommon::decode(&common).unwrap();
    assert_eq!(c.name, "Alto Sax");
    assert_eq!(c.tempo, 120.0);
    assert_eq!((c.transpose, c.octave, c.volume), (2, -1, 100));

    let mut part = vec![0u8; 0x65];
    part[..10].copy_from_slice(&[0, 1, 89, 66, 3, 110, 64, 40, 0, 10]);
    part[0x18] = 65;
    let p = ScenePart::decode(&part).unwrap();
    assert!(p.on && !p.drone);
    assert_eq!(p.tone_bank(), "SuperNATURAL Acoustic");
    assert_eq!((p.level, p.pan, p.octave), (110, 0, 1));
}

#[test]
fn decodes_setup_scene_numbers() {
    let mut setup = vec![0u8; 0x7A];
    setup[..3].copy_from_slice(&[85, 1, 4]);
    setup[0x06] = 3;
    let s = Setup::decode(&setup).unwrap();
    assert_eq!(s.scene(), SceneRef::User(55));
    assert_eq!(s.scene().to_string(), "User 055 (U02-05)");
    assert_eq!(SceneRef::from_bank(85, 64, 0).to_string(), "Preset P01-01");
}
