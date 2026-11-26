//! The native macOS menu bar (via muda). Clicks arrive on AppKit's side and
//! are forwarded as [`Command`]s through a channel; the window is woken to
//! handle them. Each frame [`NativeMenu::sync`] updates enabled items and
//! checkmarks.
//!
//! ⌘C/⌘V/⌘X are deliberately not bound: a menu item would take those keys
//! from the text fields. ⌘Z is bound to the editor's own undo, which also
//! covers renaming.

use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver};

use eframe::egui;
use muda::accelerator::{Accelerator, Code, Modifiers};
use muda::{CheckMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu};

use super::commands::{Command, MenuState, ThemeChoice, TABS};
use super::APP_NAME;

pub struct NativeMenu {
    _menu: Menu,
    commands: HashMap<MenuId, Command>,
    received: Receiver<MenuId>,
    needs_online: Vec<MenuItem>,
    needs_scene: Vec<MenuItem>,
    undo: MenuItem,
    redo: MenuItem,
    backup: MenuItem,
    restore: MenuItem,
    connect: MenuItem,
    themes: Vec<(ThemeChoice, CheckMenuItem)>,
    tabs: Vec<(super::Tab, CheckMenuItem)>,
    shown: Option<MenuState>,
}

fn key(code: Code) -> Option<Accelerator> {
    Some(Accelerator::new(Modifiers::META, code))
}

fn shift_key(code: Code) -> Option<Accelerator> {
    Some(Accelerator::new(Modifiers::META | Modifiers::SHIFT, code))
}

