//! Groundwork desktop editor.

// No console window behind the app in release builds on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod canvas;
mod dialogs;
mod theme;
mod tools;
mod widgets;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Groundwork")
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([960.0, 600.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Groundwork",
        options,
        Box::new(|cc| {
            let file = std::env::args_os().nth(1).map(std::path::PathBuf::from);
            Ok(Box::new(app::App::new(cc, file)))
        }),
    )
}
