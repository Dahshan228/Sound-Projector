#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod decode;
mod dsp;
mod library;
mod queue;
mod settings;
#[cfg(test)]
mod testutil;
mod voice;

use eframe::egui;

struct App;

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default()
            .frame(egui::Frame::central_panel(ui.style()).fill(egui::Color32::from_rgb(0x11, 0x12, 0x16)))
            .show(ui, |_ui| {});
    }
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        persist_window: true,
        viewport: egui::ViewportBuilder::default()
            .with_title("Sound Projector")
            .with_inner_size([820.0, 560.0])
            .with_min_inner_size([560.0, 380.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native("Sound Projector", options, Box::new(|_cc| Ok(Box::new(App))))
}
