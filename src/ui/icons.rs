//! The app's icons, drawn with shapes instead of borrowed from fonts, so they all match.
//!
//! Each one is designed on a 14×14 grid and scaled to the rect it's given.
//! Play, Pause, Stop, Next and Restart are filled. The rest are lines of one width.
use std::f32::consts::TAU;

use eframe::egui::{self, Color32, Painter, Pos2, Rect, Shape, Stroke, Vec2, pos2, vec2};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Play,
    Pause,
    Stop,
    Next,
    Restart,
    Loop,
    Fade,
    Settings,
    Search,
    Close,
    Plus,
    Grip,
}

/// Every icon, for the test.
#[cfg(test)]
pub const ALL: [Icon; 12] = [Icon::Play, Icon::Pause, Icon::Stop, Icon::Next, Icon::Restart, Icon::Loop, Icon::Fade, Icon::Settings, Icon::Search, Icon::Close, Icon::Plus, Icon::Grip];

/// Size of an icon next to button text.
pub const SIZE: f32 = 14.0;

/// Paints `icon` in the middle of `rect`, in `color`. The rect is usually a square. If it isn't, the icon takes the square that fits.
pub fn paint(p: &Painter, rect: Rect, icon: Icon, color: Color32) {
    let side = rect.width().min(rect.height());
    let u = side / 14.0; // one grid unit, which is one pixel at size 14
    let origin = rect.center() - Vec2::splat(side / 2.0);
    let at = |x: f32, y: f32| origin + vec2(x, y) * u;
    // Never thinner than 1.5, so the small icons on the pads keep their weight.
    let stroke = Stroke::new((1.5 * u).max(1.5), color);
    // A filled box, snapped to whole pixels so its edges stay sharp at any size.
    let bar = |x0: f32, y0: f32, x1: f32, y1: f32, radius: f32| {
        p.rect_filled(Rect::from_min_max(at(x0, y0).round(), at(x1, y1).round()), radius * u, color);
    };
    // A filled triangle with soft corners: an upright edge at `base`, pointing at `tip`.
    let tri = |base: f32, tip: f32, y0: f32, y1: f32| {
        let x = at(base, 0.0).x.round();
        let corners = [pos2(x, at(0.0, y0).y), at(tip, (y0 + y1) / 2.0), pos2(x, at(0.0, y1).y)];
        p.add(Shape::convex_polygon(rounded(&corners, 1.25 * u), color, Stroke::NONE));
    };

    match icon {
        // Sits right of centre, so it looks centred in the round pad button.
        Icon::Play => tri(4.0, 12.0, 2.0, 12.0),
        Icon::Pause => {
            bar(3.0, 2.0, 6.0, 12.0, 1.0);
            bar(8.0, 2.0, 11.0, 12.0, 1.0);
        }
        // Never smaller than 8, so it holds its own beside Plus and Close on the pads.
        Icon::Stop => {
            let size = Vec2::splat((8.0 * u).max(8.0).round());
            p.rect_filled(Rect::from_min_size((at(7.0, 7.0) - size / 2.0).round(), size), u, color);
        }
        Icon::Next => {
            tri(2.0, 9.5, 3.0, 11.0);
            bar(10.0, 3.0, 12.0, 11.0, 1.0);
        }
        Icon::Restart => {
            bar(2.0, 3.0, 4.0, 11.0, 1.0);
            tri(12.0, 4.5, 3.0, 11.0);
        }
        // Two arrows chasing each other. The second is the first, turned half way round.
        Icon::Loop => {
            for s in [1.0, -1.0] {
                let q = |x: f32, y: f32| at(7.0 + s * (x - 7.0), 7.0 + s * (y - 7.0));
                // A quarter turn round the corner, then along to the arrowhead.
                let mut line: Vec<Pos2> = (0..=6).map(|i| Vec2::angled(TAU * (0.5 + i as f32 / 24.0)) * 2.25).map(|v| q(5.0 + v.x, 6.25 + v.y)).collect();
                line.push(q(10.0, 4.0));
                p.line(line, stroke);
                p.add(Shape::convex_polygon(vec![q(9.75, 1.75), q(12.5, 4.0), q(9.75, 6.25)], color, Stroke::NONE));
            }
        }
        // A sound wave dying away: four bars, each shorter than the last.
        Icon::Fade => {
            for (i, half) in [5.0, 3.5, 2.0, 0.75].into_iter().enumerate() {
                let x = 2.5 + 3.0 * i as f32;
                p.line_segment([at(x, 7.0 - half), at(x, 7.0 + half)], stroke);
            }
        }
        // A thick ring with six chunky teeth, filling the whole box. Each tooth is its own shape, since a whole gear isn't convex.
        Icon::Settings => {
            let c = at(7.0, 7.0);
            p.circle_stroke(c, 3.5 * u, Stroke::new(2.5 * u, color));
            for i in 0..6 {
                let out = Vec2::angled(TAU * (0.25 + i as f32 / 6.0)) * u;
                let side = out.rot90();
                p.add(Shape::convex_polygon(vec![c + out * 4.5 - side * 1.6, c + out * 7.0 - side * 1.1, c + out * 7.0 + side * 1.1, c + out * 4.5 + side * 1.6], color, Stroke::NONE));
            }
        }
        Icon::Search => {
            p.circle_stroke(at(6.0, 6.0), 3.75 * u, stroke);
            p.line_segment([at(9.0, 9.0), at(11.75, 11.75)], stroke);
        }
        Icon::Close => {
            p.line_segment([at(3.25, 3.25), at(10.75, 10.75)], stroke);
            p.line_segment([at(10.75, 3.25), at(3.25, 10.75)], stroke);
        }
        Icon::Plus => {
            p.line_segment([at(2.5, 7.0), at(11.5, 7.0)], stroke);
            p.line_segment([at(7.0, 2.5), at(7.0, 11.5)], stroke);
        }
        // Six dots, two across and three down. Its rect is narrow (10×14), so it's laid out from the centre.
        Icon::Grip => {
            let u = (rect.width() / 10.0).min(rect.height() / 14.0);
            for x in [-3.0, 1.0] {
                for y in [-5.0, -1.0, 3.0] {
                    let min = (rect.center() + vec2(x, y) * u).round();
                    p.rect_filled(Rect::from_min_size(min, Vec2::splat((2.0 * u).round())), 0.0, color);
                }
            }
        }
    }
}

