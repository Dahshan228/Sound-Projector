pub mod editor;

use crate::engine::{Cmd, DeviceChoice, Engine, PadPlay, View};
use crate::hotkeys::{Captured, HotkeyAction, HotkeyService, key_name, keyboard_snapshot, poll_capture};
use crate::library::{fmt_time, frames_to_secs, import, is_clip, whole_pad};
use crate::settings::{Hotkey, PadId, Settings, Source, settings_path, sounds_dir};
use eframe::egui::{self, Align2, Color32, CornerRadius, FontFamily, FontId, Id, Rect, Sense, Stroke, StrokeKind, vec2};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

pub const BG: Color32 = Color32::from_rgb(0x11, 0x12, 0x16);
pub const BAR: Color32 = Color32::from_rgb(0x18, 0x19, 0x1e);
pub const PANEL: Color32 = Color32::from_rgb(0x15, 0x16, 0x1b);
pub const PAD: Color32 = Color32::from_rgb(0x1c, 0x1d, 0x23);
pub const BORDER: Color32 = Color32::from_rgb(0x2a, 0x2b, 0x31);
pub const ACCENT: Color32 = Color32::from_rgb(0x64, 0x50, 0xf0);
pub const ACCENT_LIGHT: Color32 = Color32::from_rgb(0x7c, 0x6c, 0xff);
pub const GREEN: Color32 = Color32::from_rgb(0x3e, 0xcf, 0x8e);
pub const TEXT: Color32 = Color32::from_rgb(0xe8, 0xe8, 0xec);
pub const MUTED: Color32 = Color32::from_rgb(0x8e, 0x8e, 0x99);
pub const PLAYING: Color32 = Color32::from_rgb(0x21, 0x1f, 0x33);
pub const ERROR: Color32 = Color32::from_rgb(0xff, 0x8a, 0x8a);
pub const WARN_BG: Color32 = Color32::from_rgb(0x2a, 0x22, 0x14);
pub const WARN: Color32 = Color32::from_rgb(0xf5, 0xc2, 0x7a);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CaptureTarget {
    Pad(PadId),
    Pause,
    Skip,
    Stop,
}

type ImportResult = (PathBuf, PadId, Result<Source, String>);

pub struct App {
    s: Settings,
    engine: Engine,
    tx: Sender<Cmd>,
    hotkeys: HotkeyService,
    capture: Option<(CaptureTarget, [bool; 256])>,
    capture_msg: Option<String>,
    import_tx: Sender<ImportResult>,
    import_rx: Receiver<ImportResult>,
    notice: Option<(String, Instant)>,
    rename: Option<(PadId, String)>,
    confirm_delete: Option<PadId>,
    seek_drag: Option<f32>,
    editor: Option<editor::Editor>,
}

pub fn pad_plays(s: &Settings) -> Vec<PadPlay> {
    let dir = sounds_dir();
    s.pads
        .iter()
        .filter_map(|p| {
            let src = s.sources.iter().find(|x| x.id == p.source)?;
            Some(PadPlay { id: p.id, path: dir.join(&src.file), rate: src.rate, start: p.start, end: p.end, boost_db: p.boost_db })
        })
        .collect()
}

pub fn bindings(s: &Settings) -> Vec<(HotkeyAction, Hotkey)> {
    let mut b: Vec<(HotkeyAction, Hotkey)> = s.pads.iter().filter_map(|p| Some((HotkeyAction::Pad(p.id), p.hotkey?))).collect();
    for (a, k) in [(HotkeyAction::Pause, s.pause_key), (HotkeyAction::Skip, s.skip_key), (HotkeyAction::Stop, s.stop_key)] {
        if let Some(k) = k {
            b.push((a, k));
        }
    }
    b
}

/// Click to set a key, right-click to clear it.
pub fn hotkey_field(ui: &mut egui::Ui, key: Option<Hotkey>, failed: bool) -> egui::Response {
    let text = key.map(key_name).unwrap_or_else(|| "Set key".into());
    let color = if failed { ERROR } else if key.is_some() { TEXT } else { MUTED };
    let r = ui.add(egui::Button::new(egui::RichText::new(text).color(color).monospace()));
    let tip = if failed { "Another app is using this key. Click to pick another." } else { "Click to set a key, right-click to clear" };
    r.on_hover_text(tip)
}

