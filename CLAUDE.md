# AE30PatchEditor

Rust GUI patch/scene editor for the Roland Aerophone Pro AE-30 over MIDI SysEx (Roland RQ1/DT1).
Protocol research: `AE30_SYSEX_NOTES.md`. Cargo workspace in `rust/`.

## Commands (run in `rust/`)

- `cargo run --bin ae30edit` - the editor (connects to the AE-30 automatically)
- `cargo test` / `cargo fmt` / `cargo clippy --all-targets` - keep all clean
- CLI tools (`ae30edit help`): `ports`, `probe`, `scene`, `names [user|preset]`, `read AA BB CC DD SIZE` (hex, plain size), `backup FILE.syx [FIRST LAST]`, `restore FILE.syx [SLOT] [--yes]` (writes! dry run without `--yes`), `tone-names`, `system-set LABEL VALUE` (stores System!), `stream-test`, `editor-test` (drives the GUI model headless), `edit-test` (Temp Scene write/read-back/restore), `playlist-test FILE.syx` (Live playlist on the unit: backs up U12-50 if empty, stores/selects an edited entry there, restores it). Experiments: `poke-temp BB CC VAL`, `poke-user SLOT BB CC VAL` (writes a stored scene!), `listen` (is the unit streaming?).
- Test writes only on empty slots (check `names user`; U03-01 is the user's), back them up first, put them back after.
- `AE30_SCREENSHOT=out.png [AE30_TAB=scene|part|effects|controllers|tone|system|live] [AE30_PICKER=1] ae30edit` - open, wait until loaded, save a PNG of the window, quit.
- `scripts/build-macos.sh` - universal `target/macos/AE-30 Patch Editor.app` + `.dmg` (ad-hoc signed unless a Developer ID is in the keychain; `logo.icns` used if added to `ae30-app/assets/icons/`)
- `scripts/build-windows.sh` - cross-compile `target/windows/AE30PatchEditor-<version>-windows-x64.zip` (needs `x86_64-pc-windows-gnu` + `brew install mingw-w64`; linker in `rust/.cargo/config.toml`). Untested on a real Windows PC so far.
- `python3 scripts/make-icons.py` - regenerate `ae30-app/assets/icons/logo.png`/`.icns`/`.ico` from `icon-source.png` (needs Pillow; `ae30-app/build.rs` embeds the `.ico` and version info in Windows builds only)
- `python3 tools/gen_data.py <aerophoneCompanion>/app/data.js` - regenerate `ae30-core/data/ae30.json`.

## Layout

- `ae30-core` (no MIDI, unit-tested): `sysex` (framing, checksum, `Addr` 7-bit math, nibbles, identity), `params` (loads `data/ae30.json`: every block's params, encoding, ranges, labels, options, display `Format`, effect-type-specific params), `image` (`Area` + `Image`: local copy of an area as named blocks; DT1 data placed by address), `session` (I/O-free state machine: paced output, merged edits, one job in flight, streamed reads, scene/tone name lists, write commands), `syx` (scene files and backups), plus older `map`/`read`/`scene` helpers.
- `ae30-app`: `midi` (midir ports, `Link::pump/run`), `editor` (GUI-free model: connection, loading, live edits + undo, follows panel changes, save/import/export/backup), `cli`, `ui/` (egui 0.37: `mod.rs` window, library, dialogs, tone picker; `pages.rs` Scene/Parts/Effects/Controllers/Tone/System; `widgets.rs` param controls, knob, fader, draggable envelope, assign curve; `theme.rs`).
- Parameter data comes from aerophoneCompanion (MIT), which parsed Roland's PDFs. Credit it if distributing.

## Gotchas

- Raw vs value: `Param::get` returns `raw - vofs`; `encode` adds it back. Option lists index by value.
- Bluetooth MIDI (macOS, via Audio MIDI Setup's Bluetooth panel) shows up as port "AE-30 Bluetooth"; read, stream, live edit, backup and GUI verified 2026-11-26 with the existing pacing. Scene write + restore of one slot also verified over Bluetooth (`playlist-test`).
- The unit answers with device ID 7F (its identity reply), and the session adopts it.
- Perf_midi_ctrl blocks are 45 bytes; RQ1 with Roland's documented 0x2E gets no reply. RQ1s must be exact whole blocks (`Image::missing_reads`).
- `Session::read_area` (one RQ1 for a whole area) is only for the temporary scene/tones. Never use it, or `read_raw` with a large size, on stored scenes: the unit streams on through following scenes and ignores all other requests until finished (see notes §11). Stored scenes use `Session::read_blocks` (pipelined whole-block RQ1s); the backup does this.
- Live tab (`pages::live`, playlist panel in `ui/mod.rs`, model in `editor/playlist.rs`): knobs set Reverb/Chorus/Delay Level and a type-specific IFX amount ("Color": Drive > Balance > Depth > … > Level); picking a type switches the effect on, and for Color routes the parts that are on to the IFX. Tone slider moves every active part's Cutoff Offset by the same amount. Playlist entries hold full scene copies (saved in the OS config dir `playlist/`): unchanged → select the source scene; edited → stage in the Temporary Scene, scene-write to a reserved slot, select it with a non-merged Setup DT1 (a merged one could jump ahead of the store). Slots come from the top down, only "INIT SCENE" ones, and are reused only while they still hold the name we stored. Edits are copied into the active entry every second and before a scene reload; `playlist_loaded` drops the active entry when another scene is picked. Verified on the AE-30 over Bluetooth 2026-11-26.
- System area streams only partly; the editor falls back to per-block reads for missing blocks.
- Writes to user scenes/tones/System use reverse-engineered `0F 00 10 0x` commands (`session::cmd`), all verified on the AE-30. Edits otherwise only touch the Temporary Scene/Tone.
- Restore (`editor.rs` `Restore`): per scene, stage in the Temporary Scene, scene-write, read back with `read_blocks`, patch real differences by DT1 straight into the stored scene, re-check. `differences`/`invalid_in_file` ignore what the unit sets itself (unused-effect params, out-of-range values). Scene reloads are suppressed while restoring; afterwards the unit returns to the previous scene.
- Preset scene names are cached in the OS cache dir (`preset-scenes.txt`); user bank lists finish at 50 entries.
- Platforms: macOS gets a native menu bar (`ui/native_menu.rs`, muda) and a unified title bar (fullsize content view; top bar leaves 70 pt for the window buttons and drags the window). Windows/Linux get the ☰ in-window menu. Both menus run `ui/commands.rs` `Command`s; keep them in step. Settings/Preferences (`ui/prefs.rs`, window in `ui/mod.rs`): theme, accent, corners, interface size (egui zoom factor), slider fill, knob speed, hide empty scenes; saved as key=value in the OS config dir `prefs.txt`. `AE30_DIALOG=prefs|about` opens one for screenshots; `AE30_RESTORE=file.syx` opens the Restore window.
- egui: put card contents in `ui.vertical` when the card sits in a horizontal layout; grids need unique ids (`widgets::grid` salts by block + first param).
