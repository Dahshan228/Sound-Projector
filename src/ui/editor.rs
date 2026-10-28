use super::{ACCENT_LIGHT, BORDER, MUTED, TEXT, hotkey_field, semibold};
use crate::engine::View;
use crate::hotkeys::HotkeyAction;
use crate::library::{clamp_region, fmt_time, frames_to_secs, secs_to_frames};
use crate::settings::{Pad, PadId, Settings, Source};
use eframe::egui::{self, Align2, Color32, CursorIcon, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, vec2};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};

const COLORS: [Color32; 5] = [
    Color32::from_rgb(0x7c, 0x6c, 0xff),
    Color32::from_rgb(0x3e, 0xcf, 0x8e),
    Color32::from_rgb(0xf5, 0xa5, 0x24),
    Color32::from_rgb(0xef, 0x5d, 0xa8),
    Color32::from_rgb(0x4c, 0xc9, 0xf0),
];
const WAVE_BG: Color32 = Color32::from_rgb(0x16, 0x17, 0x1c);
const WAVE: Color32 = Color32::from_rgb(0x4a, 0x4b, 0x57);
const EDGE_GRAB: f32 = 6.0;

pub enum EditorAction {
    None,
    Done,
    Changed,
    Preview(PadId),
    StopPreview,
    Capture(PadId),
    Delete(PadId),
}

enum Drag {
    Edge { pad: PadId, start: bool },
    Create { anchor: u64, current: u64 },
}

pub struct Editor {
    pub source: u32,
    peaks: Option<Result<Vec<(f32, f32)>, String>>,
    rx: Receiver<Result<Vec<(f32, f32)>, String>>,
    selected: Option<PadId>,
    drag: Option<Drag>,
    dirty: bool,
}

