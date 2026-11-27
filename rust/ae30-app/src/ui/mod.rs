//! The editor window.

mod commands;
#[cfg(target_os = "macos")]
mod native_menu;
mod pages;
pub mod prefs;
pub mod theme;
mod widgets;

pub use commands::{Command, ThemeChoice};

use std::time::Duration;

use eframe::egui::{self, vec2, Align, Color32, Layout, RichText, Sense, Ui};

use ae30_core::params::data;
use ae30_core::scene::SceneRef;

use crate::editor::{Conn, Editor};
use theme::palette;

pub const APP_NAME: &str = "AE-30 Patch Editor";

/// The app icon (window, Dock, About box); made by `scripts/make-icons.py`.
pub const ICON_PNG: &[u8] = include_bytes!("../../assets/icons/logo.png");

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tab {
    Scene,
    Part,
    Effects,
    Controllers,
    Tone,
    System,
    Live,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FxSel {
    Mfx(u8),
    Ifx,
    Chorus,
    Delay,
    Reverb,
}

pub struct SaveScene {
    pub slot: u16,
    pub name: String,
}

/// The Restore Scenes window: what a chosen `.syx` file holds and where it goes.
pub struct RestoreUi {
    pub file: String,
    pub scenes: Vec<(ae30_core::Area, ae30_core::Image)>,
    pub selected: Vec<bool>,
    /// Put backed-up user scenes back in their own slots.
    pub original: bool,
    /// Otherwise, the first slot (0-based).
    pub start: u16,
}

pub struct SaveTone {
    pub part: u8,
    pub slot: u16,
    pub name: String,
}

/// Everything about what the window shows (not the instrument's data).
pub struct View {
    pub tab: Tab,
    pub part: u8,
    pub tone_part: u8,
    pub partial: u8,
    pub fx: FxSel,
    pub ctl: usize,
    pub ctl_midi: bool,
    pub tone_picker: Option<u8>,
    pub picker_bank: usize,
    pub picker_search: String,
    pub save_scene: Option<SaveScene>,
    pub save_tone: Option<SaveTone>,
    pub lib_user: bool,
    pub lib_search: String,
    pub port: Option<String>,
    pub backup_choice: bool,
    pub restore: Option<RestoreUi>,
    pub about: bool,
    pub prefs: bool,
    /// The playlist entry last scrolled into view.
    pub playlist_shown: Option<usize>,
    /// Setlist select mode, the entries ticked, and the setlist they're in.
    pub pl_select: bool,
    pub pl_selected: std::collections::BTreeSet<usize>,
    pub pl_list: usize,
    /// Naming a setlist: (which, or None for a new one; the name).
    pub pl_name: Option<(Option<usize>, String)>,
    pub pl_confirm: Option<PlConfirm>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PlConfirm {
    Clear,
    Delete(usize),
}

impl Default for View {
    fn default() -> Self {
        View {
            tab: Tab::Scene,
            part: 1,
            tone_part: 1,
            partial: 1,
            fx: FxSel::Mfx(1),
            ctl: 0,
            ctl_midi: false,
            tone_picker: None,
            picker_bank: 0,
            picker_search: String::new(),
            save_scene: None,
            save_tone: None,
            lib_user: true,
            lib_search: String::new(),
            port: None,
            backup_choice: false,
            restore: None,
            about: false,
            prefs: false,
            playlist_shown: None,
            pl_select: false,
            pl_selected: Default::default(),
            pl_list: 0,
            pl_name: None,
            pl_confirm: None,
        }
    }
}

pub struct App {
    pub ed: Editor,
    pub view: View,
    started: bool,
    /// When we last looked for the AE-30 while offline.
    last_scan: Option<std::time::Instant>,
    pub prefs: prefs::Prefs,
    #[cfg(target_os = "macos")]
    native_menu: Option<native_menu::NativeMenu>,
    /// The icon as a texture, loaded on first use (About box).
    icon: Option<egui::TextureHandle>,
    /// Developer aid: `AE30_SCREENSHOT=file.png` saves the window once loaded.
    shot: Option<(String, std::time::Instant, bool)>,
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl App {
    pub fn new() -> Self {
        App {
            ed: Editor::new(),
            view: View::default(),
            started: false,
            last_scan: None,
            icon: None,
            prefs: prefs::Prefs::load(),
            #[cfg(target_os = "macos")]
            native_menu: None,
            shot: std::env::var("AE30_SCREENSHOT")
                .ok()
                .map(|p| (p, std::time::Instant::now(), false)),
        }
    }

    fn connect(&mut self, ctx: &egui::Context) {
        let c = ctx.clone();
        self.ed.connect(
            self.view.port.as_deref(),
            Some(Box::new(move || c.request_repaint())),
        );
    }

    /// Install the macOS menu bar (call from eframe's creation callback).
    #[cfg(target_os = "macos")]
    pub fn install_native_menu(&mut self, ctx: &egui::Context) {
        self.native_menu = Some(native_menu::NativeMenu::install(ctx));
    }

    /// While offline (not after the user chose Disconnect), look for the
    /// AE-30's port every two seconds and connect when it appears.
    fn auto_connect(&mut self, ctx: &egui::Context) {
        if !matches!(self.ed.conn, Conn::Failed(_)) {
            return;
        }
        ctx.request_repaint_after(Duration::from_secs(2));
        if self
            .last_scan
            .is_some_and(|t| t.elapsed() < Duration::from_secs(2))
        {
            return;
        }
        self.last_scan = Some(std::time::Instant::now());
        let hints: Vec<String> = match &self.view.port {
            Some(p) => vec![p.to_lowercase()],
            None => crate::midi::DEFAULT_PORT_HINTS
                .iter()
                .map(|h| h.to_lowercase())
                .collect(),
        };
        let found = crate::midi::list_ports().is_ok_and(|(ins, outs)| {
            let has = |l: &Vec<String>| {
                l.iter()
                    .any(|p| hints.iter().any(|h| p.to_lowercase().contains(h)))
            };
            has(&ins) && has(&outs)
        });
        if found {
            self.connect(ctx);
        }
    }

    fn has_native_menu(&self) -> bool {
        #[cfg(target_os = "macos")]
        return self.native_menu.is_some();
        #[cfg(not(target_os = "macos"))]
        false
    }

    fn handle_commands(&mut self, ctx: &egui::Context) {
        #[cfg_attr(not(target_os = "macos"), allow(unused_mut))]
        let mut cmds = commands::shortcuts(ctx);
        #[cfg(target_os = "macos")]
        if let Some(menu) = &mut self.native_menu {
            cmds.extend(menu.take_commands());
        }
        for c in cmds {
            self.run_command(ctx, c);
        }
    }

    fn open_save(&mut self) {
        if !self.ed.scene.is_loaded() {
            return;
        }
        let slot = match self.ed.current_scene() {
            Some(SceneRef::User(n)) => n - 1,
            _ => self
                .ed
                .user_names
                .iter()
                .position(|n| n == "INIT SCENE")
                .unwrap_or(0) as u16,
        };
        self.view.save_scene = Some(SaveScene {
            slot,
            name: self.ed.scene_name(),
        });
    }

    // ---- top bar --------------------------------------------------------

    fn top_bar(&mut self, ui: &mut Ui) {
        let pal = palette(ui);
        // macOS: the bar sits in the title bar, so empty space in it moves
        // the window and a double-click zooms, like any Mac toolbar.
        // Added first, so the controls on top of it get their clicks.
        #[cfg(target_os = "macos")]
        {
            let bg = ui.interact(
                ui.max_rect(),
                ui.scope_id().with("titlebar"),
                Sense::click_and_drag(),
            );
            if bg.drag_started() {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
            }
            if bg.double_clicked() {
                let max = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::Maximized(!max));
            }
        }
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            // Room for the window buttons (close, minimise, zoom).
            #[cfg(target_os = "macos")]
            ui.add_space(70.0);
            let online = self.ed.scene.is_loaded();
            if ui
                .add_enabled(online, egui::Button::new("◀"))
                .on_hover_text("Previous scene (⌘[)")
                .clicked()
            {
                self.ed.step_scene(-1);
            }
            if ui
                .add_enabled(online, egui::Button::new("▶"))
                .on_hover_text("Next scene (⌘])")
                .clicked()
            {
                self.ed.step_scene(1);
            }
            ui.add_space(6.0);
            let badge = match self.ed.current_scene() {
                Some(SceneRef::User(n)) => {
                    format!("U{:02}-{:02}", (n - 1) / 50 + 1, (n - 1) % 50 + 1)
                }
                Some(SceneRef::Preset { category, program }) => {
                    format!("P{category:02}-{program:02}")
                }
                Some(SceneRef::Unknown { .. }) => "—".into(),
                None => "".into(),
            };
            if !badge.is_empty() {
                egui::Frame::new()
                    .fill(pal.accent_soft)
                    .corner_radius(6)
                    .inner_margin(vec2(8.0, 3.0))
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new(badge)
                                .family(theme::semibold())
                                .color(pal.accent),
                        );
                    });
            }
            if online {
                let mut name = self.ed.scene_name();
                let r = ui.add(
                    egui::TextEdit::singleline(&mut name)
                        .char_limit(16)
                        .font(egui::FontId::new(20.0, theme::semibold()))
                        .frame(egui::Frame::NONE)
                        .desired_width(230.0)
                        .hint_text("Scene name"),
                );
                if r.changed() {
                    self.ed.rename_scene(&name);
                }
                r.on_hover_text("Scene name — click to rename");
                if self.ed.dirty {
                    theme::dot(ui, pal.amber, 8.0);
                    ui.label(RichText::new("edited").color(pal.amber).size(12.0));
                }
            } else {
                ui.label(theme::title(APP_NAME, 18.0));
            }

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if !self.has_native_menu() {
                    ui.menu_button("☰", |ui| self.menu_contents(ui))
                        .response
                        .on_hover_text("Menu");
                }
                let dark = ui.visuals().dark_mode;
                if ui
                    .button(if dark { "☀" } else { "☾" })
                    .on_hover_text("Light / dark (View ▸ Appearance can follow the system)")
                    .clicked()
                {
                    let t = if dark {
                        ThemeChoice::Light
                    } else {
                        ThemeChoice::Dark
                    };
                    let ctx = ui.ctx().clone();
                    self.run_command(&ctx, Command::Theme(t));
                }
                if online {
                    if ui
                        .add(theme::primary(ui, "Save…"))
                        .on_hover_text("Store this scene in a user slot (⌘S)")
                        .clicked()
                    {
                        self.open_save();
                    }
                    let redo = self.ed.redo.last().map(|e| e.label.clone());
                    if ui
                        .add_enabled(redo.is_some(), egui::Button::new("↷"))
                        .on_hover_text(format!("Redo {} (⇧⌘Z)", redo.unwrap_or_default()))
                        .clicked()
                    {
                        self.ed.redo();
                    }
                    let undo = self.ed.undo.last().map(|e| e.label.clone());
                    if ui
                        .add_enabled(undo.is_some(), egui::Button::new("↶"))
                        .on_hover_text(format!("Undo {} (⌘Z)", undo.unwrap_or_default()))
                        .clicked()
                    {
                        self.ed.undo();
                    }
                }
            });
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let tabs = [
                (Tab::Live, "Live"),
                (Tab::Scene, "Scene"),
                (Tab::Part, "Parts"),
                (Tab::Effects, "Effects"),
                (Tab::Controllers, "Controllers"),
                (Tab::Tone, "Tone"),
                (Tab::System, "System"),
            ];
            widgets::segmented(ui, &mut self.view.tab, &tabs);
        });
    }

    // ---- library ----------------------------------------------------------

    fn library(&mut self, ui: &mut Ui) {
        let pal = palette(ui);
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            widgets::segmented(
                ui,
                &mut self.view.lib_user,
                &[(true, "User"), (false, "Presets")],
            );
        });
        ui.add(
            egui::TextEdit::singleline(&mut self.view.lib_search)
                .hint_text("Search scenes")
                .desired_width(f32::INFINITY),
        );
        ui.add_space(4.0);
        let q = self.view.lib_search.to_lowercase();
        let current = self.ed.current_scene();
        let bottom_h = 74.0;
        let list_h = (ui.available_height() - bottom_h).max(100.0);
        let row_h = 22.0;
        if self.view.lib_user {
            let rows: Vec<u16> = (0..600u16)
                .filter(|&n| {
                    q.is_empty()
                        || self.ed.user_names[usize::from(n)]
                            .to_lowercase()
                            .contains(&q)
                })
                .collect();
            egui::ScrollArea::vertical()
                .id_salt("lib_user")
                .max_height(list_h)
                .auto_shrink([false, false])
                .show_rows(ui, row_h, rows.len(), |ui, range| {
                    for &n in &rows[range] {
                        let name = self.ed.user_names[usize::from(n)].clone();
                        let empty = name.is_empty() || name == "INIT SCENE";
                        let sel = current == Some(SceneRef::User(n + 1));
                        let label = format!("U{:02}-{:02}", n / 50 + 1, n % 50 + 1);
                        let r = lib_row(
                            ui,
                            &label,
                            if name.is_empty() { "…" } else { &name },
                            sel,
                            empty,
                        );
                        if r.clicked() {
                            self.ed.select_user(n);
                        }
                        r.context_menu(|ui| {
                            if ui.button("Save current scene here…").clicked() {
                                self.view.save_scene = Some(SaveScene {
                                    slot: n,
                                    name: self.ed.scene_name(),
                                });
                                ui.close();
                            }
                        });
                    }
                });
        } else {
            let rows: Vec<usize> = (0..self.ed.preset_names.len())
                .filter(|&i| {
                    q.is_empty() || self.ed.preset_names[i].name.to_lowercase().contains(&q)
                })
                .collect();
            if rows.is_empty() && q.is_empty() {
                ui.label(RichText::new("Preset names load after connecting…").color(pal.weak));
            }
            egui::ScrollArea::vertical()
                .id_salt("lib_preset")
                .max_height(list_h)
                .auto_shrink([false, false])
                .show_rows(ui, row_h, rows.len(), |ui, range| {
                    for &i in &rows[range] {
                        let s = self.ed.preset_names[i].clone();
                        let r = SceneRef::from_bank(s.msb, s.lsb, s.pc);
                        let label = match r {
                            SceneRef::Preset { category, program } => {
                                format!("P{category:02}-{program:02}")
                            }
                            _ => String::new(),
                        };
                        if lib_row(ui, &label, &s.name, current == Some(r), false).clicked() {
                            self.ed.select_scene(s.msb, s.lsb, s.pc);
                        }
                    }
                });
        }
        ui.add_space(6.0);
        ui.separator();
        let online = self.ed.is_online();
        let ctx = ui.ctx().clone();
        let mut run = None;
        ui.horizontal_wrapped(|ui| {
            let shortcut = if cfg!(target_os = "macos") {
                "⌘"
            } else {
                "Ctrl+"
            };
            if ui
                .add_enabled(online, egui::Button::new("Import…"))
                .on_hover_text(format!(
                    "Load a .syx scene into the AE-30, not stored until you save ({shortcut}O)"
                ))
                .clicked()
            {
                run = Some(Command::Import);
            }
            if ui
                .add_enabled(self.ed.scene.is_loaded(), egui::Button::new("Export…"))
                .on_hover_text(format!("Save this scene as a .syx file ({shortcut}E)"))
                .clicked()
            {
                run = Some(Command::Export);
            }
            if ui
                .add_enabled(
                    online && self.ed.backup.is_none(),
                    egui::Button::new("Back Up…"),
                )
                .on_hover_text("Save your user scenes to one .syx file")
                .clicked()
            {
                run = Some(Command::Backup);
            }
            if ui
                .add_enabled(
                    online && self.ed.backup.is_none() && self.ed.restore.is_none(),
                    egui::Button::new("Restore…"),
                )
                .on_hover_text("Write scenes from a .syx backup back to the AE-30")
                .clicked()
            {
                run = Some(Command::Restore);
            }
        });
        if let Some(c) = run {
            self.run_command(&ctx, c);
        }
    }

    // ---- playlist (Live tab) ------------------------------------------------

    /// Arrow keys and Page Up/Down (what most page-turner pedals send) step
    /// through the playlist while nothing else has the keyboard.
    fn playlist_keys(&mut self, ctx: &egui::Context) {
        if ctx.memory(|m| m.focused().is_some()) {
            return;
        }
        let none = egui::Modifiers::NONE;
        let (down, up) = ctx.input_mut(|i| {
            (
                i.consume_key(none, egui::Key::ArrowDown)
                    | i.consume_key(none, egui::Key::PageDown)
                    | i.consume_key(none, egui::Key::ArrowRight),
                i.consume_key(none, egui::Key::ArrowUp)
                    | i.consume_key(none, egui::Key::PageUp)
                    | i.consume_key(none, egui::Key::ArrowLeft),
            )
        });
        if down {
            self.ed.playlist_step(1);
        }
        if up {
            self.ed.playlist_step(-1);
        }
    }

    fn playlist(&mut self, ui: &mut Ui) {
        let pal = palette(ui);
        let pl = &self.ed.playlist;
        let n = pl.entries().len();
        if pl.current != self.view.pl_list {
            // Another setlist: selections don't carry over.
            self.view.pl_list = pl.current;
            self.view.pl_selected.clear();
        }
        ui.add_space(8.0);
        // Setlist picker and its menu.
        let mut pick = None;
        ui.horizontal(|ui| {
            ui.label(theme::section(ui, "Setlist"));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.menu_button("⋯", |ui| {
                    if ui.button("New setlist…").clicked() {
                        self.view.pl_name = Some((None, String::new()));
                    }
                    if ui.button("Rename…").clicked() {
                        let pl = &self.ed.playlist;
                        self.view.pl_name = Some((Some(pl.current), pl.list_name().to_string()));
                    }
                    ui.separator();
                    if ui.button("Delete setlist…").clicked() {
                        self.view.pl_confirm = Some(PlConfirm::Delete(self.ed.playlist.current));
                    }
                })
                .response
                .on_hover_text("New, rename or delete setlists");
                let pl = &self.ed.playlist;
                ui.spacing_mut().combo_width = ui.available_width();
                egui::ComboBox::from_id_salt("setlist")
                    .selected_text(RichText::new(pl.list_name()).family(theme::semibold()))
                    .show_ui(ui, |ui| {
                        for (i, l) in pl.lists.iter().enumerate() {
                            let label = format!("{}   ({})", l.name, l.entries.len());
                            if ui.selectable_label(i == pl.current, label).clicked() {
                                pick = Some(i);
                            }
                        }
                    });
            });
        });
        if let Some(i) = pick {
            self.ed.setlist_select(i);
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let sel_w = 64.0;
            let w = ui.available_width() - sel_w - ui.spacing().item_spacing.x;
            if ui
                .add_enabled(
                    self.ed.scene.is_loaded(),
                    egui::Button::new("+  Add current scene").min_size(vec2(w, 28.0)),
                )
                .on_hover_text("Keep a copy of the scene playing now at the end of this setlist")
                .clicked()
            {
                self.ed.playlist_add();
            }
            let r = ui.add_enabled(
                n > 0,
                egui::Button::selectable(self.view.pl_select, "Select").min_size(vec2(sel_w, 28.0)),
            );
            if r.on_hover_text("Choose entries to remove").clicked() {
                self.view.pl_select = !self.view.pl_select;
                self.view.pl_selected.clear();
            }
        });
        if n == 0 {
            self.view.pl_select = false;
        }
        if self.view.pl_select {
            let k = self.view.pl_selected.len();
            ui.horizontal_wrapped(|ui| {
                if ui.small_button("All").clicked() {
                    self.view.pl_selected = (0..n).collect();
                }
                if ui.small_button("None").clicked() {
                    self.view.pl_selected.clear();
                }
                if ui
                    .add_enabled(k > 0, egui::Button::new(format!("Remove {k}")).small())
                    .clicked()
                {
                    let which: Vec<usize> = self.view.pl_selected.iter().copied().collect();
                    self.ed.playlist_remove_many(&which);
                    self.view.pl_selected.clear();
                    self.view.pl_select = false;
                }
                if ui
                    .add(egui::Button::new(RichText::new("Clear setlist…").color(pal.bad)).small())
                    .clicked()
                {
                    self.view.pl_confirm = Some(PlConfirm::Clear);
                }
            });
        }
        ui.add_space(6.0);
        let list_h = (ui.available_height() - 96.0).max(100.0);
        let active = self.ed.playlist.active;
        let mut act: Option<(usize, i32)> = None; // (entry, 0 play / ±1 move / 2 revert / 3 remove)
        egui::ScrollArea::vertical()
            .id_salt("playlist")
            .max_height(list_h)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if n == 0 {
                    ui.label(
                        RichText::new(
                            "Pick a scene in the library, set it up, then Add current scene. \
                             Repeat for each song.",
                        )
                        .color(pal.weak),
                    );
                }
                for i in 0..n {
                    let e = &self.ed.playlist.entries()[i];
                    let name = e.name();
                    let can_revert = e.changed && e.source.is_some();
                    let pending = e.changed && !e.stored;
                    let place = self.ed.playlist_where(i);
                    let ticked = self
                        .view
                        .pl_select
                        .then(|| self.view.pl_selected.contains(&i));
                    let r =
                        playlist_row(ui, i + 1, &name, &place, active == Some(i), pending, ticked);
                    if r.clicked() {
                        if self.view.pl_select {
                            if !self.view.pl_selected.remove(&i) {
                                self.view.pl_selected.insert(i);
                            }
                        } else {
                            act = Some((i, 0));
                        }
                    }
                    if active == Some(i) && self.view.playlist_shown != active {
                        r.scroll_to_me(Some(Align::Center));
                    }
                    r.on_hover_text(if self.view.pl_select {
                        "Click to select"
                    } else if pending {
                        "Edited: stored in an empty user scene when you pick it"
                    } else {
                        "Pick to play (right-click for more)"
                    })
                    .context_menu(|ui| {
                        if ui
                            .add_enabled(i > 0, egui::Button::new("Move up"))
                            .clicked()
                        {
                            act = Some((i, -1));
                        }
                        if ui
                            .add_enabled(i + 1 < n, egui::Button::new("Move down"))
                            .clicked()
                        {
                            act = Some((i, 1));
                        }
                        if ui
                            .add_enabled(can_revert, egui::Button::new("Revert to original scene"))
                            .clicked()
                        {
                            act = Some((i, 2));
                        }
                        ui.separator();
                        if ui.button("Remove from setlist").clicked() {
                            act = Some((i, 3));
                        }
                    });
                }
            });
        self.view.playlist_shown = active;
        if act.is_some() {
            self.view.pl_selected.clear();
        }
        match act {
            Some((i, 0)) => self.ed.playlist_play(i),
            Some((i, d @ (-1 | 1))) => self.ed.playlist_move(i, d),
            Some((i, 2)) => self.ed.playlist_revert(i),
            Some((i, 3)) => self.ed.playlist_remove(i),
            _ => {}
        }
        ui.add_space(6.0);
        ui.separator();
        ui.horizontal(|ui| {
            let w = (ui.available_width() - ui.spacing().item_spacing.x) / 2.0;
            let size = vec2(w, 34.0);
            let has = n > 0;
            if ui
                .add_enabled(has, egui::Button::new("◀  Previous").min_size(size))
                .on_hover_text("↑ or Page Up")
                .clicked()
            {
                self.ed.playlist_step(-1);
            }
            if ui
                .add_enabled(has, egui::Button::new("Next  ▶").min_size(size))
                .on_hover_text("↓ or Page Down")
                .clicked()
            {
                self.ed.playlist_step(1);
            }
        });
        ui.label(
            RichText::new(
                "Edited entries are stored in empty user scenes from U12-50 down when you pick them.",
            )
            .size(11.0)
            .color(pal.weak),
        );
    }

    /// The setlist name and confirmation dialogs.
    fn setlist_dialogs(&mut self, ctx: &egui::Context) {
        if let Some((which, mut name)) = self.view.pl_name.take() {
            let mut done = None;
            let title = if which.is_some() {
                "Rename setlist"
            } else {
                "New setlist"
            };
            let modal = egui::Modal::new(egui::Id::unique("setlist_name")).show(ctx, |ui| {
                ui.set_width(300.0);
                ui.label(theme::title(title, 16.0));
                ui.add_space(6.0);
                let r = ui.add(
                    egui::TextEdit::singleline(&mut name)
                        .hint_text("e.g. Friday at Blue Note")
                        .desired_width(f32::INFINITY),
                );
                r.request_focus();
                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        done = Some(false);
                    }
                    let ok = if which.is_some() { "Rename" } else { "Create" };
                    if ui.add(theme::primary(ui, ok)).clicked() || enter {
                        done = Some(true);
                    }
                });
            });
            if modal.should_close() && done.is_none() {
                done = Some(false);
            }
            match (done, which) {
                (Some(true), Some(l)) => self.ed.setlist_rename(l, &name),
                (Some(true), None) => self.ed.setlist_new(&name),
                (Some(false), _) => {}
                (None, _) => self.view.pl_name = Some((which, name)),
            }
        }
        if let Some(c) = self.view.pl_confirm {
            let pl = &self.ed.playlist;
            let (title, text, ok) = match c {
                PlConfirm::Clear => (
                    "Clear setlist?",
                    format!(
                        "Remove all {} entries from \"{}\"? The scenes on the AE-30 aren't changed.",
                        pl.entries().len(),
                        pl.list_name()
                    ),
                    "Clear",
                ),
                PlConfirm::Delete(l) => (
                    "Delete setlist?",
                    format!(
                        "Delete \"{}\" and its {} entries? The scenes on the AE-30 aren't changed.",
                        pl.lists.get(l).map_or("", |x| x.name.as_str()),
                        pl.lists.get(l).map_or(0, |x| x.entries.len())
                    ),
                    "Delete",
                ),
            };
            let mut done = None;
            let modal = egui::Modal::new(egui::Id::unique("setlist_confirm")).show(ctx, |ui| {
                ui.set_width(320.0);
                ui.label(theme::title(title, 16.0));
                ui.add_space(6.0);
                ui.label(text);
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        done = Some(false);
                    }
                    if ui.add(theme::primary(ui, ok)).clicked() {
                        done = Some(true);
                    }
                });
            });
            if modal.should_close() && done.is_none() {
                done = Some(false);
            }
            if let Some(yes) = done {
                self.view.pl_confirm = None;
                if yes {
                    match c {
                        PlConfirm::Clear => self.ed.playlist_clear(),
                        PlConfirm::Delete(l) => self.ed.setlist_delete(l),
                    }
                    self.view.pl_selected.clear();
                    self.view.pl_select = false;
                }
            }
        }
    }

    // ---- status bar ---------------------------------------------------------

    fn status_bar(&mut self, ui: &mut Ui) {
        let pal = palette(ui);
        ui.horizontal(|ui| {
            let (color, text) = match &self.ed.conn {
                Conn::Online(id) => (
                    pal.good,
                    format!("{:?} · {}", id.model, self.ed.port_name().unwrap_or(""))
                        .replace("Ae30", "AE-30")
                        .replace("Ae20", "AE-20"),
                ),
                Conn::Connecting => (pal.amber, "Connecting…".into()),
                Conn::Offline => (pal.weak, "Not connected".into()),
                Conn::Failed(_) => (pal.bad, "Connection failed".into()),
            };
            theme::dot(ui, color, 9.0);
            ui.label(RichText::new(text).size(12.0));
            if self.ed.is_loading() {
                ui.spinner();
            }
            let job = if let Some(b) = &self.ed.backup {
                Some(("Backup", b.progress(), b.eta(), true))
            } else {
                self.ed
                    .restore
                    .as_ref()
                    .map(|r| ("Restoring", r.progress(), r.eta(), false))
            };
            if let Some((what, (done, total), eta, backup)) = job {
                let eta = eta
                    .map(|d| format!(", ~{}:{:02} left", d.as_secs() / 60, d.as_secs() % 60))
                    .unwrap_or_default();
                // Bar and text drawn separately, so the text is always in
                // the theme's text colour.
                ui.separator();
                let (rect, _) = ui.allocate_exact_size(vec2(120.0, 8.0), Sense::hover());
                ui.painter().rect_filled(rect, 4.0, pal.control);
                let mut fill = rect;
                fill.set_width(rect.width() * done as f32 / total.max(1) as f32);
                ui.painter().rect_filled(fill, 4.0, pal.accent);
                ui.label(
                    RichText::new(format!("{what} {done}/{total}{eta}"))
                        .size(12.0)
                        .color(pal.text),
                );
                if ui.small_button("Cancel").clicked() {
                    if backup {
                        self.ed.cancel_backup();
                    } else {
                        self.ed.cancel_restore();
                    }
                }
            }
            if let Some((msg, at, warn)) = &self.ed.status {
                if at.elapsed() < Duration::from_secs(8) {
                    ui.separator();
                    ui.label(RichText::new(msg).size(12.0).color(if *warn {
                        pal.amber
                    } else {
                        pal.text
                    }));
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                // Live meters from what the AE-30 sends while you play
                // (laid out right to left).
                let live = Duration::from_secs(2);
                // Pitch bend (bite, wheel): -8192..8191, drawn from the centre.
                let (bend, at) = self.ed.bend;
                let (rect, _) = ui.allocate_exact_size(vec2(80.0, 10.0), Sense::hover());
                ui.painter().rect_filled(rect, 5.0, pal.control);
                if at.elapsed() < live && bend != 0 {
                    let c = rect.center().x;
                    let x = c + rect.width() / 2.0 * f32::from(bend) / 8192.0;
                    let r = egui::Rect::from_x_y_ranges(c.min(x)..=c.max(x), rect.y_range());
                    ui.painter().rect_filled(r, 3.0, pal.amber);
                }
                ui.painter().line_segment(
                    [rect.center_top(), rect.center_bottom()],
                    egui::Stroke::new(1.0, pal.weak),
                );
                let cents = i32::from(bend) * 100 / 4096;
                let bend_text = if at.elapsed() < live && bend != 0 {
                    format!("Bend {cents:+}")
                } else {
                    "Bend".into()
                };
                ui.label(RichText::new(bend_text).size(12.0).color(pal.weak))
                    .on_hover_text(
                        "Pitch bend the AE-30 sends (bite, wheel), as a share of the bend range",
                    );
                ui.add_space(10.0);

                // Breath: CC2 or CC11.
                let (v, at) = self.ed.breath;
                let (rect, _) = ui.allocate_exact_size(vec2(110.0, 10.0), Sense::hover());
                ui.painter().rect_filled(rect, 5.0, pal.control);
                if at.elapsed() < live {
                    let mut r = rect;
                    r.set_width(rect.width() * f32::from(v) / 127.0);
                    ui.painter().rect_filled(r, 5.0, pal.accent);
                }
                let note = self.ed.note.map(note_name).unwrap_or_default();
                ui.label(
                    RichText::new(format!("Breath {note}"))
                        .size(12.0)
                        .color(pal.weak),
                );
            });
        });
    }

    // ---- offline screen --------------------------------------------------------

    fn welcome(&mut self, ui: &mut Ui) {
        let pal = palette(ui);
        ui.add_space(60.0);
        ui.vertical_centered(|ui| {
            theme::card(ui).show(ui, |ui| {
                ui.set_width(460.0);
                ui.vertical_centered(|ui| {
                    ui.label(theme::title(APP_NAME, 24.0));
                    ui.label(RichText::new("For the Roland Aerophone Pro AE-30").color(pal.weak));
                    ui.add_space(14.0);
                    match &self.ed.conn {
                        Conn::Connecting => {
                            ui.spinner();
                            ui.label("Looking for the AE-30…");
                        }
                        Conn::Failed(e) => {
                            let headline = if e.starts_with("no MIDI") {
                                "The AE-30 isn't showing up. Is it switched on and connected by USB?"
                            } else {
                                "The AE-30 didn't answer."
                            };
                            ui.label(RichText::new(headline).color(pal.bad));
                            ui.label(RichText::new("The editor connects by itself as soon as it appears.").size(12.0).color(pal.weak));
                            ui.collapsing("Details", |ui| {
                                ui.label(RichText::new(e).size(11.5).color(pal.weak));
                            });
                        }
                        _ => {}
                    }
                    ui.add_space(8.0);
                    let ports = crate::midi::list_ports().map(|(i, _)| i).unwrap_or_default();
                    ui.horizontal(|ui| {
                        ui.label("Port");
                        egui::ComboBox::from_id_salt("port")
                            .selected_text(self.view.port.clone().unwrap_or_else(|| "Find automatically".into()))
                            .width(240.0)
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut self.view.port, None, "Find automatically");
                                for p in ports {
                                    ui.selectable_value(&mut self.view.port, Some(p.clone()), p);
                                }
                            });
                    });
                    ui.add_space(8.0);
                    if ui.add(theme::primary(ui, "Connect")).clicked() {
                        self.connect(ui.ctx());
                    }
                    ui.add_space(12.0);
                    ui.label(
                        RichText::new(
                            "Connect the AE-30 by USB. If it doesn't show up, set its USB Driver to \
                             Generic (Menu ▸ System ▸ USB). Close Roland's phone app first.",
                        )
                        .size(12.0)
                        .color(pal.weak),
                    );
                });
            });
        });
    }

    // ---- dialogs ----------------------------------------------------------------

    fn tone_picker(&mut self, ctx: &egui::Context) {
        let Some(part) = self.view.tone_picker else {
            return;
        };
        let mut open = true;
        let current = self.ed.part_tone(part);
        egui::Window::new(format!("Tone for Part {part}"))
            .open(&mut open)
            .default_size(vec2(460.0, 560.0))
            .collapsible(false)
            .show(ctx, |ui| {
                let pal = palette(ui);
                ui.label(
                    RichText::new("Click a tone to hear it right away. Your scene isn't stored until you save.")
                        .size(12.0)
                        .color(pal.weak),
                );
                let banks = ["Preset A", "Preset B", "Preset C", "SuperNATURAL", "Drum Kits", "User"];
                ui.horizontal_wrapped(|ui| {
                    for (i, b) in banks.iter().enumerate() {
                        if ui.selectable_label(self.view.picker_bank == i, *b).clicked() {
                            self.view.picker_bank = i;
                            if i == 5 && self.ed.user_tones.is_none() {
                                self.ed.load_user_tones();
                            }
                        }
                    }
                });
                ui.add(
                    egui::TextEdit::singleline(&mut self.view.picker_search)
                        .hint_text("Search tones or categories")
                        .desired_width(f32::INFINITY),
                );
                let q = self.view.picker_search.to_lowercase();
                let d = data();
                let (key, msb, lsb0): (&str, u8, u8) = match self.view.picker_bank {
                    0 => ("A", 71, 72),
                    1 => ("B", 71, 73),
                    2 => ("C", 71, 74),
                    3 => ("SN", 89, 66),
                    4 => ("DK", 70, 66),
                    _ => ("U", 71, 8),
                };
                // (msb, lsb, pc, name, category)
                let items: Vec<(u8, u8, u8, String, String)> = if key == "U" {
                    match &self.ed.user_tones {
                        Some(names) => names
                            .iter()
                            .enumerate()
                            .map(|(i, n)| (71, 8 + (i / 128) as u8, (i % 128) as u8, n.clone(), String::new()))
                            .filter(|(_, _, _, n, _)| !n.is_empty() && n != "INIT TONE")
                            .collect(),
                        None => {
                            ui.spinner();
                            Vec::new()
                        }
                    }
                } else {
                    let names = d.tones.get(key).cloned().unwrap_or_default();
                    let cats = d.tone_cats.get(key).cloned().unwrap_or_default();
                    names
                        .iter()
                        .enumerate()
                        .filter(|(_, n)| !n.is_empty())
                        .map(|(i, n)| (msb, lsb0, i as u8, n.clone(), cats.get(i).cloned().unwrap_or_default()))
                        .collect()
                };
                let items: Vec<_> = items
                    .into_iter()
                    .filter(|it| q.is_empty() || it.3.to_lowercase().contains(&q) || it.4.to_lowercase().contains(&q))
                    .collect();
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show_rows(ui, 22.0, items.len(), |ui, range| {
                        for (m, l, p, name, cat) in &items[range] {
                            let sel = current == Some((*m, *l, *p));
                            let r = lib_row_w(ui, cat, name, sel, false, 125.0);
                            if r.clicked() {
                                self.ed.choose_tone(part, *m, *l, *p);
                            }
                        }
                    });
            });
        if !open {
            self.view.tone_picker = None;
        }
    }

    /// Read a `.syx` file and open the Restore window for it.
    pub fn open_restore(&mut self, path: &std::path::Path) {
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) => {
                return self
                    .ed
                    .warn(format!("Couldn't read {}: {e}", path.display()))
            }
        };
        let scenes: Vec<_> = ae30_core::syx::scenes_from_syx(&bytes)
            .into_iter()
            .filter(|(_, img)| img.is_loaded())
            .collect();
        if scenes.is_empty() {
            return self
                .ed
                .warn("That file has no complete AE-30 scenes in it.");
        }
        let original = scenes
            .iter()
            .all(|(a, _)| matches!(a, ae30_core::Area::UserScene(_)));
        let start = self
            .ed
            .user_names
            .iter()
            .position(|n| n == "INIT SCENE")
            .unwrap_or(0) as u16;
        self.view.restore = Some(RestoreUi {
            file: path
                .file_name()
                .map(|f| f.to_string_lossy().into_owned())
                .unwrap_or_default(),
            selected: vec![true; scenes.len()],
            scenes,
            original,
            start,
        });
    }

    fn restore_dialog(&mut self, ctx: &egui::Context) {
        let Some(st) = &mut self.view.restore else {
            return;
        };
        let mut close = false;
        let mut go = false;
        // Where each selected scene would go.
        let targets: Vec<Option<u16>> = {
            let mut k = 0u16;
            st.scenes
                .iter()
                .zip(&st.selected)
                .map(|((area, _), &sel)| {
                    if !sel {
                        return None;
                    }
                    let t = match (st.original, area) {
                        (true, ae30_core::Area::UserScene(n)) => Some(*n),
                        _ => Some(st.start + k),
                    };
                    k += 1;
                    t.filter(|&t| t < 600)
                })
                .collect()
        };
        let count = targets.iter().flatten().count();
        let overflow = st.selected.iter().filter(|&&s| s).count() > count;
        let overwrites = targets
            .iter()
            .flatten()
            .filter(|&&t| {
                let n = &self.ed.user_names[usize::from(t)];
                !n.is_empty() && n != "INIT SCENE"
            })
            .count();
        let can_original = st
            .scenes
            .iter()
            .all(|(a, _)| matches!(a, ae30_core::Area::UserScene(_)));
        egui::Modal::new(egui::Id::unique("restore")).show(ctx, |ui| {
            let pal = palette(ui);
            ui.set_width(560.0);
            ui.label(theme::title("Restore Scenes", 19.0));
            ui.label(
                RichText::new(format!("{} — {} scenes", st.file, st.scenes.len())).color(pal.weak),
            );
            ui.add_space(8.0);
            if can_original {
                ui.radio_value(&mut st.original, true, "Put each scene back in its own slot");
            }
            ui.horizontal(|ui| {
                ui.radio_value(&mut st.original, false, "Place them one after another from");
                let mut slot = i32::from(st.start) + 1;
                ui.add_enabled(
                    !st.original,
                    egui::DragValue::new(&mut slot)
                        .range(1..=600)
                        .custom_formatter(|v, _| {
                            let n = v as u16 - 1;
                            format!("U{:02}-{:02}", n / 50 + 1, n % 50 + 1)
                        })
                        .custom_parser(|t| {
                            let t = t.trim().trim_start_matches(['U', 'u']);
                            match t.split_once('-') {
                                Some((b, s)) => {
                                    let (b, s): (f64, f64) = (b.parse().ok()?, s.parse().ok()?);
                                    Some((b - 1.0) * 50.0 + s)
                                }
                                None => t.parse().ok(),
                            }
                        }),
                );
                st.start = (slot - 1).clamp(0, 599) as u16;
            });
            if !can_original {
                st.original = false;
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.small_button("Select all").clicked() {
                    st.selected.iter_mut().for_each(|s| *s = true);
                }
                if ui.small_button("Select none").clicked() {
                    st.selected.iter_mut().for_each(|s| *s = false);
                }
            });
            egui::ScrollArea::vertical()
                .max_height(320.0)
                .auto_shrink([false, false])
                .show_rows(ui, 24.0, st.scenes.len(), |ui, range| {
                    for i in range {
                        ui.horizontal(|ui| {
                            ui.set_height(24.0);
                            ui.checkbox(&mut st.selected[i], "");
                            let name = st.scenes[i].1.scene_name().unwrap_or_default();
                            match targets[i] {
                                Some(t) => {
                                    let old = &self.ed.user_names[usize::from(t)];
                                    let empty = old.is_empty() || old == "INIT SCENE";
                                    ui.label(
                                        RichText::new(format!("U{:02}-{:02}", t / 50 + 1, t % 50 + 1))
                                            .color(pal.accent),
                                    );
                                    ui.label(&name);
                                    let note = if empty {
                                        RichText::new("into an empty slot").size(11.5).color(pal.weak)
                                    } else if *old == name {
                                        RichText::new("same name there now").size(11.5).color(pal.weak)
                                    } else {
                                        RichText::new(format!("replaces \"{old}\"")).size(11.5).color(pal.amber)
                                    };
                                    ui.label(note);
                                }
                                None => {
                                    ui.label(RichText::new(&name).color(pal.weak));
                                }
                            }
                        });
                    }
                });
            ui.add_space(6.0);
            if overflow {
                ui.label(RichText::new("Some scenes don't fit before U12-50 and will be skipped.").color(pal.amber));
            }
            let mut warn = format!(
                "This writes {count} scene{} to the AE-30",
                if count == 1 { "" } else { "s" }
            );
            if overwrites > 0 {
                warn += &format!(", replacing {overwrites} named scene{}", if overwrites == 1 { "" } else { "s" });
            }
            warn += ". Each one is read back and checked. The current scene's unsaved edits are lost. About 3 seconds per scene.";
            ui.label(RichText::new(warn).size(12.0).color(if overwrites > 0 { pal.amber } else { pal.weak }));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let label = format!("Restore {count} Scene{}", if count == 1 { "" } else { "s" });
                if ui.add_enabled(count > 0, theme::primary(ui, &label)).clicked() {
                    go = true;
                }
                if ui.button("Cancel").clicked() {
                    close = true;
                }
            });
        });
        if go {
            let st = self.view.restore.take().unwrap();
            let items: Vec<(u16, ae30_core::Image)> = st
                .scenes
                .into_iter()
                .zip(targets)
                .filter_map(|((_, img), t)| t.map(|t| (t, img)))
                .collect();
            self.ed.start_restore(items);
        } else if close {
            self.view.restore = None;
        }
    }

    fn backup_dialog(&mut self, ctx: &egui::Context) {
        if !self.view.backup_choice {
            return;
        }
        let named = self.ed.named_scenes();
        let mut choice: Option<Vec<u16>> = None;
        let mut close = false;
        egui::Modal::new(egui::Id::unique("backup")).show(ctx, |ui| {
            let pal = palette(ui);
            ui.set_width(420.0);
            ui.label(theme::title("Back Up User Scenes", 19.0));
            ui.label(
                RichText::new(
                    "The AE-30 sends stored scenes slowly (about 1.3 seconds each). You can keep \
                     playing with the editor while it runs.",
                )
                .size(12.0)
                .color(pal.weak),
            );
            ui.add_space(8.0);
            let mins = |n: usize| (n as f32 * 1.3 / 60.0).ceil() as usize;
            if ui
                .add(theme::primary(
                    ui,
                    &format!(
                        "My scenes: {} named (~{} min)",
                        named.len(),
                        mins(named.len())
                    ),
                ))
                .on_hover_text("Skips slots called INIT SCENE")
                .clicked()
            {
                choice = Some(named.clone());
            }
            if ui
                .button(format!("All 600 scenes (~{} min)", mins(600)))
                .clicked()
            {
                choice = Some((0..600).collect());
            }
            ui.add_space(4.0);
            if ui.button("Cancel").clicked() {
                close = true;
            }
        });
        if let Some(want) = choice {
            close = true;
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("SysEx", &["syx"])
                .set_file_name("AE-30 user scenes.syx")
                .save_file()
            {
                self.ed.start_backup(path, want);
            }
        }
        if close {
            self.view.backup_choice = false;
        }
    }

    fn apply_knob_speed(&self) {
        widgets::KNOB_PIXELS.store(
            self.prefs.knob_speed.pixels() as u32,
            std::sync::atomic::Ordering::Relaxed,
        );
    }

    fn preferences_window(&mut self, ctx: &egui::Context) {
        if !self.view.prefs {
            return;
        }
        let mut p = self.prefs.clone();
        let (mut open, mut done, mut reset) = (true, false, false);
        let title = if cfg!(target_os = "macos") {
            "Settings"
        } else {
            "Preferences"
        };
        egui::Window::new(title)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(480.0)
            .show(ctx, |ui| {
                ui.label(theme::title("Appearance", 16.0));
                ui.add_space(4.0);
                egui::Grid::new("prefs_look")
                    .num_columns(2)
                    .spacing([18.0, 12.0])
                    .show(ui, |ui| {
                        ui.label("Theme");
                        ui.horizontal(|ui| {
                            for (t, label) in ThemeChoice::ALL {
                                ui.selectable_value(&mut p.theme, t, label);
                            }
                        });
                        ui.end_row();

                        let name = ui.label("Accent colour").id;
                        ui.horizontal_wrapped(|ui| {
                            let dark = ui.visuals().dark_mode;
                            let default = ("Teal (default)", theme::default_accent(dark), None);
                            let choices = std::iter::once(default)
                                .chain(theme::ACCENTS.iter().map(|&(n, c)| (n, c, Some(c))));
                            for (label, colour, value) in choices {
                                if accent_swatch(ui, label, colour, p.look.accent == value) {
                                    p.look.accent = value;
                                }
                            }
                            let current = p.look.accent.unwrap_or(theme::default_accent(dark));
                            let mut rgb = [current.r(), current.g(), current.b()];
                            let r = egui::color_picker::color_edit_button_srgb(ui, &mut rgb)
                                .labelled_by(name)
                                .on_hover_text("Any colour");
                            if r.changed() {
                                p.look.accent = Some(Color32::from_rgb(rgb[0], rgb[1], rgb[2]));
                            }
                        });
                        ui.end_row();

                        let name = ui.label("Corners").id;
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Square").weak());
                            ui.add(
                                egui::Slider::new(&mut p.look.corners, 0..=theme::MAX_CORNERS)
                                    .show_value(false),
                            )
                            .labelled_by(name);
                            ui.label(RichText::new("Round").weak());
                        });
                        ui.end_row();

                        ui.label("Interface size");
                        ui.horizontal(|ui| {
                            let mut pct = (p.look.scale * 100.0).round() as u32;
                            for n in theme::SCALES {
                                ui.selectable_value(&mut pct, n, format!("{n}%"));
                            }
                            p.look.scale = pct as f32 / 100.0;
                        });
                        ui.end_row();

                        ui.label("Sliders");
                        ui.checkbox(&mut p.look.slider_fill, "Fill the track up to the handle");
                        ui.end_row();
                    });

                ui.add_space(12.0);
                ui.label(theme::title("Editing", 16.0));
                ui.add_space(4.0);
                egui::Grid::new("prefs_edit")
                    .num_columns(2)
                    .spacing([18.0, 12.0])
                    .show(ui, |ui| {
                        ui.label("Knob speed");
                        ui.horizontal(|ui| {
                            for (k, label) in prefs::KnobSpeed::ALL {
                                ui.selectable_value(&mut p.knob_speed, k, label);
                            }
                        })
                        .response
                        .on_hover_text("How far to drag a knob for its whole range. Hold Shift while dragging for fine steps.");
                        ui.end_row();

                        ui.label("Scene list");
                        ui.checkbox(&mut p.hide_empty, "Hide empty slots (INIT SCENE)");
                        ui.end_row();
                    });

                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    reset = ui.button("Reset to Defaults").clicked();
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        done = ui.add(theme::primary(ui, "Done")).clicked();
                    });
                });
            });
        if reset {
            p = prefs::Prefs::default();
        }
        if p != self.prefs {
            if p.look != self.prefs.look {
                theme::apply(ctx, p.look);
            }
            if p.theme != self.prefs.theme {
                p.theme.apply(ctx);
            }
            self.prefs = p;
            self.apply_knob_speed();
            self.prefs.save();
        }
        if done || !open {
            self.view.prefs = false;
        }
    }

    fn about_dialog(&mut self, ctx: &egui::Context) {
        if !self.view.about {
            return;
        }
        let mut close = false;
        if self.icon.is_none() {
            if let Ok(img) = image::load_from_memory(ICON_PNG) {
                let rgba = img.to_rgba8();
                let size = [rgba.width() as usize, rgba.height() as usize];
                let pixels = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
                self.icon =
                    Some(ctx.load_texture("app-icon", pixels, egui::TextureOptions::LINEAR));
            }
        }
        let icon = self.icon.clone();
        egui::Modal::new(egui::Id::unique("about")).show(ctx, |ui| {
            let pal = palette(ui);
            ui.set_width(380.0);
            ui.vertical_centered(|ui| {
                if let Some(icon) = &icon {
                    ui.add(egui::Image::new(icon).fit_to_exact_size(vec2(112.0, 112.0)));
                }
                ui.label(theme::title(APP_NAME, 20.0));
                ui.label(
                    RichText::new(format!("Version {}", env!("CARGO_PKG_VERSION"))).color(pal.weak),
                );
                ui.add_space(8.0);
                ui.label("A scene and tone editor for the Roland Aerophone Pro AE-30.");
                ui.label(RichText::new("Copyright © 2027 Eric Dannewitz").color(pal.weak));
                ui.add_space(10.0);
                ui.hyperlink_to(
                    "☕ Support this project: buy me a coffee",
                    "https://buymeacoffee.com/ericdannewitz",
                );
                ui.add_space(10.0);
                ui.label(
                    RichText::new(
                        "Parameter data from the aerophoneCompanion project (MIT), built from \
                         Roland's published MIDI Implementation and Parameter Guide. \
                         Inter font, SIL Open Font License. Not affiliated with Roland.",
                    )
                    .size(12.0)
                    .color(pal.weak),
                );
                ui.add_space(10.0);
                if ui.add(theme::primary(ui, "OK")).clicked() {
                    close = true;
                }
            });
        });
        if close {
            self.view.about = false;
        }
    }

    fn save_scene_dialog(&mut self, ctx: &egui::Context) {
        let Some(st) = &mut self.view.save_scene else {
            return;
        };
        let mut close = false;
        let mut save = None;
        egui::Modal::new(egui::Id::unique("save_scene")).show(ctx, |ui| {
            let pal = palette(ui);
            ui.set_width(420.0);
            ui.label(theme::title("Save Scene", 19.0));
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label("Name");
                ui.add(
                    egui::TextEdit::singleline(&mut st.name)
                        .char_limit(16)
                        .desired_width(240.0),
                );
            });
            ui.add_space(6.0);
            ui.label(theme::section(ui, "User slot"));
            egui::ScrollArea::vertical()
                .max_height(300.0)
                .auto_shrink([false, false])
                .show_rows(ui, 22.0, 600, |ui, range| {
                    for n in range {
                        let n = n as u16;
                        let name = &self.ed.user_names[usize::from(n)];
                        let empty = name.is_empty() || name == "INIT SCENE";
                        let label = format!("U{:02}-{:02}", n / 50 + 1, n % 50 + 1);
                        let shown = if empty { "(empty)" } else { name.as_str() };
                        let r = lib_row(ui, &label, shown, st.slot == n, empty);
                        if r.clicked() {
                            st.slot = n;
                        }
                        if st.slot == n && ui.ctx().input(|i| i.pointer.any_click()) {
                            // keep selection visible
                        }
                    }
                });
            let target = &self.ed.user_names[usize::from(st.slot)];
            let overwrite = !(target.is_empty() || target == "INIT SCENE");
            if overwrite {
                ui.label(
                    RichText::new(format!("This replaces \"{target}\" on the AE-30."))
                        .color(pal.amber),
                );
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let label = format!("Save to U{:02}-{:02}", st.slot / 50 + 1, st.slot % 50 + 1);
                if ui.add(theme::primary(ui, &label)).clicked() {
                    save = Some((st.slot, st.name.clone()));
                }
                if ui.button("Cancel").clicked() {
                    close = true;
                }
            });
        });
        if let Some((slot, name)) = save {
            self.ed.save_scene(slot, &name);
            close = true;
        }
        if close {
            self.view.save_scene = None;
        }
    }

    fn save_tone_dialog(&mut self, ctx: &egui::Context) {
        let Some(st) = &mut self.view.save_tone else {
            return;
        };
        let mut close = false;
        let mut save = None;
        egui::Modal::new(egui::Id::unique("save_tone")).show(ctx, |ui| {
            let pal = palette(ui);
            ui.set_width(420.0);
            ui.label(theme::title(format!("Save Part {}'s Tone", st.part), 19.0));
            ui.label(
                RichText::new("The tone goes into a User Tone slot and the part is pointed at it. Save the scene too, to keep that link.")
                    .size(12.0)
                    .color(pal.weak),
            );
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label("Name");
                ui.add(egui::TextEdit::singleline(&mut st.name).char_limit(16).desired_width(240.0));
            });
            ui.add_space(6.0);
            match &self.ed.user_tones {
                None => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Reading the user tone list…");
                    });
                }
                Some(names) => {
                    egui::ScrollArea::vertical()
                        .max_height(300.0)
                        .auto_shrink([false, false])
                        .show_rows(ui, 22.0, names.len(), |ui, range| {
                            for i in range {
                                let n = &names[i];
                                let empty = n.is_empty() || n == "INIT TONE";
                                let label = format!("{:03}", i + 1);
                                if lib_row(ui, &label, if empty { "(empty)" } else { n }, st.slot == i as u16, empty)
                                    .clicked()
                                {
                                    st.slot = i as u16;
                                }
                            }
                        });
                    let n = &names[usize::from(st.slot)];
                    if !(n.is_empty() || n == "INIT TONE") {
                        ui.label(RichText::new(format!("This replaces \"{n}\".")).color(pal.amber));
                    }
                }
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let ready = self.ed.user_tones.is_some();
                if ui
                    .add_enabled(ready, theme::primary(ui, &format!("Save to User Tone {}", st.slot + 1)))
                    .clicked()
                {
                    save = Some((st.part, st.slot, st.name.clone()));
                }
                if ui.button("Cancel").clicked() {
                    close = true;
                }
            });
        });
        if let Some((part, slot, name)) = save {
            self.ed.save_tone(part, slot, &name);
            close = true;
        }
        if close {
            self.view.save_tone = None;
        }
    }
}

