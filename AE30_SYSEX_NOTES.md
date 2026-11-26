# Roland Aerophone Pro AE-30 — SysEx Research Notes

Research toward a Rust patch editor for the AE-30 (companion to the Akai EWI editor).
Compiled 2026-11-25 from Roland's published *AE-30/AE-20 MIDI Implementation* (v2.01, Jan 2022)
and the open-source `aerophoneCompanion` project (MIT).

---

## 1. Bottom line

- Roland publishes the **full SysEx parameter address map** for the AE-30. Reading and live-editing
  scenes/tones is fully documented.
- Protocol is standard Roland **RQ1/DT1** (same family as Fantom, Jupiter-X, other ZEN-Core gear).
- The only undocumented part is the **"store to user slot" command**, which an open-source AE-20
  project reverse-engineered from Roland's app. Needs verification on the AE-30 (same model ID).
- `midir` handles SysEx on all platforms, so the transport layer from the EWI editor is reusable.

---

## 2. Message formats

### Header

```
F0 41 <dev> 00 00 00 7D <cmd> ... <sum> F7
```

| Byte(s)       | Meaning                                                              |
|---------------|----------------------------------------------------------------------|
| `41`          | Roland manufacturer ID                                               |
| `<dev>`       | Device ID, `10H`–`1FH`, default **`10H`**; `7FH` = broadcast          |
| `00 00 00 7D` | Model ID — identical for AE-30 and AE-20                              |
| `<cmd>`       | `11H` = RQ1 (data request), `12H` = DT1 (data set)                    |

### RQ1 — request data

```
F0 41 10 00 00 00 7D 11  aa bb cc dd  ss tt uu vv  sum F7
                         └─address─┘  └──size───┘
```

Unit replies with one or more DT1 messages. Requests must use the fixed start address and size
of a block from the address map (not arbitrary ranges), though the companion app reads in
128-byte chunks successfully.

### DT1 — set data (both directions)

```
F0 41 10 00 00 00 7D 12  aa bb cc dd  <data...>  sum F7
```

- Max **256 bytes** of data per packet; multi-packet transfers spaced **~20 ms** apart.
- Writing to the Temporary Scene / Temporary Tone area changes the sound **immediately**.
- The AE-30 also *transmits* DT1 on its own when you edit from the panel, if Setup
  "Transmit Edit Data" (`00 10 00 07`) is On — useful for a live-mirroring editor.

### Checksum

Sum of address bytes + data (or size) bytes, then:

```
checksum = (128 - (sum % 128)) & 0x7F
```

Worked example from Roland's doc — set Temp Scene Part level, address `01 00 00 18`, value `4A`:

```
01 + 00 + 00 + 18 + 4A = 0x63 (99)   → 128 - 99 = 29 = 0x1D
F0 41 10 00 00 00 7D 12 01 00 00 18 4A 1D F7
```

Rust sketch:

```rust
fn roland_checksum(addr_and_data: &[u8]) -> u8 {
    let sum: u32 = addr_and_data.iter().map(|&b| b as u32).sum();
    ((128 - (sum % 128)) & 0x7F) as u8
}
```

### Value encoding

- Most params: one 7-bit byte, `0aaa aaaa`.
- Wider params are **nibble-packed**, 4 bits per byte, MSB first, e.g. a 4-byte value
  `0a 0b 0c 0d` = `((a*16 + b)*16 + c)*16 + d`. Marked with `#` in the map.
- Signed values are stored with an offset (e.g. `40H` = 0 for ±63 ranges; `28..E4` = −100..+100).

### Identity Request / Reply

```
Request: F0 7E 10 06 01 F7
Reply:   F0 7E 10 06 02 41 7D 03 aa 00 01 01 00 01 F7
                              └─family 7D 03─┘
         aa = 00 → AE-30,  01 → AE-20
```

---

## 3. Parameter address map (top level)

