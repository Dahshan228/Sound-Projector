use super::{CaptureTarget, ERROR, MUTED, hotkey_field, semibold};
use crate::engine::View;
use crate::hotkeys::HotkeyAction;
use crate::settings::{Hotkey, Settings};
use eframe::egui;

pub enum SettingsAction {
    None,
    Changed,
    DevicesChanged,
    Capture(CaptureTarget),
    AutostartChanged,
}

const SETUP_STEPS: [&str; 6] = [
    "Install VB-Cable, then reboot.",
    "In Discord, go to Voice & Video and set Input Device to CABLE Output.",
    "For the cleanest sound, turn off Discord's Noise Suppression (Krisp), Echo Cancellation, and Automatic Gain Control. If they stay on, Discord's processing squashes music no matter how clean the app's output is.",
    "Use Voice Activity mode. With Push to Talk, friends only hear songs while the key is held.",
    "A server admin can raise the voice channel's bitrate (Edit Channel), which makes a large difference for music.",
    "Do the same for in-game voice chat, using CABLE Output as the mic.",
];

pub fn show(
    ctx: &egui::Context,
    open: &mut bool,
    s: &mut Settings,
    view: &View,
    devices: &(Vec<String>, Vec<String>),
    failed: &[HotkeyAction],
) -> SettingsAction {
    let mut action = SettingsAction::None;
    egui::Window::new("Settings")
        .open(open)
        .collapsible(false)
        .resizable(false)
        .default_width(440.0)
        .pivot(egui::Align2::CENTER_TOP)
        .default_pos(ctx.content_rect().center_top() + egui::vec2(0.0, 40.0))
        .show(ctx, |ui| {
        ui.label(egui::RichText::new("Devices").font(semibold(14.0)));
        egui::Grid::new("devices").num_columns(2).spacing(egui::vec2(12.0, 6.0)).show(ui, |ui| {
            for (label, value, names, none_label, status) in [
                ("Mic", &mut s.mic_device, &devices.0, "Default", &view.mic),
                ("Friends output", &mut s.friends_device, &devices.1, "Auto (VB-Cable)", &view.friends),
                ("You hear output", &mut s.you_device, &devices.1, "Default", &view.you),
            ] {
                ui.label(egui::RichText::new(label).color(MUTED));
                ui.vertical(|ui| {
                    let shown = value.clone().unwrap_or_else(|| none_label.to_string());
                    egui::ComboBox::from_id_salt(label).selected_text(shown).width(280.0).show_ui(ui, |ui| {
                        if ui.selectable_label(value.is_none(), none_label).clicked() {
                            *value = None;
                            action = SettingsAction::DevicesChanged;
                        }
                        for n in names {
                            if ui.selectable_label(value.as_deref() == Some(n), n).clicked() {
                                *value = Some(n.clone());
                                action = SettingsAction::DevicesChanged;
                            }
                        }
                    });
                    match status {
                        Ok(name) => ui.label(egui::RichText::new(format!("Using {name}")).color(MUTED).small()),
                        Err(e) => ui.label(egui::RichText::new(e).color(ERROR).small()),
                    };
                });
                ui.end_row();
            }
        });

        ui.add_space(8.0);
        ui.label(egui::RichText::new("Control hotkeys").font(semibold(14.0)));
        egui::Grid::new("control-keys").num_columns(2).spacing(egui::vec2(12.0, 6.0)).show(ui, |ui| {
            let rows: [(&str, CaptureTarget, HotkeyAction, &mut Option<Hotkey>); 3] = [
                ("Pause/Resume", CaptureTarget::Pause, HotkeyAction::Pause, &mut s.pause_key),
                ("Skip", CaptureTarget::Skip, HotkeyAction::Skip, &mut s.skip_key),
                ("Stop", CaptureTarget::Stop, HotkeyAction::Stop, &mut s.stop_key),
            ];
            for (label, target, act, key) in rows {
                ui.label(egui::RichText::new(label).color(MUTED));
                let r = hotkey_field(ui, *key, failed.contains(&act));
                if r.clicked() {
                    action = SettingsAction::Capture(target);
                }
                if r.secondary_clicked() {
                    *key = None;
                    action = SettingsAction::Changed;
                }
                ui.end_row();
            }
        });

        ui.add_space(8.0);
        if ui.checkbox(&mut s.start_with_windows, "Start with Windows").changed() {
            action = SettingsAction::AutostartChanged;
        }

        ui.add_space(8.0);
        egui::CollapsingHeader::new("Setup guide").default_open(view.friends.is_err()).show(ui, |ui| {
            for (i, step) in SETUP_STEPS.iter().enumerate() {
                ui.horizontal_wrapped(|ui| {
                    ui.label(egui::RichText::new(format!("{}.", i + 1)).color(MUTED));
                    ui.label(*step);
                });
            }
            ui.hyperlink_to("Get VB-Cable (vb-audio.com)", "https://vb-audio.com/Cable/");
        });
    });
    action
}