fn setup_style(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let proportional = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let mut semibold = proportional.clone();
    if let Ok(b) = std::fs::read(r"C:\Windows\Fonts\segoeui.ttf") {
        fonts.font_data.insert("segoe".into(), Arc::new(egui::FontData::from_owned(b)));
        fonts.families.entry(FontFamily::Proportional).or_default().insert(0, "segoe".into());
        semibold.insert(0, "segoe".into());
    }
    if let Ok(b) = std::fs::read(r"C:\Windows\Fonts\seguisb.ttf") {
        fonts.font_data.insert("segoe-sb".into(), Arc::new(egui::FontData::from_owned(b)));
        semibold.insert(0, "segoe-sb".into());
    }
    // Segoe UI has no media-control glyphs (⏸ ⏭); Segoe UI Symbol does.
    if let Ok(b) = std::fs::read(r"C:\Windows\Fonts\seguisym.ttf") {
        fonts.font_data.insert("segoe-sym".into(), Arc::new(egui::FontData::from_owned(b)));
        fonts.families.entry(FontFamily::Proportional).or_default().push("segoe-sym".into());
        semibold.push("segoe-sym".into());
    }
    fonts.families.insert(FontFamily::Name("semibold".into()), semibold);
    ctx.set_fonts(fonts);

    let mut v = egui::Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = BAR;
    v.window_stroke = Stroke::new(1.0, BORDER);
    v.extreme_bg_color = PAD;
    v.faint_bg_color = PAD;
    v.override_text_color = Some(TEXT);
    v.hyperlink_color = ACCENT_LIGHT;
    v.selection.bg_fill = ACCENT;
    v.selection.stroke = Stroke::new(1.0, Color32::WHITE);
    v.window_corner_radius = CornerRadius::same(10);
    for (w, fill) in [
        (&mut v.widgets.inactive, PAD),
        (&mut v.widgets.hovered, Color32::from_rgb(0x26, 0x27, 0x2e)),
        (&mut v.widgets.active, Color32::from_rgb(0x2e, 0x2f, 0x37)),
        (&mut v.widgets.open, PAD),
    ] {
        w.weak_bg_fill = fill;
        w.bg_fill = BORDER;
        w.corner_radius = CornerRadius::same(6);
    }
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, BORDER);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, ACCENT_LIGHT);
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    ctx.set_visuals_of(egui::Theme::Dark, v);
    ctx.set_theme(egui::ThemePreference::Dark);
    ctx.all_styles_mut(|s| {
        s.spacing.item_spacing = vec2(8.0, 8.0);
        s.spacing.button_padding = vec2(10.0, 5.0);
    });
}

fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("semibold".into()))
}

impl App {
    pub fn new(cc: &eframe::CreationContext, s: Settings, notice: Option<String>) -> App {
        setup_style(&cc.egui_ctx);
        let ctx = cc.egui_ctx.clone();
        let choice = DeviceChoice { mic: s.mic_device.clone(), friends: s.friends_device.clone(), you: s.you_device.clone() };
        let engine = Engine::start(choice, (s.friends_volume, s.you_volume), pad_plays(&s), Box::new(move || ctx.request_repaint()));
        let tx = engine.sender();
        let (htx, hctx) = (tx.clone(), cc.egui_ctx.clone());
        let hotkeys = HotkeyService::start(move |a| {
            let _ = htx.send(match a {
                HotkeyAction::Pad(id) => Cmd::Enqueue(id),
                HotkeyAction::Pause => Cmd::TogglePause,
                HotkeyAction::Skip => Cmd::Skip,
                HotkeyAction::Stop => Cmd::Stop,
            });
            hctx.request_repaint();
        });
        let (import_tx, import_rx) = channel();
        let mut app = App {
            s,
            engine,
            tx,
            hotkeys,
            capture: None,
            capture_msg: None,
            import_tx,
            import_rx,
            notice: notice.map(|n| (n, Instant::now())),
            rename: None,
            confirm_delete: None,
            seek_drag: None,
            editor: None,
        };
        app.sync();
        app
    }

    /// Push the library to the engine and hotkeys, and save it.
    fn sync(&mut self) {
        let _ = self.tx.send(Cmd::SetPads(pad_plays(&self.s)));
        self.hotkeys.set_bindings(bindings(&self.s));
        self.save();
    }

    fn save(&mut self) {
        if let Err(e) = self.s.save(&settings_path()) {
            self.set_notice(format!("Couldn't save your settings: {e}"));
        }
    }