| Start address | Block                              | Notes                                   |
|---------------|------------------------------------|-----------------------------------------|
| `00 00 00 00` | **System**                         | global settings, assigns, fingering, favorites |
| `00 10 00 00` | **Setup**                          | current scene BS/PC, battery, tx-edit flag |
| `01 00 00 00` | **Temporary Scene**                | the sound currently playing             |
| `02 00 00 00` | Temporary Tone PCM (1)             | full ZEN-Core tone for Part 1           |
| `02 01 00 00` | Temporary Tone PCM (2)             |                                          |
| `02 02 00 00` | Temporary Tone PCM (3)             |                                          |
| `02 03 00 00` | Temporary Tone PCM (4)             |                                          |
| `40 00 00 00` | User Scene 001                     | 600 slots; address = `40 00` + (n−1) in 7-bit |
| `44 57 00 00` | User Scene 600                     |                                          |
| `50 00 00 00` | Preset Scene 001                   | 480 presets, readable                   |
| `53 5F 00 00` | Preset Scene 480                   |                                          |

User scene *n* (0-based) → second address byte advances 7-bit style: `40 00`, `40 01` … `40 7F`, `41 00` …

### Setup block (`00 10 00 00`, size `7A`)

| Offset | Param |
|--------|-------|
| `00` | Scene Bank Select MSB (CC#0) |
| `01` | Scene Bank Select LSB (CC#32) |
| `02` | Scene PC |
| `03` | Favorite Number (0–11) |
| `06` | Battery Level (0–4) |
| `07` | Transmit Edit Data (0/1) |
| `08–0F` | Fingering Status (nibbled 28-bit) |
| `11` | Drone Switch |
| `12` | Harmony Switch |

Writing `[msb, lsb, pc]` to `00 10 00 00` **selects a scene** (the companion app does exactly this).

### Scene bank/PC numbering

| MSB | LSB      | PC      | Meaning                                   |
|-----|----------|---------|-------------------------------------------|
| 85  | 0–11     | 0–49    | User banks U01–U12 (50 scenes each = 600) |
| 85  | 64–75    | 0–…     | Preset categories P01–P12                 |

Tone banks (for Scene Part → Tone Bank Select): MSB 71 LSB 8–11 = User Tones (4×128 = 512),
MSB 71 LSB 72–74 = ZEN-Core PR-A/B/C, MSB 89 LSB 66 = SuperNATURAL Acoustic, MSB 70 LSB 66 = Drum Kit.

---

## 4. Scene layout (offsets relative to scene base)

| Offset    | Block                           | Size   |
|-----------|---------------------------------|--------|
| `00 00 00` | Scene Common                   | `5A`   |
| `00 01 00` | S1 Button Assign [Assign2]      | `20`   |
| `00 02 00` | S2 Button Assign [Assign2]      | `20`   |
| `00 03 00`–`00 0A 00` | Side Key X, C1–C5, Tc, Ta Assign [Assign2] | `20` each |
| `00 0B 00` | Breath Controller Assign [Assign8] | `01 00` |
| `00 0D 00` / `00 0E 00` | Bite Down / Up Assign [Assign4] | `40` each |
| `00 0F 00` / `00 10 00` | Wheel Down / Up Assign [Assign2] | `20` each |
| `00 11 00` | Thumb Pad Assign [Assign4] (AE-30 only) | `40` |
| `00 12 00` | Motion Controller Assign [Assign2] (AE-30 only) | `20` |
| `00 13 00`–`00 24 00` | Same controllers again, "MIDI OUT" variants (for MIDI Control mode) | |
| `00 30 00`–`00 34 00` | **Scene Tone (Part) 1–5** [Scene Part] | `65` each |
| `00 40 00`–`00 44 00` | Scene EQ 1–5 [Part EQ]        | `09` each |
| `00 50 00`, `52`, `54`, `56`, `58` | Scene MFX 1–5 [MFX] | `01 10` each |
| `00 60 00` / `00 61 00` | MIDI Control Mode 1 / 2    | `2E` each |
| `00 70 00` | Scene Chorus [Chorus/Delay]     | `54`   |
| `00 71 00` | Scene Delay [Chorus/Delay]      | `54`   |
| `00 72 00` | Scene Reverb [Reverb]           | `53`   |
| `00 73 00` | Scene IFX [MFX]                 | `01 10` |

### Scene Common (size `5A`)

| Offset | Param | Range |
|--------|-------|-------|
| `00–0F` | Name (16 ASCII chars) | 32–127 |
| `10–13` | Scene Tempo (nibbled) | 2000–30000 = 20.00–300.00 |
| `14` | Scene Transpose | 59–70 = −5..+6 |
| `15` | Scene Octave Shift | 61–67 = −3..+3 |
| `18` | Scene Volume | 0–127 |
| `19` | IFX Delay Send Level | 0–127 |
| `1B` | Controller Bend Range | 0–24 |
| `1C–1F` | Control Source 1–4 | 0–96 (Off, CC01…CC95, Bend, Aftertouch) |
| `20` / `21` | Bite Range Down / Up | 0–34 (cent table) |
| `29–2C` | Harmony 1–4 Assign | 0–24 |
| `33` / `34` | Bend Range Motion Down / Up (AE-30) | 0–34 |
| `35` / `36` | Motion_1 / Motion_2 Setting (AE-30) | Elevation, Tilt, … |

### Scene Part (size `65`) — the "patch layer"

| Offset | Param | Range |
|--------|-------|-------|
| `00` | Part Mode | 0 Lead, 1 Drone |
| `01` | Part Switch | Off/On |
| `02` | Tone Bank Select MSB | 0–127 |
| `03` | Tone Bank Select LSB | 0–127 |
| `04` | Tone Program Change | 0–127 |
| `05` | Part Level (CC#7) | 0–127 |
| `06` | Part Pan (CC#10) | 0–127, 64 = center |
| `07` | Reverb Send (CC#91) | 0–127 |
| `08` | Chorus Send (CC#93) | 0–127 |
| `09` | Delay Send | 0–127 |
| `0B` | Output Assign | 0 Dry, 1 IFX |
| `0E` / `0F` | Keyboard Range Lower / Upper | 0–127 |
| `10` / `11` | Keyboard Fade Width Lower / Upper | 0–127 |
| `12` / `13` | Velocity Range Lower / Upper | 1–127 |
| `14` / `15` | Velocity Fade Lower / Upper | 0–127 |
| `16` | Part Coarse Tune | 16–112 = −48..+48 |
| `17` | Part Fine Tune | 14–114 = −50..+50 |
| `18` | Part Octave Shift | 61–67 = −3..+3 |
| `1B` | Cutoff Offset (CC#74) | 0–127 = −64..+63 |
| `1C` | Resonance Offset (CC#71) | " |
| `1D` | Attack Time Offset (CC#73) | " |
| `1E` | Decay Time Offset (CC#75) | " |
| `1F` | Release Time Offset (CC#72) | " |
| `20` / `21` / `22` | Vibrato Rate / Depth / Delay | " |
| `24` | Mono/Poly | 0 Mono, 1 Poly, 2 Tone |
| `25` | Legato Switch | Off, On, Tone |
| `26` | Portamento Switch (CC#65) | Off, On, Tone |
| `28–29` | Portamento Time (nibbled) | 0–128 (128 = TONE) |
| `2A` | Unison Switch | Off, On, Tone |
| `3A` | Keyboard Velocity Mode | Real, Fixed |
| `3B` | Keyboard Fixed Velocity | 1–127 |
| `48–58` | Rx switches: S1, S2, Side keys X/C1–C5/Tc/Ta, Breath, Bite Dn/Up, Wheel Dn/Up, Thumb Pad, Motion | Off/On each |
| `59–5D` | X-Fade Range Low / Up / Fade Low / Fade Up / Position | 0–127 |

### Part EQ (size `09`)

`00` Input Gain, `01` Low Gain, `02` Mid Gain, `03` High Gain (40–88 = −24..+24 dB),
`04`–`06` Low/Mid/High Freq (0–29, table 20 Hz–16 kHz), `07` Mid Q (0–5), `08` EQ Switch.

### MFX / IFX (size `01 10`)

`00` MFX Type (0–92), `01` Switch, `02` Chorus Send, `03` Reverb Send, `04`–`0B` CtrlSrc/Sens 1–4,
`0C`–`0F` CtrlAsgn 1–4, `10`+ 32 × 4-byte nibbled parameters (12768–52768 = −20000..+20000).

### Reverb (size `53`)

`00` Switch, `01` Type (0–7), `02` Level, `03`+ 20 × 4-byte nibbled parameters.

### Chorus / Delay (size `54`)

`00` Switch, `01` Type (0–9), `02` Level, `03` Reverb Send, `04`+ 20 × 4-byte nibbled parameters.

### Assign blocks

Each assign slot is 16 bytes: `00` Function (0–113: Off, CC01…CC95, Bend Down/Up, Aftertouch,
Scene Down/Up, Favorite Down/Up, Chorus/Reverb/Delay/IFX/Unison Sw, Oct Down/Up, Transpose Down/Up,
Drone Sw, Harmony Sw, Start/Stop, X-Fade), `02` Input Min, `03` Input Max, `04` Output Min,
`05` Output Max, `06` Mode (Latch/Momentary), `07` Curve (0–11).
Assign2 = 2 slots (`20`), Assign4 = 4 slots (`40`), Assign8 = 8 slots (`01 00`).

---

## 5. Tone layout (ZEN-Core; offsets relative to `02 0n 00 00`)

| Offset | Block | Size |
|--------|-------|------|
| `00 00 00` | Tone Common | `36` |
| `00 01 00` | Tone MFX [MFX] | `01 10` |
| `00 10 00` | Partial Mix Table | `26` |
| `00 20 00`–`00 23 00` | Partial 1–4 | `01 00` each |
| `00 24 00`–`00 27 00` | Partial Pitch Env 1–4 | `2D` each |
| `00 28 00`–`00 2B 00` | Partial Filter Env 1–4 | `2C` each |
| `00 2C 00`–`00 2F 00` | Partial Amp Env 1–4 | `20` each |
| `00 30 00`, `32`, `34`, `36` | Partial LFO 1–4 (two LFOs + step seq each) | `01 1E` each |
| `00 38 00`–`00 3B 00` | Partial EQ 1–4 | `1B` each |
| `00 3C 00` | Synth Common (unison, pitch drift, condition) | `09` |
| `00 3D 00` | Synth Partial Mix Table (structure sync/ring/xmod) | `17` |
| `00 3E 00`–`00 41 00` | Synth Partial 1–4 (OSC type PCM/VA/PCM-Sync/SuperSAW/Noise, PW, filter type/slope…) | `1D` each |

Tone Common highlights: `00–0F` Name, `10` Category, `15` Level, `16` Pan, `1D` Mono/Poly,
`1E` Legato, `20–23` Portamento Sw/Mode/Type/Start, `24–27` Portamento Time (nibbled),
`28`/`29` Bend Range Up/Down.

Partial highlights: `00` Level, `01` Coarse, `02` Fine, `07` Pan, `1B` Wave Group Type,
`1C–1F` Wave Group ID, `20–23` Wave Number L, `24–27` Wave Number R, `31` TVF Filter Type,
`32–35` Cutoff, `3D–40` Resonance, `56`+ four matrix-control slots (Source, 4 × Dest/Sens).

**Limitation:** SuperNATURAL Acoustic tones (MSB 89) are selectable but their internals are not in
the map — only ZEN-Core PCM/VA tones are fully editable. Same limitation as Roland's own app.

---

## 6. Undocumented command area (reverse-engineered, from aerophoneCompanion)

Not in the Roland PDF. Verified by the companion project on the **AE-20**; expected to carry over
to the AE-30 (same model ID), but **verify on a throwaway slot first**.

All are DT1 writes. Addresses below are in 7-bit form (what goes on the wire).

| Address (7-bit) | Data | Effect |
|-----------------|------|--------|
| `0F 00 10 00` | `[0]` | **System write** (commit System block to flash) |
| `0F 00 10 01` | `[0, n>>8 & 0xF, n>>4 & 0xF, n & 0xF]` | **Write Temp Scene to User Scene n** (n = lsb×50 + pc, 0–599) |
| `0F 00 10 05 + 4·(part−1)` | `[0, idx>>8 & 0xF, idx>>4 & 0xF, idx & 0xF]` | Save Part's temp tone to **User Tone slot idx** (0–511) |
| `0F 00 41 00` | `[g, bank, 0,0,0,0,0, 0F,0F,0F,0F]` | **List scene names**: g=1 user / g=0 preset, bank 0–11. Unit streams DT1s at `0F 00 41 24`: `gg bb dd mm ll pp ii ii ii ii name[16]`, ends with msb=lsb=pc=0 |
| `0F 00 42 00` | `[1, 0,0,0,0,0,0, 0F,0F,0F,0F]` | **List user tone names**: streams DT1s at `0F 00 42 24`: `gg ee dd mm ll pp name[16]` |

Open question: whether a plain DT1 directly into `40 xx 00 00` (User Scene area) persists without
the write command. The companion app avoids this and uses `0F 00 10 01`; safest to follow it.

---

## 7. Timing / transport notes

- Companion app sends in **128-byte chunks** and paces output at roughly
  `bytes × 1000 / 3125 + 8 ms` (i.e. MIDI wire rate plus margin).
- Reading a full scene = ~25 RQ1 round-trips; a full tone = ~30 more. Fine for an editor, slow for
  a 600-scene backup (minutes) — do it in the background with progress.
- **USB:** Windows/macOS need Roland's Aerophone USB driver *or* switch the unit's USB driver mode to
  "Generic" (class-compliant). Linux is class-compliant out of the box in Generic mode.
- **DIN MIDI** and **Bluetooth LE MIDI** also carry SysEx; USB is simplest.
- Close Roland's phone app / turn off Bluetooth on the unit while testing so two editors don't fight.
- Device name as seen by the OS varies: "AE-30", "Aerophone Pro", "2- AE-30", "MIDIIN2 (AE-30)".

---

## 8. Differences AE-30 vs AE-20

- Identical model ID and address map.
- AE-30-only blocks: Thumb Pad assign/MIDI-OUT, Motion Controller assign/MIDI-OUT, Rx ThumbPad,
  Rx Motion, Motion settings in System and Scene Common.
- Identity Reply family number: `00` = AE-30, `01` = AE-20.
- AE-30 firmware is at 3.20 (editor app 1.5.4); the implementation doc is v2.01 (2022), so a few
  newer parameters may be absent from the map.

---

## 9. Proposed Rust architecture

Reuse from EWI editor: `midir` port enumeration, SysEx send/receive plumbing, UI shell.

New modules:

```
ae30/
├── sysex.rs      — RQ1/DT1 builders, checksum, nibble pack/unpack, 7-bit address math,
│                   response matcher (pair RQ1 → DT1 by address), chunking + pacing
├── addrmap.rs    — static tables: block base/size, per-param offset/bits/range/labels
│                   (generate from the PDF like companion's tools/build_data.py, or hand-port)
├── scene.rs      — Scene / Part / Tone structs backed by byte buffers; typed get/set
├── device.rs     — identify(), read_scene(temp|user n), write_param(live), save_scene(n),
│                   list_scene_names(), list_user_tones(), backup_all()
└── ui/           — scene browser, part strip, tone editor
```

Editing model: edits write **single-param DT1s to the Temp Scene** so you hear changes instantly;
"Save" = one `0F 00 10 01` command. No "send whole patch" step as with the EWI.

### First milestones

1. **Probe:** Identity Request → confirm `7D 03 00` reply. Read `00 10 00 00` (Setup, 0x7A bytes)
   → decode current scene MSB/LSB/PC.
2. **Read Temp Scene Common** (`01 00 00 00`, 0x5A) → print the 16-char name. Read Part 1
   (`01 00 30 00`, 0x65) → bank/PC/level.
3. **Live edit:** DT1 one byte to `01 00 30 05` (Part 1 level) → hear it.
4. **Save:** write to an empty user slot via `0F 00 10 01`, power-cycle, confirm it stuck.
5. **Name lists** via `0F 00 41 00`; then full scene browser.
6. Tone editing + user tone save.

Back up all 600 user scenes (read-only RQ1 sweep) before any write testing.

---

## 10. Sources

- Roland, *AE-30/AE-20 MIDI Implementation* v2.01 (Jan 2022):
  https://files.kraftmusic.com/media/ownersmanual/Roland_Aerophone_AE-20_MIDI_Implementation.pdf
- Roland AE-30 Owner's Manual: https://static.roland.com/assets/media/pdf/AE-30_eng01_W.pdf
- Roland AE-30 support page (Parameter Guide, Scene List PDFs):
  https://www.roland.com/us/support/by_product/aerophone_pro/owners_manuals/
- `aerophoneCompanion` (MIT, AE-20, browser/Web MIDI) — reference for the undocumented commands
  and pacing: https://github.com/wrongwaycorrigan/aerophoneCompanion
- Roland Aerophone Pro Editor (iOS/Android), v1.5.4 / firmware 3.20:
  https://apps.apple.com/us/app/aerophone-pro-editor/id1524416526

---

## 11. Verified on a real AE-30 (2026-11-25)

- Identity reply family `7D 03 00` (AE-30); the reply carries device ID **7F**, and RQ1/DT1 with 7F work.
- Per-block RQ1 reads work for every scene, tone and System block (sizes must match exactly).
- **MIDI Control Mode blocks are 45 bytes**: an RQ1 for the documented `2E` is ignored.
- **One RQ1 covering a whole area streams all of its blocks**: Temporary Scene (3,317 bytes, 56 DT1s) in
  ~0.18 s, Temporary Tone (2,120 bytes) in ~0.14 s. The System area stops partway, so read missing System
  blocks individually. Block-by-block reading of a scene takes ~2.4 s with one round trip per block.
- **Do not stream stored (user/preset) scenes.** For those, the RQ1 size is not an address range: the scene's
  span (`3A10`) streams ~4.5 scenes; its byte count (3,317) stops at Part 5; a huge size streams the whole
  user area at ~1.3–1.7 s per scene in bursts with ~0.4–1 s pauses, and **the unit ignores every other
  request (even Identity) until it's done** — power-cycling stops it. Read stored scenes with exact
  whole-block RQ1s; they can be sent back to back (the unit answers in order). Requests that start inside
  a block of a stored scene (e.g. the second 128-byte chunk of an MFX block) get no answer.
- Scene name list (`0F 00 41 00`) works: 12 user banks in ~2.8 s (each bank is exactly 50 entries),
  presets (394 scenes) in ~9 s. The unit doesn't always send the all-zero terminator, so finish on count or idle gap.
- DT1 to the Temporary Scene takes effect and reads back (Part 1 level 100 → 99 → 100).
- **Scene write `0F 00 10 01` works on the AE-30** (tested from the editor's Save dialog, 2026-11-25).
- Not yet tested: tone write (`0F 00 10 05+`) and system write (`0F 00 10 00`).
- The unit sends pitch bend (from bite/wheel), CC2/CC11 breath and notes on USB while played; the editor's
  status bar shows breath and bend from these.