impl App {
    fn screenshot(&mut self, ctx: &egui::Context) {
        if self.shot.as_ref().is_some_and(|s| !s.2) {
            if let Ok(f) = std::env::var("AE30_RESTORE") {
                if self.view.restore.is_none() && self.ed.scene.is_loaded() {
                    self.open_restore(std::path::Path::new(&f));
                }
            }
        }
        let Some((path, at, requested)) = &mut self.shot else {
            return;
        };
        if !*requested {
            if let Ok(tab) = std::env::var("AE30_TAB") {
                self.view.tab = match tab.as_str() {
                    "part" => Tab::Part,
                    "effects" => Tab::Effects,
                    "controllers" => Tab::Controllers,
                    "tone" => Tab::Tone,
                    "system" => Tab::System,
                    "live" => Tab::Live,
                    _ => Tab::Scene,
                };
            }
            match std::env::var("AE30_DIALOG").as_deref() {
                Ok("prefs") => self.view.prefs = true,
                Ok("about") => self.view.about = true,
                Ok("select") => {
                    self.view.pl_select = true;
                    self.view.pl_list = self.ed.playlist.current;
                    self.view.pl_selected.insert(0);
                }
                Ok("setlist") => self.view.pl_name = Some((None, String::new())),
                _ => {}
            }
            if let Ok(t) = std::env::var("AE30_PICKER") {
                self.view.tone_picker = t.parse().ok();
            }
            let ready = !self.ed.is_loading() && self.ed.scene.is_loaded();
            if (ready && at.elapsed() > Duration::from_secs(3))
                || at.elapsed() > Duration::from_secs(20)
            {
                *requested = true;
                let path = path.clone();
                let c = ctx.clone();
                ctx.request_screenshot(move |img| {
                    let [w, h] = img.size;
                    let bytes: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
                    if let Some(buf) = image::RgbaImage::from_raw(w as u32, h as u32, bytes) {
                        let _ = buf.save(&path);
                    }
                    c.send_viewport_cmd(egui::ViewportCommand::Close);
                });
            }
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }
}

/// A round colour choice; returns true when clicked.
fn accent_swatch(ui: &mut Ui, name: &str, colour: Color32, selected: bool) -> bool {
    let size = ui.spacing().interact_size.y;
    let (rect, r) = ui.allocate_exact_size(vec2(size, size), Sense::click());
    let painter = ui.painter();
    if selected {
        painter.circle_stroke(
            rect.center(),
            size * 0.5,
            egui::Stroke::new(2.0, ui.visuals().text_color()),
        );
    }
    let grow = if r.hovered() { 1.0 } else { 0.0 };
    painter.circle_filled(rect.center(), size * 0.36 + grow, colour);
    r.on_hover_text(name).clicked()
}

/// A list row: a short tag and a name.
fn lib_row(ui: &mut Ui, tag: &str, name: &str, selected: bool, dim: bool) -> egui::Response {
    lib_row_w(ui, tag, name, selected, dim, 70.0)
}

fn lib_row_w(
    ui: &mut Ui,
    tag: &str,
    name: &str,
    selected: bool,
    dim: bool,
    tag_w: f32,
) -> egui::Response {
    let pal = palette(ui);
    let (rect, r) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::click());
    let bg = if selected {
        pal.accent_soft
    } else if r.hovered() {
        pal.hover
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, 5.0, bg);
    let font = egui::FontId::proportional(12.5);
    ui.painter().text(
        rect.left_center() + vec2(8.0, 0.0),
        egui::Align2::LEFT_CENTER,
        tag,
        egui::FontId::proportional(11.0),
        if selected { pal.accent } else { pal.weak },
    );
    ui.painter().text(
        rect.left_center() + vec2(tag_w, 0.0),
        egui::Align2::LEFT_CENTER,
        name,
        font,
        if dim { pal.weak } else { pal.text },
    );
    r
}