impl Editor {
    pub fn open(src: &Source, path: PathBuf) -> Editor {
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let _ = tx.send(crate::decode::compute_peaks(&path, 256));
        });
        Editor { source: src.id, peaks: None, rx, selected: None, drag: None, dirty: false }
    }

    pub fn select(&mut self, pad: PadId) {
        self.selected = Some(pad);
    }

    pub fn show(&mut self, ui: &mut egui::Ui, s: &mut Settings, view: &View, failed: &[HotkeyAction]) -> EditorAction {
        if let Ok(p) = self.rx.try_recv() {
            self.peaks = Some(p);
        }
        let Some(src) = s.sources.iter().find(|x| x.id == self.source).cloned() else { return EditorAction::Done };
        let mut action = EditorAction::None;

        ui.horizontal(|ui| {
            let secs = frames_to_secs(src.frames, src.rate);
            ui.label(egui::RichText::new(format!("Editing: {} ({})", src.name, fmt_time(secs))).font(semibold(16.0)));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Done").clicked() {
                    action = EditorAction::Done;
                }
            });
        });
        ui.label(egui::RichText::new("Original file is never changed").color(MUTED));
        ui.add_space(4.0);

        self.waveform(ui, s, &src, view);
        ui.add_space(6.0);
        self.chips(ui, s, &src);
        ui.add_space(6.0);
        if let Some(a) = self.fields(ui, s, &src, view, failed) {
            action = a;
        }

        let idle = !ui.input(|i| i.pointer.any_down()) && ui.ctx().memory(|m| m.focused().is_none());
        if self.dirty && idle && matches!(action, EditorAction::None) {
            self.dirty = false;
            action = EditorAction::Changed;
        }
        action
    }

    fn waveform(&mut self, ui: &mut egui::Ui, s: &mut Settings, src: &Source, view: &View) {
        let (rect, r) = ui.allocate_exact_size(vec2(ui.available_width(), 130.0), Sense::click_and_drag());
        let p = ui.painter_at(rect);
        p.rect_filled(rect, 8.0, WAVE_BG);
        let total = src.frames.max(1);
        let x_of = |f: u64| rect.left() + rect.width() * (f as f32 / total as f32);
        let frame_at = |x: f32| (((x - rect.left()) / rect.width()).clamp(0.0, 1.0) as f64 * total as f64) as u64;

        match &self.peaks {
            None => {
                p.text(rect.center(), Align2::CENTER_CENTER, "Loading waveform…", FontId::proportional(13.0), MUTED);
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
            }
            Some(Err(e)) => {
                p.text(rect.center(), Align2::CENTER_CENTER, format!("Couldn't read this file: {e}"), FontId::proportional(13.0), MUTED);
            }
            Some(Ok(peaks)) if !peaks.is_empty() => {
                let cols = rect.width().max(1.0) as usize;
                let mid = rect.center().y;
                let half = rect.height() * 0.45;
                for c in 0..cols {
                    let a = c * peaks.len() / cols;
                    let b = ((c + 1) * peaks.len() / cols).max(a + 1).min(peaks.len());
                    let (lo, hi) = peaks[a..b].iter().fold((0.0f32, 0.0f32), |(l, h), &(x, y)| (l.min(x), h.max(y)));
                    let x = rect.left() + c as f32 + 0.5;
                    p.line_segment([Pos2::new(x, mid - hi * half), Pos2::new(x, mid - lo * half - 0.5)], Stroke::new(1.0, WAVE));
                }
            }
            Some(Ok(_)) => {}
        }

        let pads: Vec<(usize, Pad)> = s.pads.iter().filter(|x| x.source == src.id).cloned().enumerate().collect();
        for (i, pad) in &pads {
            let color = COLORS[i % COLORS.len()];
            let sel = self.selected == Some(pad.id);
            let reg = Rect::from_x_y_ranges(x_of(pad.start)..=x_of(pad.end), rect.y_range());
            p.rect_filled(reg, 0.0, color.gamma_multiply(if sel { 0.28 } else { 0.12 }));
            let w = if sel { 3.0 } else { 2.0 };
            p.line_segment([reg.left_top(), reg.left_bottom()], Stroke::new(w, color));
            p.line_segment([reg.right_top(), reg.right_bottom()], Stroke::new(w, color));
            let room = reg.width() - 18.0;
            if room >= 16.0 {
                let mut job = egui::text::LayoutJob::simple_singleline(pad.name.clone(), FontId::proportional(11.0), Color32::from_rgb(0x11, 0x12, 0x16));
                job.wrap = egui::text::TextWrapping::truncate_at_width(room);
                let g = p.layout_job(job);
                let tag = Rect::from_min_size(reg.left_top() + vec2(4.0, 4.0 + 18.0 * (*i % 3) as f32), g.size() + vec2(10.0, 4.0));
                p.rect_filled(tag, 4.0, color);
                p.galley(tag.min + vec2(5.0, 2.0), g, TEXT);
            }
        }
        if let Some(Drag::Create { anchor, current }) = self.drag {
            let reg = Rect::from_x_y_ranges(x_of(anchor.min(current))..=x_of(anchor.max(current)), rect.y_range());
            p.rect(reg, 0.0, ACCENT_LIGHT.gamma_multiply(0.2), Stroke::new(1.0, ACCENT_LIGHT), StrokeKind::Inside);
        }
        if let (Some(secs), Some(pad)) = (view.preview_secs, self.selected.and_then(|id| pads.iter().find(|(_, x)| x.id == id))) {
            let f = pad.1.start + secs_to_frames(secs as f64, src.rate);
            p.line_segment([Pos2::new(x_of(f), rect.top()), Pos2::new(x_of(f), rect.bottom())], Stroke::new(1.5, Color32::WHITE));
        }
        p.rect_stroke(rect, 8.0, Stroke::new(1.0, BORDER), StrokeKind::Inside);

        // Edges of the selected region win, then any other region's.
        let edge_at = |x: f32| -> Option<(PadId, bool)> {
            let mut order: Vec<&Pad> = pads.iter().map(|(_, p)| p).collect();
            order.sort_by_key(|p| self.selected != Some(p.id));
            order.into_iter().find_map(|p| {
                if (x - x_of(p.start)).abs() <= EDGE_GRAB {
                    Some((p.id, true))
                } else if (x - x_of(p.end)).abs() <= EDGE_GRAB {
                    Some((p.id, false))
                } else {
                    None
                }
            })
        };
        if let Some(pos) = r.hover_pos()
            && (edge_at(pos.x).is_some() || matches!(self.drag, Some(Drag::Edge { .. }))) {
                ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
            }
        let min_len = secs_to_frames(0.05, src.rate).max(1);
        // Drags are recognised a few pixels after the press; anchor at where the button went down.
        if r.drag_started()
            && let Some(pos) = ui.input(|i| i.pointer.press_origin()).or(r.interact_pointer_pos()) {
                self.drag = Some(match edge_at(pos.x) {
                    Some((pad, start)) => {
                        self.selected = Some(pad);
                        Drag::Edge { pad, start }
                    }
                    None => Drag::Create { anchor: frame_at(pos.x), current: frame_at(pos.x) },
                });
            }
        if r.dragged()
            && let Some(pos) = r.interact_pointer_pos() {
                let f = frame_at(pos.x);
                match &mut self.drag {
                    Some(Drag::Edge { pad, start }) => {
                        if let Some(p) = s.pads.iter_mut().find(|p| p.id == *pad) {
                            let (a, b) = if *start { (f.min(p.end.saturating_sub(min_len)), p.end) } else { (p.start, f.max(p.start + min_len)) };
                            (p.start, p.end) = clamp_region(a, b, src.frames, min_len);
                            self.dirty = true;
                        }
                    }
                    Some(Drag::Create { current, .. }) => *current = f,
                    None => {}
                }
            }
        if r.drag_stopped() {
            if let Some(Drag::Create { anchor, current }) = self.drag.take() {
                if anchor.abs_diff(current) >= min_len {
                    let (start, end) = clamp_region(anchor, current, src.frames, min_len);
                    let n = s.pads.iter().filter(|p| p.source == src.id && p.name.starts_with("Section")).count() + 1;
                    let id = s.new_id();
                    let pad = Pad { id, source: src.id, name: format!("Section {n}"), start, end, boost_db: 0.0, hotkey: None };
                    let at = s.pads.iter().rposition(|p| p.source == src.id).map_or(s.pads.len(), |i| i + 1);
                    s.pads.insert(at, pad);
                    self.selected = Some(id);
                    self.dirty = true;
                } else {
                    self.select_at(&pads, frame_at(r.interact_pointer_pos().map_or(rect.left(), |p| p.x)));
                }
            }
            self.drag = None;
        }
        if r.clicked()
            && let Some(pos) = r.interact_pointer_pos() {
                self.select_at(&pads, frame_at(pos.x));
            }
    }

    /// Pick the shortest region under the pointer, so sections win over the whole-song pad.
    fn select_at(&mut self, pads: &[(usize, Pad)], f: u64) {
        self.selected = pads.iter().filter(|(_, p)| p.start <= f && f < p.end).min_by_key(|(_, p)| p.end - p.start).map(|(_, p)| p.id);
    }

    fn chips(&mut self, ui: &mut egui::Ui, s: &Settings, src: &Source) {
        ui.horizontal_wrapped(|ui| {
            for (i, pad) in s.pads.iter().filter(|p| p.source == src.id).enumerate() {
                let color = COLORS[i % COLORS.len()];
                let text = format!(
                    "     {}  {} → {}",
                    pad.name,
                    fmt_secs(frames_to_secs(pad.start, src.rate)),
                    fmt_secs(frames_to_secs(pad.end, src.rate))
                );
                let r = ui.add(egui::Button::new(egui::RichText::new(text).color(TEXT)).selected(self.selected == Some(pad.id)));
                ui.painter().circle_filled(r.rect.left_center() + vec2(12.0, 0.0), 4.0, color);
                if r.clicked() {
                    self.selected = Some(pad.id);
                }
            }
        });
    }

    fn fields(&mut self, ui: &mut egui::Ui, s: &mut Settings, src: &Source, view: &View, failed: &[HotkeyAction]) -> Option<EditorAction> {
        let only_pad = s.pads.iter().filter(|p| p.source == src.id).count() <= 1;
        let Some(pad) = self.selected.and_then(|id| s.pads.iter_mut().find(|p| p.id == id)) else {
            ui.label(egui::RichText::new("Drag across the waveform to mark a section, or click a section to edit it.").color(MUTED));
            return None;
        };
        let mut action = None;
        let dur = frames_to_secs(src.frames, src.rate);
        let min_len = secs_to_frames(0.05, src.rate).max(1);
        egui::Grid::new("pad-fields").num_columns(2).spacing(vec2(12.0, 8.0)).show(ui, |ui| {
            ui.label(egui::RichText::new("Name").color(MUTED));
            if ui.add(egui::TextEdit::singleline(&mut pad.name).desired_width(260.0)).changed() {
                self.dirty = true;
            }
            ui.end_row();

            ui.label(egui::RichText::new("Start / End").color(MUTED));
            ui.horizontal(|ui| {
                let mut a = frames_to_secs(pad.start, src.rate);
                let mut b = frames_to_secs(pad.end, src.rate);
                let ra = ui.add(egui::DragValue::new(&mut a).speed(0.01).range(0.0..=dur).fixed_decimals(2).suffix(" s"));
                ui.label("→");
                let rb = ui.add(egui::DragValue::new(&mut b).speed(0.01).range(0.0..=dur).fixed_decimals(2).suffix(" s"));
                if ra.changed() || rb.changed() {
                    (pad.start, pad.end) = clamp_region(secs_to_frames(a, src.rate), secs_to_frames(b, src.rate), src.frames, min_len);
                    self.dirty = true;
                }
            });
            ui.end_row();

            ui.label(egui::RichText::new("Boost").color(MUTED));
            let r = ui.add(egui::Slider::new(&mut pad.boost_db, 0.0..=18.0).step_by(0.5).custom_formatter(|v, _| format!("+{v:.1} dB")));
            if r.changed() {
                self.dirty = true;
            }
            ui.end_row();

            ui.label(egui::RichText::new("Hotkey").color(MUTED));
            let r = hotkey_field(ui, pad.hotkey, failed.contains(&HotkeyAction::Pad(pad.id)));
            if r.clicked() {
                action = Some(EditorAction::Capture(pad.id));
            }
            if r.secondary_clicked() {
                pad.hotkey = None;
                self.dirty = true;
            }
            ui.end_row();
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if view.preview_secs.is_some() {
                if ui.button("■ Stop preview").clicked() {
                    action = Some(EditorAction::StopPreview);
                }
            } else if ui.button("▶ Preview").on_hover_text("Plays to you only").clicked() {
                action = Some(EditorAction::Preview(pad.id));
            }
            let del = ui.add_enabled(!only_pad, egui::Button::new("Delete section"));
            let del = if only_pad { del.on_disabled_hover_text("Delete the whole sound from the grid instead") } else { del };
            if del.clicked() {
                action = Some(EditorAction::Delete(pad.id));
            }
        });
        action
    }
}

