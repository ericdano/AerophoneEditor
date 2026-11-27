//! Everything the menus can do, in one place, so the native macOS menu bar
//! and the in-window menu (Windows, Linux) offer the same commands.

use eframe::egui::{self, Key, KeyboardShortcut, Modifiers};

use super::{App, Tab};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ThemeChoice {
    System,
    Light,
    Dark,
}

impl ThemeChoice {
    pub const ALL: [(ThemeChoice, &'static str); 3] = [
        (ThemeChoice::System, "Match System"),
        (ThemeChoice::Light, "Light"),
        (ThemeChoice::Dark, "Dark"),
    ];

    pub fn key(self) -> &'static str {
        match self {
            ThemeChoice::System => "system",
            ThemeChoice::Light => "light",
            ThemeChoice::Dark => "dark",
        }
    }

    /// Apply to egui and to the native window (title bar, dialogs, scroll bars).
    pub fn apply(self, ctx: &egui::Context) {
        let (pref, sys) = match self {
            ThemeChoice::System => (
                egui::ThemePreference::System,
                egui::SystemTheme::SystemDefault,
            ),
            ThemeChoice::Light => (egui::ThemePreference::Light, egui::SystemTheme::Light),
            ThemeChoice::Dark => (egui::ThemePreference::Dark, egui::SystemTheme::Dark),
        };
        ctx.set_theme(pref);
        ctx.send_viewport_cmd(egui::ViewportCommand::SetTheme(sys));
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Command {
    About,
    SaveScene,
    Import,
    Export,
    Backup,
    Restore,
    Undo,
    Redo,
    PrevScene,
    NextScene,
    Reload,
    Reconnect,
    Disconnect,
    Tab(Tab),
    Theme(ThemeChoice),
    Preferences,
    RolandSupport,
}

pub const TABS: [(Tab, &str, Key); 7] = [
    (Tab::Live, "Live", Key::Num1),
    (Tab::Scene, "Scene", Key::Num2),
    (Tab::Part, "Parts", Key::Num3),
    (Tab::Effects, "Effects", Key::Num4),
    (Tab::Controllers, "Controllers", Key::Num5),
    (Tab::Tone, "Tone", Key::Num6),
    (Tab::System, "System", Key::Num7),
];

/// Keyboard shortcuts (⌘ on macOS, Ctrl elsewhere). On macOS the menu bar
/// usually sees them first; these cover Windows and Linux.
pub fn shortcuts(ctx: &egui::Context) -> Vec<Command> {
    let cmd = |k| KeyboardShortcut::new(Modifiers::COMMAND, k);
    let shift = |k| KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, k);
    let mut list = vec![
        (shift(Key::Z), Command::Redo),
        (cmd(Key::Y), Command::Redo),
        (cmd(Key::Z), Command::Undo),
        (cmd(Key::S), Command::SaveScene),
        (cmd(Key::O), Command::Import),
        (cmd(Key::E), Command::Export),
        (cmd(Key::R), Command::Reload),
        (cmd(Key::Comma), Command::Preferences),
        (cmd(Key::OpenBracket), Command::PrevScene),
        (cmd(Key::CloseBracket), Command::NextScene),
    ];
    list.extend(TABS.iter().map(|&(t, _, k)| (cmd(k), Command::Tab(t))));
    ctx.input_mut(|i| {
        list.into_iter()
            .filter(|(s, _)| i.consume_shortcut(s))
            .map(|(_, c)| c)
            .collect()
    })
}

/// What the menus need to know to enable items and place checkmarks.
#[derive(Clone, PartialEq, Debug)]
pub struct MenuState {
    pub online: bool,
    pub loaded: bool,
    pub can_undo: bool,
    pub can_redo: bool,
    pub backing_up: bool,
    pub theme: ThemeChoice,
    pub tab: Tab,
}

impl App {
    pub fn menu_state(&self) -> MenuState {
        MenuState {
            online: self.ed.is_online(),
            loaded: self.ed.scene.is_loaded(),
            can_undo: !self.ed.undo.is_empty(),
            can_redo: !self.ed.redo.is_empty(),
            backing_up: self.ed.backup.is_some() || self.ed.restore.is_some(),
            theme: self.prefs.theme,
            tab: self.view.tab,
        }
    }