impl NativeMenu {
    /// Build and install the menu bar. Call on the main thread once the
    /// app has started (eframe's creation callback).
    pub fn install(ctx: &egui::Context) -> Self {
        let mut commands = HashMap::new();
        let mut item = |text: &str, cmd: Command, accel: Option<Accelerator>| {
            let i = MenuItem::new(text, true, accel);
            commands.insert(i.id().clone(), cmd);
            i
        };
        let about = item(&format!("About {APP_NAME}"), Command::About, None);
        let settings = item("Settings…", Command::Preferences, key(Code::Comma));

        let save = item("Save to User Scene…", Command::SaveScene, key(Code::KeyS));
        let import = item("Import Scene…", Command::Import, key(Code::KeyO));
        let export = item("Export Scene…", Command::Export, key(Code::KeyE));
        let backup = item("Back Up User Scenes…", Command::Backup, None);
        let restore = item("Restore Scenes…", Command::Restore, None);

        let undo = item("Undo", Command::Undo, key(Code::KeyZ));
        let redo = item("Redo", Command::Redo, shift_key(Code::KeyZ));

        let prev = item("Previous Scene", Command::PrevScene, key(Code::BracketLeft));
        let next = item("Next Scene", Command::NextScene, key(Code::BracketRight));
        let reload = item("Reload from AE-30", Command::Reload, key(Code::KeyR));
        let connect = item("Connect", Command::Reconnect, None);
        let disconnect = item("Disconnect", Command::Disconnect, None);

        let support = item("Roland AE-30 Manuals", Command::RolandSupport, None);

        let digit = [
            Code::Digit1,
            Code::Digit2,
            Code::Digit3,
            Code::Digit4,
            Code::Digit5,
            Code::Digit6,
        ];
        let mut tabs = Vec::new();
        let view_menu = Submenu::new("View", true);
        for (i, &(t, label, _)) in TABS.iter().enumerate() {
            let c = CheckMenuItem::new(label, true, false, key(digit[i]));
            commands.insert(c.id().clone(), Command::Tab(t));
            let _ = view_menu.append(&c);
            tabs.push((t, c));
        }
        let theme_menu = Submenu::new("Appearance", true);
        let mut themes = Vec::new();
        for (t, label) in ThemeChoice::ALL {
            let c = CheckMenuItem::new(label, true, false, None);
            commands.insert(c.id().clone(), Command::Theme(t));
            let _ = theme_menu.append(&c);
            themes.push((t, c));
        }

        let sep = PredefinedMenuItem::separator;
        let app_menu = Submenu::new(APP_NAME, true);
        let _ = app_menu.append_items(&[
            &about,
            &sep(),
            &settings,
            &sep(),
            &PredefinedMenuItem::services(None),
            &sep(),
            &PredefinedMenuItem::hide(None),
            &PredefinedMenuItem::hide_others(None),
            &PredefinedMenuItem::show_all(None),
            &sep(),
            &PredefinedMenuItem::quit(None),
        ]);
        let file_menu = Submenu::new("File", true);
        let _ = file_menu.append_items(&[
            &save,
            &sep(),
            &import,
            &export,
            &sep(),
            &backup,
            &restore,
            &sep(),
            &PredefinedMenuItem::close_window(None),
        ]);
        let edit_menu = Submenu::new("Edit", true);
        let _ = edit_menu.append_items(&[&undo, &redo]);
        let _ = view_menu.append_items(&[
            &sep(),
            &theme_menu,
            &sep(),
            &PredefinedMenuItem::fullscreen(None),
        ]);
        let scene_menu = Submenu::new("Scene", true);
        let _ = scene_menu.append_items(&[&prev, &next, &sep(), &reload]);
        let device_menu = Submenu::new("AE-30", true);
        let _ = device_menu.append_items(&[&connect, &disconnect]);
        let window_menu = Submenu::new("Window", true);
        let _ = window_menu.append_items(&[
            &PredefinedMenuItem::minimize(None),
            &PredefinedMenuItem::maximize(None),
            &sep(),
            &PredefinedMenuItem::bring_all_to_front(None),
        ]);
        let help_menu = Submenu::new("Help", true);
        let _ = help_menu.append(&support);

        let menu = Menu::new();
        let _ = menu.append_items(&[
            &app_menu,
            &file_menu,
            &edit_menu,
            &view_menu,
            &scene_menu,
            &device_menu,
            &window_menu,
            &help_menu,
        ]);
        menu.init_for_nsapp();
        window_menu.set_as_windows_menu_for_nsapp();
        help_menu.set_as_help_menu_for_nsapp();

        let (tx, received) = mpsc::channel();
        let ctx = ctx.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            let _ = tx.send(event.id);
            ctx.request_repaint();
        }));

        NativeMenu {
            _menu: menu,
            commands,
            received,
            needs_online: vec![import, reload, disconnect],
            needs_scene: vec![save, export, prev, next],
            undo,
            redo,
            backup,
            restore,
            connect,
            themes,
            tabs,
            shown: None,
        }
    }

    /// Menu clicks since the last call.
    pub fn take_commands(&mut self) -> Vec<Command> {
        let cmds: Vec<Command> = self
            .received
            .try_iter()
            .filter_map(|id| self.commands.get(&id).copied())
            .collect();
        if !cmds.is_empty() {
            // Clicking a check item flips it; put the checkmarks right.
            self.shown = None;
        }
        cmds
    }

    pub fn sync(&mut self, s: &MenuState) {
        if self.shown.as_ref() == Some(s) {
            return;
        }
        for i in &self.needs_online {
            i.set_enabled(s.online);
        }
        for i in &self.needs_scene {
            i.set_enabled(s.loaded);
        }
        self.undo.set_enabled(s.can_undo);
        self.redo.set_enabled(s.can_redo);
        self.backup.set_enabled(s.online && !s.backing_up);
        self.restore.set_enabled(s.online && !s.backing_up);
        self.connect
            .set_text(if s.online { "Reconnect" } else { "Connect" });
        for (t, c) in &self.themes {
            c.set_checked(*t == s.theme);
        }
        for (t, c) in &self.tabs {
            c.set_checked(*t == s.tab);
        }
        self.shown = Some(s.clone());
    }
}
