use super::{ACCENT, ACCENT_LIGHT, BORDER, MUTED, TEXT, hotkey_field, press_hint, semibold};
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
    /// Play source frames `from..to` to you only.
    Listen { from: u64, to: u64, boost_db: f32 },
    StopPreview,
    Press(PadId),
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
    listen: Option<(u64, u64, f32)>, // the last Listen: the playhead counts from its start, Space resumes it
    drag: Option<Drag>,
    dirty: bool,
}

impl Editor {
    pub fn open(src: &Source, path: PathBuf) -> Editor {
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let _ = tx.send(crate::decode::compute_peaks(&path, 256));
        });
        Editor { source: src.id, peaks: None, rx, selected: None, listen: None, drag: None, dirty: false }
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
            if ui.button("← Back").clicked() {
                action = EditorAction::Done;
            }
            let secs = frames_to_secs(src.frames, src.rate);
            ui.label(egui::RichText::new(format!("{} ({})", src.name, fmt_time(secs))).font(semibold(16.0)));
        });
        ui.label(
            egui::RichText::new("Click anywhere to listen from there (only you hear it) · Space pauses · Drag across to mark a snippet")
                .color(MUTED),
        );
        ui.add_space(4.0);

        if let Some(a) = self.waveform(ui, s, &src, view) {
            action = a;
        }
        ui.add_space(6.0);
        if let Some(a) = self.chips(ui, s, &src, view) {
            action = a;
        }
        ui.add_space(6.0);
        if let Some(a) = self.fields(ui, s, &src, view, failed) {
            action = a;
        }

        // Space pauses and resumes listening, unless a text field has the keys.
        if matches!(action, EditorAction::None)
            && ui.input(|i| i.key_pressed(egui::Key::Space))
            && ui.ctx().memory(|m| m.focused().is_none())
        {
            action = match (view.preview_secs, self.listen) {
                (Some(secs), Some((from, to, boost_db))) => {
                    self.listen = Some(((from + secs_to_frames(secs as f64, src.rate)).min(to), to, boost_db));
                    EditorAction::StopPreview
                }
                (Some(_), None) => EditorAction::StopPreview,
                (None, Some((from, to, boost_db))) => self.listen_range(from, to, boost_db),
                (None, None) => match self.selected.and_then(|id| s.pads.iter().find(|p| p.id == id)) {
                    Some(p) => self.listen_range(p.start, p.end, p.boost_db),
                    None => self.listen_range(0, src.frames, 0.0),
                },
            };
        }

        let idle = !ui.input(|i| i.pointer.any_down()) && ui.ctx().memory(|m| m.focused().is_none());
        if self.dirty && idle && matches!(action, EditorAction::None) {
            self.dirty = false;
            action = EditorAction::Changed;
        }
        action
    }

    fn waveform(&mut self, ui: &mut egui::Ui, s: &mut Settings, src: &Source, view: &View) -> Option<EditorAction> {
        let mut action = None;
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
        // Not playing, it stays put and dims: that's where Space picks up.
        if let Some((from, ..)) = self.listen {
            let (f, color) = match view.preview_secs {
                Some(secs) => (from + secs_to_frames(secs as f64, src.rate), Color32::WHITE),
                None => (from, MUTED),
            };
            p.line_segment([Pos2::new(x_of(f), rect.top()), Pos2::new(x_of(f), rect.bottom())], Stroke::new(2.0, color));
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
                    let (group, instant) = s.pads.iter().find(|p| Some(p.id) == self.selected).map_or((None, false), |p| (p.group, p.instant));
                    let pad = Pad { id, source: src.id, name: format!("Section {n}"), start, end, boost_db: 0.0, hotkey: None, group, instant };
                    let at = s.pads.iter().rposition(|p| p.source == src.id).map_or(s.pads.len(), |i| i + 1);
                    s.pads.insert(at, pad);
                    self.selected = Some(id);
                    self.dirty = true;
                } else {
                    action = Some(self.listen_at(&pads, frame_at(r.interact_pointer_pos().map_or(rect.left(), |p| p.x)), src));
                }
            }
            self.drag = None;
        }
        if r.clicked()
            && let Some(pos) = r.interact_pointer_pos() {
                action = Some(self.listen_at(&pads, frame_at(pos.x), src));
            }
        action
    }

    /// Select what's under `f`, and listen from there to the end of the song.
    fn listen_at(&mut self, pads: &[(usize, Pad)], f: u64, src: &Source) -> EditorAction {
        self.select_at(pads, f);
        let boost_db = pads.iter().find(|(_, p)| Some(p.id) == self.selected).map_or(0.0, |(_, p)| p.boost_db);
        self.listen_range(f, src.frames, boost_db)
    }

    fn listen_range(&mut self, from: u64, to: u64, boost_db: f32) -> EditorAction {
        self.listen = Some((from, to, boost_db));
        EditorAction::Listen { from, to, boost_db }
    }

    /// Pick the shortest region under the pointer, so sections win over the whole-song pad.
    fn select_at(&mut self, pads: &[(usize, Pad)], f: u64) {
        self.selected = pads.iter().filter(|(_, p)| p.start <= f && f < p.end).min_by_key(|(_, p)| p.end - p.start).map(|(_, p)| p.id);
    }

    /// Picking a snippet aims Space at it. If you're already listening, it goes straight there.
    fn pick(&mut self, pad: &Pad, listening: bool) -> Option<EditorAction> {
        self.selected = Some(pad.id);
        if listening {
            return Some(self.listen_range(pad.start, pad.end, pad.boost_db));
        }
        self.listen = None;
        None
    }

    fn chips(&mut self, ui: &mut egui::Ui, s: &Settings, src: &Source, view: &View) -> Option<EditorAction> {
        let mut action = None;
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
                    action = self.pick(pad, view.preview_secs.is_some());
                }
            }
        });
        action
    }

    fn fields(&mut self, ui: &mut egui::Ui, s: &mut Settings, src: &Source, view: &View, failed: &[HotkeyAction]) -> Option<EditorAction> {
        let only_pad = s.pads.iter().filter(|p| p.source == src.id).count() <= 1;
        let Some(pad) = self.selected.and_then(|id| s.pads.iter_mut().find(|p| p.id == id)) else {
            ui.label(egui::RichText::new("Click a snippet to edit it, or drag across the waveform to mark one.").color(MUTED));
            return None;
        };
        let mut action = None;
        let dur = frames_to_secs(src.frames, src.rate);
        let min_len = secs_to_frames(0.05, src.rate).max(1);
        ui.horizontal(|ui| {
            let (icon, words) = press_hint(view, pad.id, pad.instant);
            let friends = egui::Button::new(egui::RichText::new(format!("{icon} {words}")).color(TEXT)).fill(ACCENT);
            if ui.add(friends).clicked() {
                action = Some(EditorAction::Press(pad.id));
            }
            if view.preview_secs.is_some() {
                if ui.button("■ Stop preview").clicked() {
                    action = Some(EditorAction::StopPreview);
                }
            } else if ui.button("▶ Preview").on_hover_text("Plays this part to you only").clicked() {
                let (a, b, boost) = (pad.start, pad.end, pad.boost_db);
                action = Some(self.listen_range(a, b, boost));
            }
            let del = ui.add_enabled(!only_pad, egui::Button::new("Delete snippet"));
            let del = if only_pad { del.on_disabled_hover_text("Delete the whole sound from the grid instead") } else { del };
            if del.clicked() {
                action = Some(EditorAction::Delete(pad.id));
            }
        });
        ui.add_space(8.0);
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
        step_with(ctx, ed, s, t, events, &View::default())
    }

    fn step_with(ctx: &egui::Context, ed: &mut Editor, s: &mut Settings, t: f64, events: Vec<Event>, view: &View) -> EditorAction {
        let raw = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), time: Some(t), events, ..Default::default() };
        let mut out = None;
        // egui may run more than one pass per frame; keep the first real action, like the app does.
        let mut full = ctx.run_ui(raw, |ui| {
            let a = ed.show(ui, s, view, &[]);
            if out.is_none() || !matches!(a, EditorAction::None) {
                out = Some(a);
            }
        });
        full.textures_delta.clear(); // headless: nothing uploads textures
        out.unwrap()
    }

    fn setup() -> (egui::Context, Settings, Editor) {
        let ctx = egui::Context::default();
        super::super::setup_style(&ctx);
        let src = Source { id: 1, file: "missing.wav".into(), name: "Song".into(), rate: 1000, frames: 100_000, channels: 1 };
        let s = Settings { sources: vec![src.clone()], pads: vec![crate::library::whole_pad(&src, 2)], next_id: 3, ..Default::default() };
        let ed = Editor::open(&src, PathBuf::from("missing.wav"));
        (ctx, s, ed)
    }

    const Y: f32 = 120.0; // inside the waveform (header + note take ~60 px)

    fn press(x: f32, pressed: bool) -> Event {
        Event::PointerButton { pos: Pos2::new(x, Y), button: PointerButton::Primary, pressed, modifiers: Default::default() }
    }

    fn space() -> Event {
        Event::Key { key: egui::Key::Space, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }
    }

    #[test]
    fn clicking_the_waveform_listens_from_there_to_the_end() {
        let (ctx, mut s, mut ed) = setup();
        step(&ctx, &mut ed, &mut s, 0.0, vec![]);
        step(&ctx, &mut ed, &mut s, 0.1, vec![Event::PointerMoved(Pos2::new(400.0, Y)), press(400.0, true)]);
        let a = step(&ctx, &mut ed, &mut s, 0.15, vec![press(400.0, false)]);
        let EditorAction::Listen { from, to, .. } = a else { panic!("expected Listen") };
        assert!((from as i64 - 50_000).abs() < 2_000 && to == 100_000, "{from} {to}");
    }

    #[test]
    fn space_pauses_where_it_got_to_and_resumes_from_there() {
        let (ctx, mut s, mut ed) = setup();
        step(&ctx, &mut ed, &mut s, 0.0, vec![]);
        step(&ctx, &mut ed, &mut s, 0.1, vec![Event::PointerMoved(Pos2::new(400.0, Y)), press(400.0, true)]);
        let EditorAction::Listen { from, .. } = step(&ctx, &mut ed, &mut s, 0.15, vec![press(400.0, false)]) else { panic!() };
        let playing = View { preview_secs: Some(2.0), ..View::default() };
        assert!(matches!(step_with(&ctx, &mut ed, &mut s, 0.2, vec![space()], &playing), EditorAction::StopPreview));
        let EditorAction::Listen { from: resumed, to, .. } = step(&ctx, &mut ed, &mut s, 0.3, vec![space()]) else { panic!("expected Listen") };
        assert_eq!((resumed, to), (from + 2_000, 100_000)); // 2 s in at 1 kHz
    }

    fn snippet() -> Pad {
        Pad { id: 3, source: 1, name: "Hook".into(), start: 10_000, end: 20_000, boost_db: 0.0, hotkey: None, group: None, instant: false }
    }

    #[test]
    fn space_plays_the_snippet_you_just_picked_not_the_last_one_you_heard() {
        let (ctx, mut s, mut ed) = setup();
        step(&ctx, &mut ed, &mut s, 0.0, vec![]);
        step(&ctx, &mut ed, &mut s, 0.1, vec![Event::PointerMoved(Pos2::new(400.0, Y)), press(400.0, true)]);
        assert!(matches!(step(&ctx, &mut ed, &mut s, 0.15, vec![press(400.0, false)]), EditorAction::Listen { .. }));
        let hook = snippet();
        s.pads.push(hook.clone());
        assert!(ed.pick(&hook, false).is_none()); // nothing's playing, so picking stays quiet
        let EditorAction::Listen { from, to, .. } = step(&ctx, &mut ed, &mut s, 0.3, vec![space()]) else { panic!("expected Listen") };
        assert_eq!((from, to), (10_000, 20_000));
    }

    #[test]
    fn picking_a_snippet_while_listening_jumps_to_it() {
        let (_ctx, _s, mut ed) = setup();
        let Some(EditorAction::Listen { from, to, .. }) = ed.pick(&snippet(), true) else { panic!("expected Listen") };
        assert_eq!((from, to), (10_000, 20_000));
    }

    #[test]
    fn dragging_across_the_waveform_creates_a_section() {
        let (ctx, mut s, mut ed) = setup();
        step(&ctx, &mut ed, &mut s, 0.0, vec![]);
        let y = Y;
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

    #[test]
    fn a_new_section_joins_the_group_of_the_pad_it_came_from() {
        let (ctx, mut s, mut ed) = setup();
        s.pads[0].group = Some(7);
        s.pads[0].instant = true;
        ed.select(2);
        step(&ctx, &mut ed, &mut s, 0.0, vec![]);
        step(&ctx, &mut ed, &mut s, 0.1, vec![Event::PointerMoved(Pos2::new(200.0, Y)), press(200.0, true)]);
        for (i, x) in [260.0, 320.0, 400.0].into_iter().enumerate() {
            step(&ctx, &mut ed, &mut s, 0.2 + i as f64 * 0.05, vec![Event::PointerMoved(Pos2::new(x, Y))]);
        }
        step(&ctx, &mut ed, &mut s, 0.5, vec![press(400.0, false)]);
        assert_eq!(s.pads.iter().map(|p| p.group).collect::<Vec<_>>(), vec![Some(7), Some(7)]);
        assert!(s.pads[1].instant);
    }
}
