# Changelog

## Unreleased

### Added

- **Fold away the side panels on the Live tab:** the « and » buttons hide the scene list and the setlist, so the effect knobs get the whole window. A thin strip at the edge opens them again, and the app remembers your choice. Page-turner pedals keep stepping through the setlist while it's hidden.

## v0.5.0 — 2026-11-26

The editor is now called Aerophone Editor, the Mac version is signed and notarized by Apple, and a demo mode lets you try it without an AE-30.

### Added

- **Demo mode:** try every screen without an AE-30. Click **Try Demo** on the connection screen (or choose Demo Mode from the menu) to work with a simulated AE-30 that has a few demo scenes. Nothing is sent over MIDI, and your setlists aren't touched.
- **Opens where you left off:** the app starts on the tab you used last (Live the first time).

### Changed

- **New license:** the source is now under the PolyForm Strict License 1.0.0 (read and use noncommercially; no changes or redistribution). v0.4.0 and earlier stay under the GPL v3.
- **Bigger knobs on the Live tab:** the four effect cards grow with the window and switch to a 2×2 grid on tall screens.
- **Notices a disconnected AE-30:** when the USB cable is pulled or Bluetooth drops, the app now shows the connection screen and reconnects by itself when the AE-30 is back. Before, it kept showing the old connection.
- **New name: Aerophone Editor.** The app, its files (`AerophoneEditor.exe`, `AerophoneEditor-<version>.dmg`), the command-line tool (`aerophoneeditor`) and the GitHub repository (`ericdano/AerophoneEditor`; the old address redirects) are renamed. The app ID is now `com.ericdannewitz.aerophoneeditor`. Your settings, setlists and preset name cache move to the new settings folder automatically on first launch.

### Installing

- **macOS:** open the `.dmg` and drag **Aerophone Editor** to Applications. The app is signed and notarized by Apple, so it opens without warnings. You can delete the old "AE-30 Patch Editor" app.
- **Windows:** unzip and run `AerophoneEditor.exe`. If SmartScreen appears, choose **More info ▸ Run anyway**.
- Your settings, setlists and preset name cache move over automatically on first launch.

## v0.4.0 — 2026-11-26

The Live tab now keeps several named setlists, and you can remove entries in bulk.

### Added

- **Setlists:** keep several named playlists on the Live tab. Create, rename and delete them from the ⋯ menu, and switch with the setlist menu. Reserved user scenes are shared between setlists, so no two entries use the same one. Your existing playlist becomes "Setlist 1".
- **Select mode:** tick entries to remove several at once, or clear a whole setlist (with a confirmation). Removed entries free their reserved user scenes for reuse. The scenes on the AE-30 aren't changed.

### Installing

- **macOS:** open the `.dmg` and drag the app to Applications, replacing the old version. The app isn't notarized yet, so the first time, Control-click it and choose **Open** (or go to System Settings ▸ Privacy & Security ▸ **Open Anyway**).
- **Windows:** unzip and run `AE30PatchEditor.exe`. If SmartScreen appears, choose **More info ▸ Run anyway**.
- Your settings and playlist carry over.

## v0.3.0 — 2026-11-26

This release adds a Live screen for playing on stage, with a playlist of your scenes.

### Added

