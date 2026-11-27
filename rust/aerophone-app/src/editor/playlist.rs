//! The Live playlist: scenes the app keeps for a performance.
//!
//! Each entry holds a full copy of its scene. Picking an entry:
//! - unchanged: selects the scene it came from (instant);
//! - changed and already stored: selects its reserved user scene (instant);
//! - changed since it was last stored: sends the copy to the Temporary Scene,
//!   stores it in a reserved user scene and selects that.
//!
//! Reserved scenes are taken from the top of the user area (U12-50 down),
//! only from slots named "INIT SCENE", and only reused while they still
//! hold the name we stored there, so the user's own scenes are never
//! overwritten. Edits made while an entry plays are copied back into it
//! (nothing is stored until the entry is picked again).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use aerophone_core::image::{Area, Image};
use aerophone_core::scene::SceneRef;
use aerophone_core::syx;

use super::{Editor, SETTLE, TAG_SCENE};

/// A user scene slot nobody uses.
const EMPTY: &str = "INIT SCENE";
/// How often edits to the playing entry are copied into it.
const CAPTURE_EVERY: Duration = Duration::from_millis(1000);

#[derive(Debug, Clone)]
pub struct Entry {
    /// The scene's settings (a Temporary Scene image).
    pub scene: Image,
    /// Bank select of the scene it came from.
    pub source: Option<(u8, u8, u8)>,
    /// Its reserved user scene (0..600) and the name stored there.
    pub slot: Option<(u16, String)>,
    /// Differs from `source`.
    pub changed: bool,
    /// `slot` holds exactly `scene`.
    pub stored: bool,
}

impl Entry {
    pub fn name(&self) -> String {
        self.scene.scene_name().unwrap_or_default()
    }

    pub fn source_ref(&self) -> Option<SceneRef> {
        self.source.map(|(m, l, p)| SceneRef::from_bank(m, l, p))
    }
}

/// A named list of entries.
#[derive(Debug, Clone, Default)]
pub struct Setlist {
    pub name: String,
    pub entries: Vec<Entry>,
}

#[derive(Debug)]
pub struct Playlist {
    /// Always at least one.
    pub lists: Vec<Setlist>,
    /// The setlist shown.
    pub current: usize,
    /// The entry of the current setlist playing now.
    pub active: Option<usize>,
    /// The scene the active entry plays from.
    at: Option<SceneRef>,
    /// Reserved slots no entry uses any more: (slot, name stored there).
    spare: Vec<(u16, String)>,
    last_capture: Option<Instant>,
    /// Where it's saved; None keeps it in memory only (tests).
    dir: Option<PathBuf>,
}

impl Default for Playlist {
    fn default() -> Self {
        Playlist {
            lists: vec![Setlist {
                name: "Setlist 1".into(),
                entries: Vec::new(),
            }],
            current: 0,
            active: None,
            at: None,
            spare: Vec::new(),
            last_capture: None,
            dir: None,
        }
    }
}

fn slot_label(n: u16) -> String {
    format!("U{:02}-{:02}", n / 50 + 1, n % 50 + 1)
}

fn same_data(a: &Image, b: &Image) -> bool {
    a.blocks.len() == b.blocks.len()
        && a.blocks
            .iter()
            .zip(&b.blocks)
            .all(|(x, y)| x.is_loaded() == y.is_loaded() && x.data == y.data)
}

impl Playlist {
    /// The saved playlist from the OS config folder.
    pub fn load() -> Self {
        let dir = crate::paths::dirs().map(|d| d.config_dir().join("playlist"));
        let mut p = Playlist {
            dir: dir.clone(),
            ..Playlist::default()
        };
        let Some(dir) = dir else { return p };
        let Ok(text) = std::fs::read_to_string(dir.join("playlist.txt")) else {
            return p;
        };
        let scenes = |file: &str| {
            let bytes = std::fs::read(dir.join(file)).ok()?;
            syx::scene_from_syx(&bytes).filter(|s| s.is_loaded())
        };
        p.parse(&text, scenes);
        p
    }

    /// The entries of the setlist shown.
    pub fn entries(&self) -> &[Entry] {
        &self.lists[self.current].entries
    }

    pub fn entries_mut(&mut self) -> &mut Vec<Entry> {
        &mut self.lists[self.current].entries
    }

    pub fn list_name(&self) -> &str {
        &self.lists[self.current].name
    }

