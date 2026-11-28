// No console window for the editor on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::process::ExitCode;

use aerophone_app::{cli, ui};
use eframe::egui;

fn main() -> ExitCode {
    // Windows release builds have no console of their own; the command-line
    // tools borrow the one they were started from.
    #[cfg(windows)]
    if std::env::args().len() > 1 {
        use windows_sys::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
        // SAFETY: no pointers; fails harmlessly without a console.
        unsafe { AttachConsole(ATTACH_PARENT_PROCESS) };
    }
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let port = take_option(&mut args, "--port");
    let result = match args.first().map(String::as_str) {
        // Used by the build scripts to check a packaged app starts.
        Some("--smoke") => {
            println!("{} {} started", ui::APP_NAME, env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some("-h" | "--help" | "help") => {
            println!("{}", cli::USAGE);
            Ok(())
        }
        None => return gui(),
        Some(cmd) => cli::run(cmd, &args[1..], port.as_deref()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn take_option(args: &mut Vec<String>, name: &str) -> Option<String> {
    let i = args.iter().position(|a| a == name)?;
    args.remove(i);
    (i < args.len()).then(|| args.remove(i))
}

/// The window size: 1320×880, or `AE30_WINDOW=WxH` (screenshots, layouts).
fn window_size() -> [f32; 2] {
    std::env::var("AE30_WINDOW")
        .ok()
        .and_then(|v| {
            let (w, h) = v.split_once('x')?;
            Some([w.trim().parse().ok()?, h.trim().parse().ok()?])
        })
        .unwrap_or([1320.0, 880.0])
}

fn gui() -> ExitCode {
    let mut viewport = egui::ViewportBuilder::default()
        .with_title(ui::APP_NAME)
        .with_app_id("ae30-patch-editor")
        .with_inner_size(window_size())
        // Small windows get the compact layout.
        .with_min_inner_size([360.0, 360.0]);
    if let Ok(icon) = eframe::icon_data::from_png_bytes(ui::ICON_PNG) {
        viewport = viewport.with_icon(icon);
    }
    // macOS: one unified toolbar, as in Finder or Logic — the content runs
    // under a transparent title bar and the window buttons sit on the top bar.
    if cfg!(target_os = "macos") {
        viewport = viewport
            .with_fullsize_content_view(true)
            .with_titlebar_shown(false)
            .with_title_shown(false);
    }
    let result = eframe::run_native(
        ui::APP_NAME,
        eframe::NativeOptions {
            viewport,
            ..Default::default()
        },
        Box::new(|cc| {
            #[allow(unused_mut)]
            let mut app = ui::App::new();
            #[cfg(target_os = "macos")]
            app.install_native_menu(&cc.egui_ctx);
            let _ = cc;
            Ok(Box::new(app))
        }),
    );
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