- **Live tab** (the first tab, ⌘1/Ctrl+1): the scene's name in large type, big knobs for Reverb, Chorus, Delay and Color (the insert effect) with a type menu under each, and sliders for Level and Dark/Bright. Picking an effect type switches it on, and Color routes the active parts through the insert effect. The Color knob sets the setting that means "how much" for its type (Drive, Balance, Depth and so on), and a note appears when an effect is on but no part sends to it.
- **Playlist:** add scenes for a gig, then switch with a click, Previous/Next, the arrow keys or Page Up/Down (which most page-turner pedals send). Each entry keeps its own copy of the scene, including tweaks made while it plays. Unchanged entries select their original scene. An edited entry is stored in an empty user scene when you next pick it, counting down from U12-50 and using only slots named INIT SCENE, so later picks are instant. The playlist is saved between sessions.
- `ae30edit playlist-test FILE.syx`: checks the playlist on the AE-30 using U12-50 (only if it's empty), then restores it.

### Verified on the AE-30

- Storing a scene and restoring a backup work over Bluetooth MIDI on macOS.

### Installing

- **macOS:** open the `.dmg` and drag the app to Applications, replacing the old version. The app isn't notarized yet, so the first time, Control-click it and choose **Open** (or go to System Settings ▸ Privacy & Security ▸ **Open Anyway**).
- **Windows:** unzip and run `AE30PatchEditor.exe`. If SmartScreen appears, choose **More info ▸ Run anyway**.
- Your settings carry over.

## v0.2.1 — 2026-11-26

The AE-30 Patch Editor now has its own app icon. Everything else is as in v0.2.0; see below for its features.

### Added

- **App icon:** used for the window, the Dock, the About box, the macOS app and the Windows `.exe` (which now also shows its version and description in Explorer). `rust/scripts/make-icons.py` makes the icon files from `ae30-app/assets/icons/icon-source.png`.

### Installing

- **macOS:** open the `.dmg` and drag the app to Applications, replacing the old version. The app isn't notarized yet, so the first time, Control-click it and choose **Open** (or go to System Settings ▸ Privacy & Security ▸ **Open Anyway**). If the Dock still shows a blank icon, restart the Dock or log out and back in.
- **Windows:** unzip and run `AE30PatchEditor.exe`. If SmartScreen appears, choose **More info ▸ Run anyway**.
- Your settings carry over.

## v0.2.0 — 2026-11-26

This release adds restoring backups to the AE-30, and confirms on a real AE-30 that every store command works.

### Added

- **Restore Scenes:** write the scenes from a `.syx` backup (or single scene files) back to the AE-30, either to their own slots or one after another from a chosen slot. Choose which scenes to include; any named scene about to be replaced is flagged first. Each scene is read back and checked byte for byte. Values the AE-30 changes while saving are written back directly, and settings it manages itself (unused effects) are recognised. About 3 seconds per scene, with progress and Cancel in the status bar. Available from File ▸ Restore Scenes…, the ☰ menu, the library's Restore… button and `ae30edit restore`.

### Verified on the AE-30

- Saving a part's tone to a user tone slot, and storing System settings, both survive a power cycle.

### Installing

- **macOS:** open the `.dmg` and drag the app to Applications, replacing the old version. The app isn't notarized yet, so the first time, Control-click it and choose **Open** (or go to System Settings ▸ Privacy & Security ▸ **Open Anyway**).
- **Windows:** unzip and run `AE30PatchEditor.exe`. If SmartScreen appears, choose **More info ▸ Run anyway**.
- Your settings and cached preset names carry over from v0.1.0.

### Known limitations

- The Windows build hasn't been tested on a Windows PC yet.
- SuperNATURAL Acoustic tones and drum kits can be chosen but not edited inside, and Part 5's tone can't be edited.
- Backups and restore cover user scenes, not user tones. A restored scene that uses a user tone plays whatever tone is in that slot now.

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
- Restoring a backup to the AE-30 isn't built yet; backups can be inspected and individual scenes imported. (Added since: see Unreleased.)
- The user-tone and System store commands are reverse-engineered and less tested than scene saving, which has been tested on an AE-30. (Both verified since.)

### Credits

- Parameter data from [aerophoneCompanion](https://github.com/wrongwaycorrigan/aerophoneCompanion) (MIT), built from Roland's published documents.
- Inter font, under the SIL Open Font License.
- Not affiliated with Roland.

Copyright © 2027 Eric Dannewitz · [Buy me a coffee](https://buymeacoffee.com/ericdannewitz)