    /// Read `playlist.txt`; `scene` loads an entry's scene file. Files from
    /// before setlists (no `list` lines) become one setlist.
    fn parse(&mut self, text: &str, scene: impl Fn(&str) -> Option<Image>) {
        let mut lists: Vec<Setlist> = Vec::new();
        let mut current = 0;
        for line in text.lines() {
            let f: Vec<&str> = line.split('\t').collect();
            match f.as_slice() {
                ["list", name] => lists.push(Setlist {
                    name: name.to_string(),
                    entries: Vec::new(),
                }),
                ["current", n] => current = n.parse().unwrap_or(0),
                ["entry", file, source, slot, slot_name, changed, stored] => {
                    let Some(img) = scene(file) else { continue };
                    let source = {
                        let b: Vec<u8> = source.split(',').filter_map(|x| x.parse().ok()).collect();
                        (b.len() == 3).then(|| (b[0], b[1], b[2]))
                    };
                    let slot = slot
                        .parse::<u16>()
                        .ok()
                        .filter(|&n| n < 600)
                        .map(|n| (n, slot_name.to_string()));
                    if lists.is_empty() {
                        lists.push(Setlist {
                            name: "Setlist 1".into(),
                            entries: Vec::new(),
                        });
                    }
                    lists.last_mut().unwrap().entries.push(Entry {
                        scene: img,
                        source,
                        changed: *changed == "1" || source.is_none(),
                        stored: *stored == "1" && slot.is_some(),
                        slot,
                    });
                }
                ["spare", slot, name] => {
                    if let Ok(n) = slot.parse::<u16>() {
                        if n < 600 {
                            self.spare.push((n, name.to_string()));
                        }
                    }
                }
                _ => {}
            }
        }
        if !lists.is_empty() {
            self.current = current.min(lists.len() - 1);
            self.lists = lists;
        }
    }

    fn file_name(list: usize, i: usize) -> String {
        format!("{:02}-{:03}.syx", list + 1, i + 1)
    }

    fn to_text(&self) -> String {
        let mut s = String::from("# Aerophone Editor setlists\n");
        for (l, list) in self.lists.iter().enumerate() {
            s += &format!("list\t{}\n", list.name.replace(['\t', '\n'], " "));
            for (i, e) in list.entries.iter().enumerate() {
                let source = e
                    .source
                    .map_or("-".to_string(), |(m, l, p)| format!("{m},{l},{p}"));
                let (slot, name) = e
                    .slot
                    .as_ref()
                    .map_or(("-".to_string(), ""), |(n, s)| (n.to_string(), s.as_str()));
                s += &format!(
                    "entry\t{}\t{source}\t{slot}\t{name}\t{}\t{}\n",
                    Self::file_name(l, i),
                    u8::from(e.changed),
                    u8::from(e.stored)
                );
            }
        }
        s += &format!("current\t{}\n", self.current);
        for (n, name) in &self.spare {
            s += &format!("spare\t{n}\t{name}\n");
        }
        s
    }

    /// Write it to disk. Returns an error message on failure.
    fn save(&self, device: u8) -> Result<(), String> {
        let Some(dir) = &self.dir else { return Ok(()) };
        let fail = |e: std::io::Error| format!("Couldn't save the setlists: {e}");
        std::fs::create_dir_all(dir).map_err(fail)?;
        let mut written = std::collections::HashSet::new();
        for (l, list) in self.lists.iter().enumerate() {
            for (i, e) in list.entries.iter().enumerate() {
                let f = Self::file_name(l, i);
                std::fs::write(dir.join(&f), syx::scene_to_syx(&e.scene, device)).map_err(fail)?;
                written.insert(f);
            }
        }
        std::fs::write(dir.join("playlist.txt"), self.to_text()).map_err(fail)?;
        // Scene files of entries since removed (and older names).
        if let Ok(rd) = std::fs::read_dir(dir) {
            for f in rd.flatten() {
                let name = f.file_name().to_string_lossy().to_string();
                if name.ends_with(".syx") && !written.contains(&name) {
                    let _ = std::fs::remove_file(f.path());
                }
            }
        }
        Ok(())
    }

    /// Slots held by entries of any setlist, or kept spare.
    fn reserved(&self) -> impl Iterator<Item = u16> + '_ {
        self.lists
            .iter()
            .flat_map(|l| &l.entries)
            .filter_map(|e| e.slot.as_ref().map(|s| s.0))
            .chain(self.spare.iter().map(|s| s.0))
    }
}

impl Editor {
    fn save_playlist(&mut self) {
        if let Err(e) = self.playlist.save(self.session.device) {
            self.warn(e);
        }
    }

