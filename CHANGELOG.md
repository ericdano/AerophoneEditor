# Changelog

## v0.1.0 — 2026-11-25

First release of the AE-30 Patch Editor, a scene and tone editor for the Roland Aerophone Pro AE-30. Changes are sent to the instrument as you make them, so you hear each edit immediately. Nothing is stored on the AE-30 until you save.

### Editing

- **Scene:** a five-part mixer with part on/off, Lead/Drone, tone, level, pan, octave and reverb/chorus/delay sends, plus tempo, transpose, volume, bend and bite ranges, control sources and harmony.
- **Parts:** every part setting, including tuning, sound offsets (cutoff, resonance, envelope, vibrato), mono/poly, legato, portamento, unison, key and velocity ranges, X-Fade, scale tune, receive switches and part EQ.
- **Effects:** a signal-flow overview, and the controls for each type of the part MFX, IFX, chorus, delay and reverb.
- **Controllers:** breath, bite, S1/S2, side keys, wheel, thumb pad and motion assignments, for both the internal sound and MIDI Out, each with its response curve. The breath curve shows a live dot while you play.
- **Tone:** a ZEN-Core tone editor covering oscillator (PCM/VA/SuperSAW), pitch, filter, amp, envelopes you edit by dragging, two LFOs with step sequencer, matrix control, partial EQ, unison and structure, and the tone's own effect.
- **System:** the AE-30's global settings, with a button to store them.
- Undo and redo for every edit.
- A tone picker covering Preset A/B/C, SuperNATURAL, drum kits and user tones. Clicking a tone plays it straight away.

### Library

- Searchable lists of the 600 user and 480 preset scenes; clicking one switches the AE-30 to it.
- Save the current scene to any user slot, or a part's tone to any of the 512 user tone slots.
- Import and export single scenes as `.syx` files.
- Back up your named user scenes, or all 600, to one `.syx` file, at about 1.3 seconds per scene.

### Live

- Breath and pitch bend meters and the playing note in the status bar.
- Changes made on the AE-30's own panel appear in the editor.
- The editor finds the AE-30 by itself and reconnects after unplugging or power-cycling.

### Mac and Windows

- On macOS: a native menu bar with standard shortcuts, a unified title bar, and a universal app for Apple Silicon and Intel.
- On Windows 10/11: the same commands in a ☰ menu.
- Preferences: theme (match system, light or dark), accent colour, corner roundness, interface size, slider style, knob speed, and hiding empty scene slots.

### Command line

`ae30edit help` lists the tools for checking the connection, reading memory and backing up without the window.

### Installing

- **macOS:** open the `.dmg` and drag the app to Applications. The app isn't notarized yet, so the first time, Control-click it and choose **Open** (or go to System Settings ▸ Privacy & Security ▸ **Open Anyway**).
- **Windows:** unzip and run `AE30PatchEditor.exe`. If SmartScreen appears, choose **More info ▸ Run anyway**. If Windows doesn't see the AE-30, install Roland's USB driver or set the AE-30's USB driver mode to Generic.
- Close Roland's Aerophone app while using the editor, so the two don't conflict.

### Known limitations

- The Windows build hasn't been tested on a Windows PC yet.
- SuperNATURAL Acoustic tones and drum kits can be chosen but not edited inside. Roland doesn't publish their parameters, and its own app has the same limit.
- Part 5's tone can't be edited (the AE-30 has temporary tones for parts 1–4 only).
- Restoring a backup to the AE-30 isn't built yet; backups can be inspected and individual scenes imported.
- The user-tone and System store commands are reverse-engineered and less tested than scene saving, which has been tested on an AE-30.

### Credits

- Parameter data from [aerophoneCompanion](https://github.com/wrongwaycorrigan/aerophoneCompanion) (MIT), built from Roland's published documents.
- Inter font, under the SIL Open Font License.
- Not affiliated with Roland.

Copyright © 2027 Eric Dannewitz · [Buy me a coffee](https://buymeacoffee.com/ericdannewitz)