fn fmt_secs(s: f64) -> String {
    format!("{}:{:05.2}", (s / 60.0) as u64, s % 60.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, PointerButton, Pos2, RawInput};

    fn step(ctx: &egui::Context, ed: &mut Editor, s: &mut Settings, t: f64, events: Vec<Event>) -> EditorAction {
        let raw = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), time: Some(t), events, ..Default::default() };
        let mut out = None;
        // egui may run more than one pass per frame; keep the first real action, like the app does.
        let mut full = ctx.run_ui(raw, |ui| {
            let a = ed.show(ui, s, &View::default(), &[]);
            if out.is_none() || !matches!(a, EditorAction::None) {
                out = Some(a);
            }
        });
        full.textures_delta.clear(); // headless: nothing uploads textures
        out.unwrap()
    }

    #[test]
    fn dragging_across_the_waveform_creates_a_section() {
        let ctx = egui::Context::default();
        super::super::setup_style(&ctx);
        let src = Source { id: 1, file: "missing.wav".into(), name: "Song".into(), rate: 1000, frames: 100_000, channels: 1 };
        let mut s = Settings { sources: vec![src.clone()], pads: vec![crate::library::whole_pad(&src, 2)], next_id: 3, ..Default::default() };
        let mut ed = Editor::open(&src, PathBuf::from("missing.wav"));
        step(&ctx, &mut ed, &mut s, 0.0, vec![]);
        let y = 120.0; // inside the waveform (header + note take ~60 px)
        let press = |x: f32, pressed| Event::PointerButton { pos: Pos2::new(x, y), button: PointerButton::Primary, pressed, modifiers: Default::default() };
        step(&ctx, &mut ed, &mut s, 0.1, vec![Event::PointerMoved(Pos2::new(200.0, y)), press(200.0, true)]);
        for (i, x) in [260.0, 320.0, 400.0].into_iter().enumerate() {
            step(&ctx, &mut ed, &mut s, 0.2 + i as f64 * 0.05, vec![Event::PointerMoved(Pos2::new(x, y))]);
        }
        let released = step(&ctx, &mut ed, &mut s, 0.5, vec![press(400.0, false)]);

        assert_eq!(s.pads.len(), 2);
        let sec = &s.pads[1];
        assert_eq!((sec.name.as_str(), sec.source), ("Section 1", 1));
        let width = 800.0f64; // root ui spans the screen
        let (want_a, want_b) = ((200.0 / width * 100_000.0) as i64, (400.0 / width * 100_000.0) as i64);
        assert!((sec.start as i64 - want_a).abs() < 2_000 && (sec.end as i64 - want_b).abs() < 2_000, "{} {}", sec.start, sec.end);
        assert!(matches!(released, EditorAction::Changed)); // saved as soon as the mouse is let go
    }
}