    /// Does user scene `n` still hold what we stored there as `name`?
    fn slot_is_ours(&self, n: u16, name: &str) -> bool {
        self.user_names
            .get(usize::from(n))
            .is_some_and(|s| !s.is_empty() && s.trim_end() == name.trim_end())
    }

    /// A user scene entry `i` may store into: its own, a spare one, or the
    /// highest empty one.
    fn playlist_slot(&mut self, i: usize) -> Option<u16> {
        if let Some((n, name)) = self.playlist.entries()[i].slot.clone() {
            if self.slot_is_ours(n, &name) {
                return Some(n);
            }
            // Someone stored something else there; let it be.
            self.playlist.entries_mut()[i].slot = None;
        }
        // Spare slots overwritten by someone else are dropped.
        let spare = std::mem::take(&mut self.playlist.spare);
        let mut ours = spare
            .into_iter()
            .filter(|(n, name)| self.slot_is_ours(*n, name))
            .collect::<Vec<_>>()
            .into_iter();
        if let Some((n, _)) = ours.next() {
            self.playlist.spare = ours.collect();
            return Some(n);
        }
        let reserved: Vec<u16> = self.playlist.reserved().collect();
        (0..600u16)
            .rev()
            .find(|&n| self.user_names[usize::from(n)] == EMPTY && !reserved.contains(&n))
    }

    /// Add the scene playing now to the end of the playlist and make it the
    /// active entry.
    pub fn playlist_add(&mut self) {
        if !self.scene.is_loaded() {
            return self.warn("The scene hasn't finished loading.");
        }
        let source = self
            .setup
            .blocks
            .first()
            .filter(|b| b.is_loaded())
            .map(|b| (b.data[0], b.data[1], b.data[2]));
        let entry = Entry {
            scene: self.scene.clone(),
            source,
            slot: None,
            changed: self.dirty || source.is_none(),
            stored: false,
        };
        let name = entry.name();
        self.playlist.entries_mut().push(entry);
        self.playlist.active = Some(self.playlist.entries().len() - 1);
        self.playlist.at = self.current_scene();
        self.save_playlist();
        self.tell(format!("Added \"{name}\" to the playlist"));
    }

    pub fn playlist_remove(&mut self, i: usize) {
        self.playlist_remove_many(&[i]);
    }

    /// Remove entries of the current setlist; their reserved user scenes
    /// become free for other entries.
    pub fn playlist_remove_many(&mut self, which: &[usize]) {
        let mut which: Vec<usize> = which
            .iter()
            .copied()
            .filter(|&i| i < self.playlist.entries().len())
            .collect();
        which.sort_unstable();
        which.dedup();
        for &i in which.iter().rev() {
            let e = self.playlist.entries_mut().remove(i);
            if let Some(slot) = e.slot {
                self.playlist.spare.push(slot);
            }
            self.playlist.active = match self.playlist.active {
                Some(a) if a == i => None,
                Some(a) if a > i => Some(a - 1),
                a => a,
            };
        }
        if !which.is_empty() {
            self.save_playlist();
        }
    }

    /// Empty the current setlist.
    pub fn playlist_clear(&mut self) {
        let all: Vec<usize> = (0..self.playlist.entries().len()).collect();
        self.playlist_remove_many(&all);
    }

    /// Show setlist `l` (keeping edits of the entry playing now; the
    /// scene keeps playing, but no entry is active).
    pub fn setlist_select(&mut self, l: usize) {
        if l >= self.playlist.lists.len() || l == self.playlist.current {
            return;
        }
        self.playlist_capture(false);
        self.playlist.current = l;
        self.playlist.active = None;
        self.save_playlist();
    }

    /// Add an empty setlist and show it.
    pub fn setlist_new(&mut self, name: &str) {
        let name = match name.trim() {
            "" => format!("Setlist {}", self.playlist.lists.len() + 1),
            n => n.to_string(),
        };
        self.playlist_capture(false);
        self.playlist.lists.push(Setlist {
            name,
            entries: Vec::new(),
        });
        self.playlist.current = self.playlist.lists.len() - 1;
        self.playlist.active = None;
        self.save_playlist();
    }

    pub fn setlist_rename(&mut self, l: usize, name: &str) {
        let name = name.trim();
        if let (Some(list), false) = (self.playlist.lists.get_mut(l), name.is_empty()) {
            list.name = name.to_string();
            self.save_playlist();
        }
    }