/// A playlist row: number, name, and where it plays from.
fn playlist_row(
    ui: &mut Ui,
    num: usize,
    name: &str,
    place: &str,
    selected: bool,
    pending: bool,
    ticked: Option<bool>,
) -> egui::Response {
    let pal = palette(ui);
    let (rect, r) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::click());
    let bg = if selected {
        pal.accent_soft
    } else if r.hovered() {
        pal.hover
    } else {
        Color32::TRANSPARENT
    };
    let p = ui.painter();
    p.rect_filled(rect, 6.0, bg);
    // In select mode, a tick box replaces the number.
    if let Some(on) = ticked {
        let b =
            egui::Rect::from_center_size(rect.left_center() + vec2(16.0, 0.0), vec2(15.0, 15.0));
        if on {
            p.rect_filled(b, 3.0, pal.accent);
            p.line(
                vec![
                    b.left_center() + vec2(3.0, 0.0),
                    b.center_bottom() + vec2(-1.0, -4.0),
                    b.right_top() + vec2(-3.0, 4.0),
                ],
                egui::Stroke::new(2.0, pal.on_accent),
            );
        } else {
            p.rect_stroke(
                b,
                3.0,
                egui::Stroke::new(1.5, pal.weak),
                egui::StrokeKind::Inside,
            );
        }
    } else {
        p.text(
            rect.left_center() + vec2(10.0, 0.0),
            egui::Align2::LEFT_CENTER,
            num.to_string(),
            egui::FontId::proportional(12.0),
            if selected { pal.accent } else { pal.weak },
        );
    }
    p.text(
        rect.right_center() - vec2(8.0, 0.0),
        egui::Align2::RIGHT_CENTER,
        place,
        egui::FontId::proportional(11.0),
        if pending { pal.amber } else { pal.weak },
    );
    let font = egui::FontId::new(14.0, theme::semibold());
    p.text(
        rect.left_center() + vec2(34.0, 0.0),
        egui::Align2::LEFT_CENTER,
        name,
        font,
        pal.text,
    );
    r
}

