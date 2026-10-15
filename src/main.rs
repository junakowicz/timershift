// Release builds on Windows shouldn't open a console window next to the app.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod engine;
mod icon;
mod notify;
mod storage;
mod tray;

use std::path::Path;

use eframe::egui;

fn main() -> eframe::Result {
    // Used by scripts/bundle-macos.sh to produce the .app icon.
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let [flag, path] = args.as_slice()
        && flag == "--write-icon"
    {
        if let Err(err) = icon::write_png(Path::new(path), 1024) {
            eprintln!("timershift: could not write icon: {err}");
            std::process::exit(1);
        }
        return Ok(());
    }

    notify::init();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("TimerShift")
            .with_inner_size([340.0, 520.0])
            .with_min_inner_size([310.0, 470.0])
            .with_icon(icon::icon_data(256)),
        ..Default::default()
    };
    eframe::run_native(
        "TimerShift",
        options,
        Box::new(|cc| Ok(Box::new(app::TimerApp::new(cc)))),
    )
}