    fn set_notice(&mut self, msg: String) {
        self.notice = Some((msg, Instant::now()));
    }

    fn import_paths(&mut self, paths: Vec<PathBuf>) {
        let jobs: Vec<(PathBuf, u32, PadId)> = paths.into_iter().map(|p| (p, self.s.new_id(), self.s.new_id())).collect();
        let tx = self.import_tx.clone();
        std::thread::spawn(move || {
            let dir = sounds_dir();
            for (path, src_id, pad_id) in jobs {
                let r = import(&path, &dir, src_id);
                let _ = tx.send((path, pad_id, r));
            }
        });
    }

    fn poll_imports(&mut self) {
        let mut changed = false;
        while let Ok((path, pad_id, r)) = self.import_rx.try_recv() {
            match r {
                Ok(src) => {
                    self.s.pads.push(whole_pad(&src, pad_id));
                    self.s.sources.push(src);
                    changed = true;
                }
                Err(_) => {
                    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    self.set_notice(format!("Couldn't add '{name}': not a supported audio file (MP3, WAV, OGG, FLAC)."));
                }
            }
        }
        if changed {
            self.sync();
        }
    }

    fn pick_files(&mut self) {
        if let Some(paths) = rfd::FileDialog::new().add_filter("Audio", &["mp3", "wav", "ogg", "flac"]).pick_files() {
            self.import_paths(paths);
        }
    }

    fn start_capture(&mut self, t: CaptureTarget) {
        self.hotkeys.set_bindings(vec![]); // so existing keys don't fire while choosing
        self.capture = Some((t, keyboard_snapshot()));
        self.capture_msg = None;
    }

    fn key_of(&self, t: CaptureTarget) -> Option<Hotkey> {
        match t {
            CaptureTarget::Pad(id) => self.s.pads.iter().find(|p| p.id == id).and_then(|p| p.hotkey),
            CaptureTarget::Pause => self.s.pause_key,
            CaptureTarget::Skip => self.s.skip_key,
            CaptureTarget::Stop => self.s.stop_key,
        }
    }

    fn set_key(&mut self, t: CaptureTarget, h: Option<Hotkey>) {
        match t {
            CaptureTarget::Pad(id) => {
                if let Some(p) = self.s.pads.iter_mut().find(|p| p.id == id) {
                    p.hotkey = h;
                }
            }
            CaptureTarget::Pause => self.s.pause_key = h,
            CaptureTarget::Skip => self.s.skip_key = h,
            CaptureTarget::Stop => self.s.stop_key = h,
        }
    }

    fn owner_of(&self, h: Hotkey, except: CaptureTarget) -> Option<String> {
        if let Some(p) = self.s.pads.iter().find(|p| p.hotkey == Some(h) && except != CaptureTarget::Pad(p.id)) {
            return Some(p.name.clone());
        }
        [(CaptureTarget::Pause, self.s.pause_key, "Pause/Resume"), (CaptureTarget::Skip, self.s.skip_key, "Skip"), (CaptureTarget::Stop, self.s.stop_key, "Stop")]
            .into_iter()
            .find(|(t, k, _)| *k == Some(h) && *t != except)
            .map(|(_, _, n)| n.to_string())
    }

    fn poll_capture(&mut self, ctx: &egui::Context) {
        let Some((target, baseline)) = self.capture else { return };
        ctx.request_repaint_after(Duration::from_millis(16));
        match poll_capture(&baseline) {
            Some(Captured::Key(h)) => {
                if let Some(owner) = self.owner_of(h, target) {
                    self.capture_msg = Some(format!("{} is already used by '{owner}'.", key_name(h)));
                    self.capture = Some((target, keyboard_snapshot()));
                    return;
                }
                self.set_key(target, Some(h));
                self.capture = None;
                self.sync();
            }
            Some(Captured::Cancel) => self.end_capture(),
            None => {}
        }
    }

    fn end_capture(&mut self) {
        self.capture = None;
        self.capture_msg = None;
        self.sync(); // restores the bindings
    }

    fn delete_pad(&mut self, id: PadId) {
        let Some(i) = self.s.pads.iter().position(|p| p.id == id) else { return };
        let pad = self.s.pads.remove(i);
        if !self.s.pads.iter().any(|p| p.source == pad.source)
            && let Some(j) = self.s.sources.iter().position(|s| s.id == pad.source) {
                let src = self.s.sources.remove(j);
                let _ = std::fs::remove_file(sounds_dir().join(&src.file));
            }
        self.sync();
    }