    /// Delete setlist `l` and its entries (there's always one left).
    pub fn setlist_delete(&mut self, l: usize) {
        if l >= self.playlist.lists.len() {
            return;
        }
        let list = self.playlist.lists.remove(l);
        self.playlist
            .spare
            .extend(list.entries.into_iter().filter_map(|e| e.slot));
        if self.playlist.lists.is_empty() {
            self.playlist.lists.push(Setlist {
                name: "Setlist 1".into(),
                entries: Vec::new(),
            });
        }
        if self.playlist.current == l {
            self.playlist.active = None;
        }
        if self.playlist.current >= l && self.playlist.current > 0 {
            self.playlist.current -= 1;
        }
        self.tell(format!("Deleted setlist \"{}\"", list.name));
        self.save_playlist();
    }

    /// Move entry `i` up (`-1`) or down (`1`).
    pub fn playlist_move(&mut self, i: usize, dir: i32) {
        let j = i as i32 + dir;
        let len = self.playlist.entries().len();
        if i >= len || j < 0 || j as usize >= len {
            return;
        }
        let j = j as usize;
        self.playlist.entries_mut().swap(i, j);
        self.playlist.active = match self.playlist.active {
            Some(a) if a == i => Some(j),
            Some(a) if a == j => Some(i),
            a => a,
        };
        self.save_playlist();
    }

    /// Forget an entry's changes: it plays its original scene again.
    pub fn playlist_revert(&mut self, i: usize) {
        let Some(e) = self.playlist.entries_mut().get_mut(i) else {
            return;
        };
        if e.source.is_none() {
            return;
        }
        e.changed = false;
        e.stored = false;
        let playing = self.playlist.active == Some(i);
        self.save_playlist();
        if playing {
            self.dirty = false;
            self.playlist.active = None;
            self.playlist_play(i);
        }
    }

    /// Step through the playlist by `dir` from the active entry.
    pub fn playlist_step(&mut self, dir: i32) {
        let len = self.playlist.entries().len() as i32;
        if len == 0 {
            return;
        }
        let i = match self.playlist.active {
            Some(a) => (a as i32 + dir).clamp(0, len - 1),
            None if dir < 0 => len - 1,
            None => 0,
        } as usize;
        if Some(i) != self.playlist.active {
            self.playlist_play(i);
        }
    }

    /// Play entry `i` (see the module notes).
    pub fn playlist_play(&mut self, i: usize) {
        if i >= self.playlist.entries().len() {
            return;
        }
        if self.restore.is_some() || self.backup.is_some() {
            return self.warn("Wait for the backup or restore to finish first.");
        }
        // Keep the edits made to the entry playing now.
        self.playlist_capture(false);
        self.dirty = false;
        self.playlist.active = Some(i);
        let e = self.playlist.entries()[i].clone();
        let name = e.name();
        if !e.changed {
            if let Some((m, l, p)) = e.source {
                self.playlist.at = Some(SceneRef::from_bank(m, l, p));
                self.select_scene(m, l, p);
                return;
            }
        }
        if let (true, Some((n, stored_as))) = (e.stored, &e.slot) {
            if self.slot_is_ours(*n, stored_as) {
                self.playlist.at = Some(SceneRef::User(n + 1));
                self.select_user(*n);
                return;
            }
        }
        // Send the copy to the Temporary Scene; it plays once it's there.
        self.scene_reload = None;
        self.session.cancel_reads(|t| t == TAG_SCENE);
        self.pending.remove(&TAG_SCENE);
        for b in e.scene.blocks.iter().filter(|b| b.is_loaded()) {
            self.scene.apply(b.addr, &b.data);
            self.session.write(b.addr, &b.data, false);
        }
        self.undo.clear();
        self.redo.clear();
        match self.playlist_slot(i) {
            Some(n) => {
                self.session.write_scene(n);
                self.user_names[usize::from(n)] = name.clone();
                let e = &mut self.playlist.entries_mut()[i];
                e.slot = Some((n, name.clone()));
                e.stored = true;
                // Select it after the store (not merged with a select
                // queued earlier, which would go out first).
                let bank = [85, (n / 50) as u8, (n % 50) as u8];
                self.session.write(Area::Setup.base(), &bank, false);
                self.setup.apply(Area::Setup.base(), &bank);
                self.current = self.current_scene();
                self.playlist.at = self.current;
                self.tell(format!(
                    "Stored \"{name}\" in {} for the playlist",
                    slot_label(n)
                ));
            }
            None => {
                self.playlist.at = self.current_scene();
                let why = if self.user_names.iter().all(|n| n.is_empty()) {
                    "scene names are still loading"
                } else {
                    "no empty user scene left"
                };
                self.tell(format!(
                    "Playing \"{name}\" from the Temporary Scene ({why}, so it isn't stored)"
                ));
            }
        }
        // The tones change with the scene: read them once it's all sent.
        let at = Instant::now() + self.session.queued_time() + SETTLE;
        self.tone_reload = [Some(at); 4];
        self.save_playlist();
    }