    pub fn run_command(&mut self, ctx: &egui::Context, c: Command) {
        let loaded = self.ed.scene.is_loaded();
        match c {
            Command::About => self.view.about = true,
            Command::SaveScene => self.open_save(),
            Command::Import if self.ed.is_online() => {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("SysEx", &["syx"])
                    .pick_file()
                {
                    self.ed.import_scene(&path);
                }
            }
            Command::Export if loaded => {
                let name = format!("{}.syx", super::safe_file_name(&self.ed.scene_name()));
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("SysEx", &["syx"])
                    .set_file_name(name)
                    .save_file()
                {
                    self.ed.export_scene(&path);
                }
            }
            Command::Backup if self.ed.is_online() && self.ed.backup.is_none() => {
                self.view.backup_choice = true;
            }
            Command::Restore
                if self.ed.is_online() && self.ed.backup.is_none() && self.ed.restore.is_none() =>
            {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("SysEx", &["syx"])
                    .pick_file()
                {
                    self.open_restore(&path);
                }
            }
            Command::Undo => self.ed.undo(),
            Command::Redo => self.ed.redo(),
            Command::PrevScene if loaded => self.ed.step_scene(-1),
            Command::NextScene if loaded => self.ed.step_scene(1),
            Command::Reload if self.ed.is_online() => self.ed.load_scene(),
            Command::Reconnect => self.connect(ctx),
            Command::Disconnect => self.ed.disconnect(),
            Command::Tab(t) => self.view.tab = t,
            Command::Theme(t) => {
                self.prefs.theme = t;
                t.apply(ctx);
                self.prefs.save();
            }
            Command::Preferences => self.view.prefs = true,
            Command::RolandSupport => ctx.open_url(egui::OpenUrl::new_tab(
                "https://www.roland.com/us/support/by_product/aerophone_pro/owners_manuals/",
            )),
            _ => {}
        }
    }

    /// The in-window menu, for platforms without a native menu bar.
    pub fn menu_contents(&mut self, ui: &mut egui::Ui) {
        let s = self.menu_state();
        let ctx = ui.ctx().clone();
        let mut run = None;
        let key = |k: &str| Some(format!("Ctrl+{k}"));
        let item = |ui: &mut egui::Ui, enabled: bool, text: &str, short: Option<String>| {
            let mut b = egui::Button::new(text);
            if let Some(s) = short {
                b = b.shortcut_text(s);
            }
            let clicked = ui.add_enabled(enabled, b).clicked();
            if clicked {
                ui.close();
            }
            clicked
        };
        let entries: [(bool, &str, Option<String>, Command); 10] = [
            (
                s.loaded,
                "Save to User Scene…",
                key("S"),
                Command::SaveScene,
            ),
            (s.online, "Import Scene…", key("O"), Command::Import),
            (s.loaded, "Export Scene…", key("E"), Command::Export),
            (
                s.online && !s.backing_up,
                "Back Up User Scenes…",
                None,
                Command::Backup,
            ),
            (
                s.online && !s.backing_up,
                "Restore Scenes…",
                None,
                Command::Restore,
            ),
            (s.can_undo, "Undo", key("Z"), Command::Undo),
            (s.can_redo, "Redo", key("Y"), Command::Redo),
            (s.loaded, "Previous Scene", key("["), Command::PrevScene),
            (s.loaded, "Next Scene", key("]"), Command::NextScene),
            (s.online, "Reload from AE-30", key("R"), Command::Reload),
        ];
        for (i, (enabled, text, short, c)) in entries.into_iter().enumerate() {
            if i == 5 || i == 7 {
                ui.separator();
            }
            if item(ui, enabled, text, short) {
                run = Some(c);
            }
        }
        ui.separator();
        ui.menu_button("Appearance", |ui| {
            for (t, label) in ThemeChoice::ALL {
                if ui.radio(s.theme == t, label).clicked() {
                    run = Some(Command::Theme(t));
                    ui.close();
                }
            }
        });
        if item(ui, true, "Preferences…", key(",")) {
            run = Some(Command::Preferences);
        }
        let connect = if s.online { "Reconnect" } else { "Connect" };
        if item(ui, true, connect, None) {
            run = Some(Command::Reconnect);
        }
        if item(ui, s.online, "Disconnect", None) {
            run = Some(Command::Disconnect);
        }
        ui.separator();
        if item(ui, true, "Roland AE-30 Manuals", None) {
            run = Some(Command::RolandSupport);
        }
        if item(ui, true, "About", None) {
            run = Some(Command::About);
        }
        if let Some(c) = run {
            self.run_command(&ctx, c);
        }
    }
}
