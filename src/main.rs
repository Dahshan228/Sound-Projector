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
mod tray;
mod ui;
mod voice;

use eframe::egui;

fn main() -> eframe::Result {
    if tray::already_running() {
        if let Some(h) = tray::find_window() {
            tray::show_window(h);
        }
        return Ok(());
    }
    let (settings, notice) = settings::Settings::load(&settings::settings_path());
    let options = eframe::NativeOptions {
        persist_window: true,
        viewport: egui::ViewportBuilder::default()
            .with_title("Sound Projector")
            .with_inner_size([820.0, 560.0])
            .with_min_inner_size([560.0, 380.0])
            .with_drag_and_drop(true)
            .with_icon({
                let (rgba, width, height) = tray::icon_rgba();
                egui::IconData { rgba, width, height }
            }),
        ..Default::default()
    };
    eframe::run_native("Sound Projector", options, Box::new(move |cc| Ok(Box::new(ui::App::new(cc, settings, notice)))))
}