    /// Copy edits of the playing scene into the active entry. `strict`
    /// also checks the unit is still on that entry's scene (for periodic
    /// captures, which may run while a newly picked scene is coming in).
    pub(super) fn playlist_capture(&mut self, strict: bool) {
        let Some(i) = self.playlist.active else {
            return;
        };
        if !self.dirty || !self.scene.is_loaded() || self.restore.is_some() {
            return;
        }
        if strict
            && (self.scene_reload.is_some()
                || self.pending.get(&TAG_SCENE).is_some_and(|&n| n > 0)
                || self.current_scene() != self.playlist.at)
        {
            return;
        }
        let Some(e) = self.playlist.entries_mut().get_mut(i) else {
            return;
        };
        if same_data(&e.scene, &self.scene) {
            return;
        }
        e.scene = self.scene.clone();
        e.changed = true;
        e.stored = false;
        self.save_playlist();
    }

    /// Called every tick: keep the active entry up to date with edits.
    pub(super) fn playlist_tick(&mut self, now: Instant) -> Option<Duration> {
        if self.playlist.active.is_none() || !self.dirty {
            return None;
        }
        let due = self
            .playlist
            .last_capture
            .map_or(now, |t| t + CAPTURE_EVERY);
        if now < due {
            return Some(due - now);
        }
        self.playlist.last_capture = Some(now);
        self.playlist_capture(true);
        None
    }

    /// After the scene has been (re)loaded from the unit.
    pub(super) fn playlist_loaded(&mut self) {
        let Some(i) = self.playlist.active else {
            return;
        };
        let now = self.current_scene();
        if now != self.playlist.at {
            // Another scene was picked (library or panel).
            self.playlist.active = None;
            return;
        }
        // An unchanged entry follows its original scene.
        let Some(e) = self.playlist.entries_mut().get_mut(i) else {
            return;
        };
        if !e.changed && e.source_ref() == now && !same_data(&e.scene, &self.scene) {
            e.scene = self.scene.clone();
            self.save_playlist();
        }
    }

