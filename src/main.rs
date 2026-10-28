#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod decode;
mod dsp;
mod engine;
mod hotkeys;
mod library;
mod queue;
mod settings;
#[cfg(test)]
mod testutil;
mod ui;
mod voice;

use eframe::egui;

fn main() -> eframe::Result {
    let (settings, notice) = settings::Settings::load(&settings::settings_path());
    let options = eframe::NativeOptions {
        persist_window: true,
        viewport: egui::ViewportBuilder::default()
            .with_title("Sound Projector")
            .with_inner_size([820.0, 560.0])
            .with_min_inner_size([560.0, 380.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native("Sound Projector", options, Box::new(move |cc| Ok(Box::new(ui::App::new(cc, settings, notice)))))
}