fn note_name(n: u8) -> String {
    const NAMES: [&str; 12] = [
        "C", "C♯", "D", "E♭", "E", "F", "F♯", "G", "A♭", "A", "B♭", "B",
    ];
    format!("{}{}", NAMES[usize::from(n % 12)], i32::from(n / 12) - 1)
}

fn safe_file_name(s: &str) -> String {
    let s: String = s
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || " -_".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    let s = s.trim().to_string();
    if s.is_empty() {
        "Scene".into()
    } else {
        s
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if !self.started {
            self.started = true;
            theme::install(&ctx, self.prefs.look);
            self.prefs.theme.apply(&ctx);
            self.apply_knob_speed();
            self.connect(&ctx);
        }
        if let Some(wait) = self.ed.tick() {
            ctx.request_repaint_after(wait.max(Duration::from_millis(1)));
        }
        self.auto_connect(&ctx);
        if self.ed.is_loading()
            || self.ed.breath.1.elapsed() < Duration::from_secs(3)
            || self.ed.bend.1.elapsed() < Duration::from_secs(3)
        {
            ctx.request_repaint_after(Duration::from_millis(40));
        }
        self.handle_commands(&ctx);

        let pal = if ctx.global_style().visuals.dark_mode {
            theme::DARK
        } else {
            theme::LIGHT
        };
        let bar = egui::Frame::new()
            .fill(pal.surface)
            .stroke(egui::Stroke::new(1.0, pal.border))
            .inner_margin(egui::Margin::symmetric(14, 8));
        // On macOS the top bar is also the title bar: align its first row
        // with the window buttons.
        let top = if cfg!(target_os = "macos") {
            bar.inner_margin(egui::Margin {
                left: 14,
                right: 14,
                top: 4,
                bottom: 8,
            })
        } else {
            bar
        };
        egui::Panel::top("top")
            .frame(top)
            .show(ui, |ui| self.top_bar(ui));
        egui::Panel::bottom("status")
            .frame(bar.inner_margin(egui::Margin::symmetric(14, 5)))
            .show(ui, |ui| self.status_bar(ui));
        if self.ed.is_online() {
            egui::Panel::left("library")
                .frame(bar.inner_margin(egui::Margin::symmetric(10, 6)))
                .resizable(true)
                .default_size(250.0)
                .min_size(200.0)
                .show(ui, |ui| self.library(ui));
        }
        if self.ed.is_online() && self.view.tab == Tab::Live {
            self.playlist_keys(&ctx);
            egui::Panel::right("playlist")
                .frame(bar.inner_margin(egui::Margin::symmetric(10, 6)))
                .resizable(true)
                .default_size(280.0)
                .min_size(220.0)
                .show(ui, |ui| self.playlist(ui));
        }
        let page = egui::Frame::new()
            .fill(pal.bg)
            .inner_margin(egui::Margin::same(16));
        egui::CentralPanel::default().frame(page).show(ui, |ui| {
            if !self.ed.is_online() {
                return self.welcome(ui);
            }
            egui::ScrollArea::vertical()
                .id_salt(("page", self.view.tab as u8))
                .auto_shrink([false, false])
                .show(ui, |ui| match self.view.tab {
                    Tab::Scene => pages::scene(ui, &mut self.view, &mut self.ed),
                    Tab::Part => pages::part(ui, &mut self.view, &mut self.ed),
                    Tab::Effects => pages::effects(ui, &mut self.view, &mut self.ed),
                    Tab::Controllers => pages::controllers(ui, &mut self.view, &mut self.ed),
                    Tab::Tone => pages::tone(ui, &mut self.view, &mut self.ed),
                    Tab::System => pages::system(ui, &mut self.ed),
                    Tab::Live => pages::live(ui, &mut self.ed),
                });
        });
        self.tone_picker(&ctx);
        self.save_scene_dialog(&ctx);
        self.backup_dialog(&ctx);
        self.restore_dialog(&ctx);
        self.save_tone_dialog(&ctx);
        self.setlist_dialogs(&ctx);
        self.about_dialog(&ctx);
        self.preferences_window(&ctx);
        self.screenshot(&ctx);
        #[cfg(target_os = "macos")]
        {
            let state = self.menu_state();
            if let Some(menu) = &mut self.native_menu {
                menu.sync(&state);
            }
        }
        // Send anything the frame's edits queued without waiting a frame.
        if let Some(wait) = self.ed.tick() {
            ctx.request_repaint_after(wait.max(Duration::from_millis(1)));
        }
    }
}