/// The outline of a polygon with each corner cut back by `cut` and curved.
fn rounded(corners: &[Pos2], cut: f32) -> Vec<Pos2> {
    let n = corners.len();
    let corner = |i: usize| {
        let (before, v, after) = (corners[(i + n - 1) % n], corners[i], corners[(i + 1) % n]);
        let (a, b) = (v + (before - v).normalized() * cut, v + (after - v).normalized() * cut);
        [a, a.lerp(b, 0.5).lerp(v, 0.5), b]
    };
    (0..n).flat_map(corner).collect()
}

/// An icon to put in a button or hint, in the colour of the text beside it.
pub fn atom(icon: Icon) -> egui::Atom<'static> {
    egui::Atom::paint(Vec2::splat(SIZE), move |ui, a| paint(ui.painter(), a.rect, icon, a.fallback_text_color))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every icon draws something, and nothing spills out of the rect it was given.
    #[test]
    fn every_icon_stays_inside_its_rect() {
        let ctx = egui::Context::default();
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(200.0, 200.0));
        let out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), ..Default::default() }, |ui| {
            let p = ui.painter();
            let bounds = || {
                let mut all = Vec::new();
                p.for_each_shape(|s| all.push(s.shape.visual_bounding_rect()));
                all
            };
            for size in [vec2(12.0, 12.0), vec2(14.0, 14.0), vec2(28.0, 28.0), vec2(10.0, 14.0)] {
                let rect = Rect::from_min_size(pos2(50.0, 60.0), size);
                for icon in ALL {
                    let before = bounds().len();
                    paint(p, rect, icon, Color32::WHITE);
                    let drawn = bounds().split_off(before);
                    assert!(!drawn.is_empty(), "{icon:?} drew nothing at {size:?}");
                    for b in drawn {
                        assert!(rect.expand(1.0).contains_rect(b), "{icon:?} at {size:?} drew {b:?}, outside {rect:?}");
                    }
                }
            }
        });
        out.drop_without_applying_deltas();
    }
}