    fn pad_name(&self, id: PadId) -> String {
        self.s.pads.iter().find(|p| p.id == id).map(|p| p.name.clone()).unwrap_or_else(|| "(deleted)".into())
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("top").frame(egui::Frame::new().fill(BAR).inner_margin(egui::Margin::symmetric(14, 10))).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Sound Projector").font(semibold(16.0)));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let you = self.volume_slider(ui, "You hear", GREEN, false);
                    ui.add_space(12.0);
                    let friends = self.volume_slider(ui, "Friends hear", ACCENT_LIGHT, true);
                    if you || friends {
                        let _ = self.tx.send(Cmd::SetVolumes { friends: self.s.friends_volume, you: self.s.you_volume });
                    }
                });
            });
        });
    }

    /// Returns true when the value changed; saves when the drag ends.
    fn volume_slider(&mut self, ui: &mut egui::Ui, label: &str, color: Color32, friends: bool) -> bool {
        let r = ui
            .scope(|ui| {
                ui.visuals_mut().selection.bg_fill = color;
                let v = if friends { &mut self.s.friends_volume } else { &mut self.s.you_volume };
                let r = ui.add(egui::Slider::new(v, 0.0..=1.0).show_value(false).trailing_fill(true));
                ui.label(egui::RichText::new(label).color(MUTED));
                r
            })
            .inner;
        if r.drag_stopped() || (r.changed() && !r.dragged()) {
            self.save();
        }
        r.changed()
    }

    fn banners(&mut self, ui: &mut egui::Ui, view: &View) {
        let mut lines = Vec::new();
        if let Err(e) = &view.friends {
            lines.push(format!("Friends can't hear sounds: {e}."));
        } else if let Err(e) = &view.mic {
            lines.push(format!("Your mic isn't coming through: {e}."));
        }
        if let Err(e) = &view.you {
            lines.push(format!("You can't hear sounds: {e}."));
        }
        if let Some(n) = self.engine.take_notice() {
            self.set_notice(n);
        }
        if self.notice.as_ref().is_some_and(|(_, t)| t.elapsed() > Duration::from_secs(6)) {
            self.notice = None;
        }
        if lines.is_empty() && self.notice.is_none() {
            return;
        }
        egui::Panel::top("banners").frame(egui::Frame::new().fill(WARN_BG).inner_margin(egui::Margin::symmetric(14, 8))).show(ui, |ui| {
            for l in &lines {
                ui.label(egui::RichText::new(l).color(WARN));
            }
            if let Some((n, _)) = self.notice.clone() {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(n).color(TEXT));
                    if ui.small_button("×").on_hover_text("Dismiss").clicked() {
                        self.notice = None;
                    }
                });
                ui.ctx().request_repaint_after(Duration::from_secs(1));
            }
        });
    }

    fn now_playing(&mut self, ui: &mut egui::Ui, view: &View) {
        egui::Panel::bottom("now").frame(egui::Frame::new().fill(BAR).inner_margin(egui::Margin::symmetric(14, 10))).show(ui, |ui| {
            let playing = view.current.is_some();
            ui.horizontal(|ui| {
                let title = match view.current {
                    Some((_, pad)) => egui::RichText::new(format!("▶  {}", self.pad_name(pad))).font(semibold(14.0)),
                    None => egui::RichText::new("Nothing playing").color(MUTED),
                };
                ui.label(title);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_enabled_ui(playing, |ui| {
                        if ui.button("■ Stop").clicked() {
                            let _ = self.tx.send(Cmd::Stop);
                        }
                        if ui.button("⏭ Skip").clicked() {
                            let _ = self.tx.send(Cmd::Skip);
                        }
                        let pause = if view.paused { "▶ Resume" } else { "⏸ Pause" };
                        if ui.button(pause).clicked() {
                            let _ = self.tx.send(Cmd::TogglePause);
                        }
                    });
                });
            });
            ui.horizontal(|ui| {
                let frac = self.seek_drag.unwrap_or(if view.dur_secs > 0.0 { view.pos_secs / view.dur_secs } else { 0.0 });
                let shown = if self.seek_drag.is_some() { frac * view.dur_secs } else { view.pos_secs };
                ui.label(egui::RichText::new(fmt_time(shown as f64)).color(MUTED).monospace());
                let w = (ui.available_width() - 50.0).max(40.0);
                let (rect, r) = ui.allocate_exact_size(vec2(w, 14.0), if playing { Sense::click_and_drag() } else { Sense::hover() });
                let bar = Rect::from_center_size(rect.center(), vec2(rect.width(), 4.0));
                ui.painter().rect_filled(bar, 2.0, BORDER);
                let filled = Rect::from_min_size(bar.min, vec2(bar.width() * frac.clamp(0.0, 1.0), bar.height()));
                ui.painter().rect_filled(filled, 2.0, ACCENT_LIGHT);
                if r.hovered() || r.dragged() {
                    ui.painter().circle_filled(filled.right_center(), 6.0, Color32::WHITE);
                }
                if let Some(p) = r.interact_pointer_pos() {
                    self.seek_drag = Some(((p.x - bar.left()) / bar.width()).clamp(0.0, 1.0));
                }
                if (r.drag_stopped() || r.clicked())
                    && let Some(f) = self.seek_drag.take() {
                        let _ = self.tx.send(Cmd::Seek(f));
                    }
                ui.label(egui::RichText::new(fmt_time(view.dur_secs as f64)).color(MUTED).monospace());
            });
        });
    }

    fn queue_panel(&mut self, ui: &mut egui::Ui, view: &View) {
        egui::Panel::right("queue")
            .exact_size(210.0)
            .frame(egui::Frame::new().fill(PANEL).inner_margin(egui::Margin::same(12)))
            .show(ui, |ui| {
                ui.label(egui::RichText::new("UP NEXT").color(MUTED).size(11.0));
                if view.up_next.is_empty() {
                    ui.label(egui::RichText::new("Click a pad or press its hotkey to queue it.").color(MUTED));
                    return;
                }
                let mut rows: Vec<Rect> = Vec::new();
                let mut remove = None;
                let (_, dropped) = ui.dnd_drop_zone::<u64, ()>(egui::Frame::NONE, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        for &(entry, pad) in &view.up_next {
                            let r = egui::Frame::new().fill(PAD).corner_radius(6).inner_margin(egui::Margin::symmetric(8, 6)).show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.horizontal(|ui| {
                                    ui.dnd_drag_source(Id::unique(("queue", entry)), entry, |ui| {
                                        ui.label(egui::RichText::new("⋮⋮").color(MUTED)).on_hover_text("Drag to reorder");
                                    });
                                    ui.add(egui::Label::new(self.pad_name(pad)).truncate());
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        if ui.small_button("×").on_hover_text("Remove from queue").clicked() {
                                            remove = Some(entry);
                                        }
                                    });
                                });
                            });
                            rows.push(r.response.rect);
                        }
                    });
                });
                if let Some(e) = remove {
                    let _ = self.tx.send(Cmd::Remove(e));
                }
                if let (Some(entry), Some(p)) = (dropped, ui.ctx().pointer_interact_pos()) {
                    let from = view.up_next.iter().position(|(e, _)| *e == *entry).unwrap_or(0);
                    let mut to = rows.iter().position(|r| p.y < r.center().y).unwrap_or(rows.len());
                    if from < to {
                        to -= 1;
                    }
                    let _ = self.tx.send(Cmd::Move(*entry, to));
                }
            });
    }

    fn edit(&mut self, ui: &mut egui::Ui, view: &View) {
        let failed = self.hotkeys.failed();
        let action = egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BG).inner_margin(egui::Margin::same(14)))
            .show(ui, |ui| egui::ScrollArea::vertical().show(ui, |ui| self.editor.as_mut().map(|e| e.show(ui, &mut self.s, view, &failed))).inner)
            .inner;
        match action {
            Some(editor::EditorAction::Done) => {
                let _ = self.tx.send(Cmd::StopPreview);
                self.editor = None;
                self.sync();
            }
            Some(editor::EditorAction::Changed) => self.sync(),
            Some(editor::EditorAction::Preview(id)) => {
                self.sync();
                if let Some(pp) = pad_plays(&self.s).into_iter().find(|p| p.id == id) {
                    let _ = self.tx.send(Cmd::Preview(pp));
                }
            }
            Some(editor::EditorAction::StopPreview) => {
                let _ = self.tx.send(Cmd::StopPreview);
            }
            Some(editor::EditorAction::Capture(id)) => self.start_capture(CaptureTarget::Pad(id)),
            Some(editor::EditorAction::Delete(id)) => self.confirm_delete = Some(id),
            Some(editor::EditorAction::None) | None => {}
        }
    }

    fn grid(&mut self, ui: &mut egui::Ui, view: &View) {
        egui::CentralPanel::default().frame(egui::Frame::new().fill(BG).inner_margin(egui::Margin::same(12))).show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                let gap = 8.0;
                let cols = ((ui.available_width() + gap) / 180.0).floor().max(1.0) as usize;
                let w = (ui.available_width() - gap * (cols as f32 - 1.0)) / cols as f32;
                let failed = self.hotkeys.failed();
                let ids: Vec<PadId> = self.s.pads.iter().map(|p| p.id).collect();
                let n = ids.len() + 1; // + Add tile
                for row in 0..n.div_ceil(cols) {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = gap;
                        for col in 0..cols {
                            let i = row * cols + col;
                            if i < ids.len() {
                                self.pad_tile(ui, ids[i], w, view, &failed);
                            } else if i == ids.len() {
                                self.add_tile(ui, w);
                            }
                        }
                    });
                    ui.add_space(gap - ui.spacing().item_spacing.y);
                }
            });
        });
    }

    fn pad_tile(&mut self, ui: &mut egui::Ui, id: PadId, w: f32, view: &View, failed: &[HotkeyAction]) {
        let Some(pad) = self.s.pads.iter().find(|p| p.id == id).cloned() else { return };
        let src = self.s.sources.iter().find(|s| s.id == pad.source).cloned();
        let (rect, r) = ui.allocate_exact_size(vec2(w, 64.0), Sense::click());
        r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &pad.name));
        let playing = view.current.map(|c| c.1) == Some(id);
        let stroke = if playing {
            Stroke::new(1.5, ACCENT_LIGHT)
        } else if r.hovered() {
            Stroke::new(1.0, ACCENT_LIGHT)
        } else {
            Stroke::new(1.0, BORDER)
        };
        let p = ui.painter();
        p.rect(rect, 8.0, if playing { PLAYING } else { PAD }, stroke, StrokeKind::Inside);
        let inner = rect.shrink2(vec2(12.0, 10.0));
        let mut job = egui::text::LayoutJob::simple_singleline(pad.name.clone(), semibold(14.0), TEXT);
        job.wrap = egui::text::TextWrapping::truncate_at_width(inner.width());
        p.galley(inner.left_top(), p.layout_job(job), TEXT);

        let secs = src.as_ref().map_or(0.0, |s| frames_to_secs(pad.end - pad.start, s.rate));
        let mut meta = if src.as_ref().is_some_and(|s| is_clip(&pad, s)) { format!("Song clip · {}", fmt_time(secs)) } else { fmt_time(secs) };
        if let Some(q) = view.up_next.iter().position(|(_, p)| *p == id) {
            meta.push_str(&format!(" · #{} in queue", q + 1));
        }
        let mut meta_width = inner.width();
        if let Some(h) = pad.hotkey {
            let bad = failed.contains(&HotkeyAction::Pad(id));
            let g = p.layout_no_wrap(key_name(h), FontId::monospace(11.0), if bad { ERROR } else { Color32::from_rgb(0xc8, 0xc8, 0xd0) });
            let badge = Rect::from_min_size(inner.right_bottom() - g.size() - vec2(8.0, 3.0), g.size() + vec2(8.0, 6.0));
            meta_width = badge.left() - inner.left() - 6.0;
            p.rect_filled(badge, 4.0, if bad { Color32::from_rgb(0x4a, 0x1f, 0x24) } else { BORDER });
            p.galley(badge.min + vec2(4.0, 3.0), g, TEXT);
            if bad {
                r.clone().on_hover_text("Another app is using this key");
            }
        }
        let mut job = egui::text::LayoutJob::simple_singleline(meta, FontId::proportional(12.0), MUTED);
        job.wrap = egui::text::TextWrapping::truncate_at_width(meta_width.max(0.0));
        let g = p.layout_job(job);
        p.galley(inner.left_bottom() - vec2(0.0, g.size().y), g, MUTED);

        if r.clicked() {
            let _ = self.tx.send(Cmd::Enqueue(id));
        }
        r.context_menu(|ui| {
            if let Some(src) = src.clone()
                && ui.button("Edit").clicked()
            {
                let mut ed = editor::Editor::open(&src, sounds_dir().join(&src.file));
                ed.select(id);
                self.editor = Some(ed);
                ui.close();
            }
            if ui.button("Rename").clicked() {
                self.rename = Some((id, pad.name.clone()));
                ui.close();
            }
            if ui.button("Set hotkey…").clicked() {
                self.start_capture(CaptureTarget::Pad(id));
                ui.close();
            }
            if pad.hotkey.is_some() && ui.button("Clear hotkey").clicked() {
                self.set_key(CaptureTarget::Pad(id), None);
                self.sync();
                ui.close();
            }
            if ui.button("Delete").clicked() {
                self.confirm_delete = Some(id);
                ui.close();
            }
        });
    }

    fn add_tile(&mut self, ui: &mut egui::Ui, w: f32) {
        let (rect, r) = ui.allocate_exact_size(vec2(w, 64.0), Sense::click());
        r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Add sounds"));
        let p = ui.painter();
        p.rect(rect, 8.0, BG, Stroke::new(1.0, if r.hovered() { ACCENT_LIGHT } else { BORDER }), StrokeKind::Inside);
        let inner = rect.shrink2(vec2(12.0, 10.0));
        p.text(inner.left_top(), Align2::LEFT_TOP, "+ Add", semibold(14.0), TEXT);
        p.text(inner.left_bottom(), Align2::LEFT_BOTTOM, "or drop files here", FontId::proportional(12.0), MUTED);
        if r.clicked() {
            self.pick_files();
        }
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        if let Some((target, _)) = self.capture {
            let current = self.key_of(target);
            egui::Modal::new(Id::unique("capture")).show(ctx, |ui| {
                ui.set_width(320.0);
                ui.label(egui::RichText::new("Press a key…").font(semibold(16.0)));
                ui.label(egui::RichText::new("Pick keys your game doesn't use, like the numpad or F13–F24.").color(MUTED));
                if let Some(m) = &self.capture_msg {
                    ui.label(egui::RichText::new(m).color(ERROR));
                }
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        self.end_capture();
                    }
                    if current.is_some() && ui.button("Clear key").clicked() {
                        self.set_key(target, None);
                        self.end_capture();
                    }
                    ui.label(egui::RichText::new("Esc cancels").color(MUTED));
                });
            });
        }
        if let Some((id, mut name)) = self.rename.take() {
            let mut keep = true;
            egui::Modal::new(Id::unique("rename")).show(ctx, |ui| {
                ui.set_width(300.0);
                ui.label(egui::RichText::new("Rename").font(semibold(16.0)));
                let r = ui.add(egui::TextEdit::singleline(&mut name).desired_width(f32::INFINITY));
                r.request_focus();
                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() || enter {
                        if let Some(p) = self.s.pads.iter_mut().find(|p| p.id == id) {
                            p.name = name.trim().to_string();
                        }
                        keep = false;
                    }
                    if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        keep = false;
                    }
                });
            });
            if keep {
                self.rename = Some((id, name));
            } else {
                self.sync();
            }
        }
        if let Some(id) = self.confirm_delete {
            let name = self.pad_name(id);
            egui::Modal::new(Id::unique("delete")).show(ctx, |ui| {
                ui.set_width(300.0);
                ui.label(egui::RichText::new(format!("Delete '{name}'?")).font(semibold(16.0)));
                ui.horizontal(|ui| {
                    if ui.button(egui::RichText::new("Delete").color(ERROR)).clicked() {
                        self.confirm_delete = None;
                        self.delete_pad(id);
                    }
                    if ui.button("Cancel").clicked() {
                        self.confirm_delete = None;
                    }
                });
            });
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll_imports();
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        if !dropped.is_empty() {
            self.import_paths(dropped);
        }
        self.poll_capture(&ctx);
        let view = self.engine.view();
        self.top_bar(ui);
        self.banners(ui, &view);
        self.now_playing(ui, &view);
        self.queue_panel(ui, &view);
        if self.editor.is_some() {
            self.edit(ui, &view);
        } else {
            self.grid(ui, &view);
        }
        self.dialogs(&ctx);
        if view.current.is_some() || view.preview_secs.is_some() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }
}