    /// Where an entry plays from, for display ("U03-01", "P02-14", "U12-50").
    pub fn playlist_where(&self, i: usize) -> String {
        let Some(e) = self.playlist.entries().get(i) else {
            return String::new();
        };
        match (&e.slot, e.changed, e.stored) {
            (Some((n, _)), true, true) => slot_label(*n),
            (_, true, _) => "edited".into(),
            _ => e.source_ref().map_or_else(String::new, |r| r.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene(name: &str) -> Image {
        let mut img = Image::new(Area::TempScene);
        for (a, n) in img.missing_reads() {
            img.apply(a, &vec![0; n as usize]);
        }
        let i = img.index("Perf_common").unwrap();
        img.set_bytes(i, 0, &aerophone_core::image::name_bytes(name));
        img
    }

    fn editor() -> Editor {
        let mut ed = Editor::new();
        ed.playlist = Playlist::default();
        for (a, n) in ed.setup.missing_reads() {
            ed.setup.apply(a, &vec![0; n as usize]);
        }
        ed.setup.apply(Area::Setup.base(), &[85, 0, 0]);
        ed.user_names = vec!["Mine".to_string(); 600];
        ed.user_names[599] = EMPTY.into();
        ed.user_names[597] = EMPTY.into();
        ed
    }

    #[test]
    fn text_round_trip() {
        let mut p = Playlist::default();
        p.entries_mut().push(Entry {
            scene: scene("A"),
            source: Some((85, 2, 0)),
            slot: Some((599, "A".into())),
            changed: true,
            stored: true,
        });
        p.entries_mut().push(Entry {
            scene: scene("B"),
            source: None,
            slot: None,
            changed: true,
            stored: false,
        });
        p.spare.push((598, "Old".into()));
        let text = p.to_text();
        let mut q = Playlist::default();
        q.parse(&text, |f| {
            Some(scene(if f == "001.syx" { "A" } else { "B" }))
        });
        assert_eq!(q.entries().len(), 2);
        assert_eq!(q.entries()[0].source, Some((85, 2, 0)));
        assert_eq!(q.entries()[0].slot, Some((599, "A".into())));
        assert!(q.entries()[0].stored);
        assert_eq!(q.entries()[1].name(), "B");
        assert_eq!(q.spare, [(598, "Old".into())]);
    }

    #[test]
    fn slots_are_only_empty_or_our_own() {
        let mut ed = editor();
        for n in ["A", "B", "C"] {
            ed.playlist.entries_mut().push(Entry {
                scene: scene(n),
                source: None,
                slot: None,
                changed: true,
                stored: false,
            });
        }
        assert_eq!(ed.playlist_slot(0), Some(599));
        ed.playlist.entries_mut()[0].slot = Some((599, "A".into()));
        ed.user_names[599] = "A".into();
        assert_eq!(ed.playlist_slot(1), Some(597), "skips the user's own 598");
        ed.playlist.entries_mut()[1].slot = Some((597, "B".into()));
        ed.user_names[597] = "B".into();
        assert_eq!(ed.playlist_slot(2), None, "no empty slot left");
        // Its own slot, while it still holds what we stored.
        assert_eq!(ed.playlist_slot(0), Some(599));
        // Overwritten by someone else: not ours any more.
        ed.user_names[599] = "Gig opener".into();
        assert_eq!(ed.playlist_slot(0), None);
        assert_eq!(ed.playlist.entries()[0].slot, None);
        // A removed entry's slot is reused.
        ed.playlist_remove(1);
        assert_eq!(ed.playlist_slot(1), Some(597));
    }

    #[test]
    fn changed_entry_is_stored_then_selected() {
        let mut ed = editor();
        ed.playlist.entries_mut().push(Entry {
            scene: scene("Solo"),
            source: Some((85, 0, 0)),
            slot: None,
            changed: true,
            stored: false,
        });
        ed.playlist_play(0);
        let e = &ed.playlist.entries()[0];
        assert_eq!(e.slot, Some((599, "Solo".into())));
        assert!(e.stored);
        assert_eq!(ed.user_names[599], "Solo");
        assert_eq!(ed.current_scene(), Some(SceneRef::User(600)));
        assert_eq!(ed.scene.scene_name().as_deref(), Some("Solo"));
        // Edits while it plays go back into the entry.
        ed.rename_scene("Solo 2");
        ed.playlist_capture(true);
        let e = &ed.playlist.entries()[0];
        assert!(!e.stored);
        assert_eq!(e.name(), "Solo 2");
    }

    #[test]
    fn setlists_share_slots_and_survive_a_round_trip() {
        let mut ed = editor();
        let entry = |n: &str, slot: Option<(u16, String)>| Entry {
            scene: scene(n),
            source: Some((85, 0, 0)),
            slot,
            changed: false,
            stored: false,
        };
        ed.playlist
            .entries_mut()
            .push(entry("A", Some((599, "A".into()))));
        ed.user_names[599] = "A".into();
        ed.setlist_new("Friday gig");
        assert_eq!(ed.playlist.current, 1);
        ed.playlist.entries_mut().push(entry("B", None));
        ed.playlist.entries_mut().push(entry("C", None));
        // 599 belongs to setlist 1's entry: B gets the next empty one.
        assert_eq!(ed.playlist_slot(0), Some(597));
        let text = ed.playlist.to_text();
        let mut q = Playlist::default();
        q.parse(&text, |f| Some(scene(f)));
        assert_eq!(q.lists.len(), 2);
        assert_eq!(q.lists[1].name, "Friday gig");
        assert_eq!(q.current, 1);
        assert_eq!(q.lists[0].entries.len(), 1);
        assert_eq!(q.entries().len(), 2);
        // Removing and clearing.
        ed.playlist_remove_many(&[1, 0, 1]);
        assert!(ed.playlist.entries().is_empty());
        ed.setlist_select(0);
        ed.playlist_clear();
        assert_eq!(ed.playlist.spare, [(599, "A".into())]);
        ed.setlist_delete(1);
        ed.setlist_delete(0);
        assert_eq!(ed.playlist.lists.len(), 1, "always one setlist");
    }

    #[test]
    fn old_single_playlist_file_loads() {
        let mut q = Playlist::default();
        q.parse("entry\t001.syx\t85,0,1\t-\t\t0\t0\n", |f| Some(scene(f)));
        assert_eq!(q.lists.len(), 1);
        assert_eq!(q.entries()[0].source, Some((85, 0, 1)));
    }
}
