#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod decode;
mod dsp;
mod engine;
mod hotkeys;
mod icon;
mod library;
mod net;
mod queue;
mod settings;
#[cfg(test)]
mod testutil;
mod tray;
mod ui;
mod updater;
mod voice;
mod youtube;

use eframe::egui;

fn main() -> eframe::Result {
    // The installer asks this before installing VB-Cable: exit code 0 means it's already there.
    if std::env::args().any(|a| a == "--has-vbcable") {
        let (_, outs) = engine::list_devices();
        std::process::exit(if outs.iter().any(|n| n.contains(engine::CABLE)) { 0 } else { 1 });
    }
    if tray::already_running() {
        if let Some(h) = tray::find_window() {
            tray::show_window(h);
        }
        return Ok(());
    }
    let (settings, notice) = match settings::Settings::load(&settings::settings_path()) {
        Ok(x) => x,
        Err(e) => {
            // Starting with defaults would save over the user's library.
            rfd::MessageDialog::new()
                .set_level(rfd::MessageLevel::Error)
                .set_title("Sound Projector")
                .set_description(format!("Couldn't read your settings file: {e}\n\nClose anything that might have it open, then start Sound Projector again."))
                .show();
            return Ok(());
        }
    };
    let update_failed = std::env::args().any(|a| a == "--update-failed").then(|| {
        format!("The update didn't finish, so you're still on {}. Your songs and settings are fine.", env!("CARGO_PKG_VERSION"))
    });
    let notice = notice.or(update_failed);
    let options = eframe::NativeOptions {
        persist_window: true,
        viewport: egui::ViewportBuilder::default()
            .with_title("Sound Projector")
            .with_inner_size([820.0, 560.0])
            .with_min_inner_size([560.0, 380.0])
            .with_drag_and_drop(true)
            .with_icon({
                egui::IconData { rgba: icon::icon_rgba(64), width: 64, height: 64 }
            }),
        ..Default::default()
    };
    eframe::run_native("Sound Projector", options, Box::new(move |cc| Ok(Box::new(ui::App::new(cc, settings, notice)))))
}
