//! Controls bound to instrument parameters. Each reads its value from the
//! editor and, when the user changes it, calls `Editor::set`, which updates
//! the local copy, sends the DT1 and records undo.

use eframe::egui::{
    self, emath::Rot2, pos2, vec2, Align2, Color32, ComboBox, FontId, Pos2, Rect, Response,
    RichText, Sense, Shape, Slider, SliderClamping, Stroke, Ui,
};

use aerophone_core::params::{Kind, Param};

use super::theme::{self, palette};
use crate::editor::{Editor, Target};

/// One parameter's control, sized for a settings grid.
pub fn control(ui: &mut Ui, ed: &mut Editor, t: Target, block: &str, p: &'static Param) {
    let Some(v) = ed.value(t, block, p) else {
        ui.label(RichText::new("–").weak());
        return;
    };
    let id = ui.scope_id().with((block, p.off, p.fx));
    if p.kind == Kind::Switch
        || (p.opts.len() == 2 && p.min == 0 && p.max == 1 && p.is_choice() && is_onoff(p))
    {
        let mut on = v != 0;
        let r = toggle(ui, &mut on);
        if r.changed() {
            ed.set(t, block, p, i32::from(on));
        }
        return;
    }
    if p.is_choice() {
        let mut sel = v;
        ComboBox::from_id_salt(id)
            .selected_text(p.show(v))
            .height(320.0)
            .show_ui(ui, |ui| {
                for (i, o) in p.opts.iter().enumerate() {
                    ui.selectable_value(&mut sel, i as i32, o);
                }
            });
        if sel != v {
            ed.set(t, block, p, sel);
        }
        return;
    }
    let mut x = v;
    let r = ui.add(
        Slider::new(&mut x, p.min..=p.max.max(p.min))
            .clamping(SliderClamping::Edits)
            .custom_formatter(|n, _| p.show(n.round() as i32))
            .custom_parser(move |s| parse_value(p, s)),
    );
    if r.changed() && x != v {
        ed.set(t, block, p, x);
    }
    if r.double_clicked() {
        if let Some(d) = default_of(p) {
            ed.set(t, block, p, d);
        }
    }
}

fn is_onoff(p: &Param) -> bool {
    matches!(p.opts[0].to_lowercase().as_str(), "off" | "0")
}

/// Accept a typed number (in display units for plain values).
fn parse_value(p: &Param, s: &str) -> Option<f64> {
    let s = s.trim().trim_start_matches('+');
    let num: String = s
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '-' || *c == '.')
        .collect();
    let x: f64 = num.parse().ok()?;
    if let Some(aerophone_core::params::Format::Lin { add, mul, .. }) = &p.fmt {
        if *mul != 0.0 {
            return Some((x / mul - add).round());
        }
    }
    Some(x)
}

/// A neutral value to reset to on double-click: the centre of signed ranges.
pub fn default_of(p: &Param) -> Option<i32> {
    (p.min < 0 && p.max > 0).then_some(0)
}

/// A label and control as one row of a two-column `Grid`.
pub fn row(ui: &mut Ui, ed: &mut Editor, t: Target, block: &str, p: &'static Param) {
    let label = p.label();
    let r = ui.add(egui::Label::new(RichText::new(label).color(palette(ui).weak)).truncate());
    if label.len() > 18 {
        r.on_hover_text(label);
    }
    control(ui, ed, t, block, p);
    ui.end_row();
}

/// A grid of parameters.
pub fn grid<'a>(
    ui: &mut Ui,
    ed: &mut Editor,
    t: Target,
    block: &str,
    params: impl IntoIterator<Item = &'a Param>,
) where
    'a: 'static,
{
    let params: Vec<&'static Param> = params.into_iter().collect();
    let Some(first) = params.first() else { return };
    let salt = (block.to_string(), first.off, first.fx, params.len());
    ui.spacing_mut().slider_width = 120.0;
    ui.spacing_mut().combo_width = 140.0;
    egui::Grid::new(salt)
        .num_columns(2)
        .spacing(vec2(12.0, 7.0))
        .min_col_width(110.0)
        .show(ui, |ui| {
            for p in params {
                if !ed.hides(p) {
                    row(ui, ed, t, block, p);
                }
            }
        });
}

/// An on/off switch.
pub fn toggle(ui: &mut Ui, on: &mut bool) -> Response {
    let p = palette(ui);
    let size = vec2(34.0, 19.0);
    let (rect, mut r) = ui.allocate_exact_size(size, Sense::click());
    if r.clicked() {
        *on = !*on;
        r.mark_changed();
    }
    let t = ui.ctx().animate_bool_responsive(r.id, *on);
    let bg = if *on {
        p.accent
    } else if r.hovered() {
        p.hover
    } else {
        p.control
    };
    let painter = ui.painter();
    painter.rect_filled(rect, size.y / 2.0, bg);
    let x = egui::lerp(rect.left() + size.y / 2.0..=rect.right() - size.y / 2.0, t);
    let knob = if *on { p.on_accent } else { p.weak };
    painter.circle_filled(pos2(x, rect.center().y), size.y / 2.0 - 3.0, knob);
    r
}

/// Pixels of drag for a knob's full range (Preferences ▸ Knob speed).
pub static KNOB_PIXELS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(160);

/// A rotary knob with its label and value underneath. Drag up/down or
/// scroll to change; double-click resets signed values to the centre.
pub fn knob(ui: &mut Ui, ed: &mut Editor, t: Target, block: &str, p: &'static Param, label: &str) {
    let v = ed.value(t, block, p);
    let show = |x: i32| p.show(x);
    ui.vertical_centered(|ui| {
        if let Some(n) = dial(ui, 38.0, v, (p.min, p.max), default_of(p), &show, label) {
            ed.set(t, block, p, n);
        }
    });
}

/// The drawing and handling of a knob of `size` points for a value in
/// `range`, labelled underneath. Returns a new value when it's changed.
/// With `v` None it shows an empty, inactive knob.
pub fn dial(
    ui: &mut Ui,
    size: f32,
    v: Option<i32>,
    (min, max): (i32, i32),
    reset: Option<i32>,
    show: &dyn Fn(i32) -> String,
    label: &str,
) -> Option<i32> {
    let pal = palette(ui);
    let scale = (size / 38.0).sqrt();
    ui.spacing_mut().item_spacing.y = 1.0 * scale;
    let (rect, r) = ui.allocate_exact_size(vec2(size, size), Sense::click_and_drag());
    let range = (max - min).max(1) as f32;
    let mut new = None;
    if let Some(v) = v {
        let acc_id = r.id.with("acc");
        if r.drag_started() {
            ui.data_mut(|d| d.insert_temp(acc_id, v as f32));
        }
        if r.dragged() {
            let fine = if ui.input(|i| i.modifiers.shift) {
                0.2
            } else {
                1.0
            };
            let px = KNOB_PIXELS
                .load(std::sync::atomic::Ordering::Relaxed)
                .max(20) as f32;
            let delta = -r.drag_delta().y * range / px * fine;
            let acc = ui.data_mut(|d| {
                let a = d.get_temp_mut_or(acc_id, v as f32);
                *a = (*a + delta).clamp(min as f32, max as f32);
                *a
            });
            new = Some(acc.round() as i32);
        }
        if r.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll.abs() > 0.5 {
                let step = (range / 64.0).max(1.0) * scroll.signum();
                new = Some((v as f32 + step).round() as i32);
            }
        }
        if r.double_clicked() {
            new = reset;
        }
    }
    let new = new
        .map(|n| n.clamp(min, max.max(min)))
        .filter(|&n| Some(n) != v);
    let shown = new.or(v);
    // Draw: track arc, value arc from the zero point, pointer.
    let c = rect.center();
    let thick = 4.0 * scale;
    let rad = size / 2.0 - thick * 0.75;
    let a0 = std::f32::consts::PI * 0.75;
    let sweep = std::f32::consts::PI * 1.5;
    let frac = |x: i32| (x - min) as f32 / range;
    let zero = if min < 0 && max > 0 { frac(0) } else { 0.0 };
    let arc = |from: f32, to: f32| -> Vec<Pos2> {
        let n = 48;
        (0..=n)
            .map(|i| {
                let f = from + (to - from) * i as f32 / n as f32;
                let a = a0 + sweep * f;
                c + vec2(a.cos(), a.sin()) * rad
            })
            .collect()
    };
    let painter = ui.painter();
    painter.add(Shape::line(arc(0.0, 1.0), Stroke::new(thick, pal.control)));
    painter.circle_filled(c, rad - thick * 1.5, pal.raised);
    if let Some(s) = shown {
        let f = frac(s);
        let (lo, hi) = if f < zero { (f, zero) } else { (zero, f) };
        let col = if r.hovered() || r.dragged() {
            pal.accent
        } else {
            pal.accent.gamma_multiply(0.85)
        };
        painter.add(Shape::line(arc(lo, hi), Stroke::new(thick, col)));
        let a = a0 + sweep * f;
        painter.line_segment(
            [
                c + Rot2::from_angle(a) * vec2(rad - thick * 3.0, 0.0),
                c + Rot2::from_angle(a) * vec2(rad - thick * 1.25, 0.0),
            ],
            Stroke::new(2.0 * scale, pal.text),
        );
        r.on_hover_text(format!(
            "{label}: {}  (drag, scroll; Shift for fine)",
            show(s)
        ));
    }
    ui.label(RichText::new(label).size(10.5 * scale).color(pal.weak));
    ui.label(RichText::new(shown.map_or("–".into(), show)).size(11.5 * scale));
    new
}

/// A vertical fader for levels.
pub fn fader(ui: &mut Ui, ed: &mut Editor, t: Target, block: &str, p: &'static Param, height: f32) {
    let Some(v) = ed.value(t, block, p) else {
        return;
    };
    let mut x = v;
    ui.spacing_mut().slider_width = height;
    let r = ui.add(
        Slider::new(&mut x, p.min..=p.max)
            .vertical()
            .show_value(false)
            .clamping(SliderClamping::Edits),
    );
    if r.changed() && x != v {
        ed.set(t, block, p, x);
    }
}

/// A segmented choice (like tabs) between a few options.
pub fn segmented<T: PartialEq + Copy>(ui: &mut Ui, value: &mut T, options: &[(T, &str)]) -> bool {
    let p = palette(ui);
    let mut changed = false;
    egui::Frame::new()
        .fill(p.control)
        .corner_radius(theme::control_radius(ui).saturating_add(2))
        .inner_margin(3)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for (v, label) in options {
                    let selected = *value == *v;
                    let text = if selected {
                        RichText::new(*label)
                            .color(p.on_accent)
                            .family(theme::semibold())
                    } else {
                        RichText::new(*label)
                    };
                    let b = egui::Button::new(text)
                        .fill(if selected {
                            p.accent
                        } else {
                            Color32::TRANSPARENT
                        })
                        .corner_radius(7);
                    if ui.add(b).clicked() && !selected {
                        *value = *v;
                        changed = true;
                    }
                }
            });
        });
    changed
}

// ---- envelope editor ------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Env {
    Amp,
    Filter,
    Pitch,
}

/// The ZEN-Core envelope of `block`, drawn and editable by dragging its
/// points: sideways changes the time, up and down the level.
pub fn envelope(ui: &mut Ui, ed: &mut Editor, t: Target, block: &str, env: Env) {
    let pal = palette(ui);
    let table = ed.image(t).block(block).map(|b| b.table);
    let Some(table) = table else { return };
    let time = |i: usize| table.named(&format!("Time{i}"));
    let (levels, lmin, lmax): (Vec<Option<&'static Param>>, f32, f32) = match env {
        Env::Amp => (
            vec![
                None,
                table.named("Level1"),
                table.named("Level2"),
                table.named("Level3"),
                None,
            ],
            0.0,
            1023.0,
        ),
        Env::Filter => (
            (0..5).map(|i| table.named(&format!("Level{i}"))).collect(),
            0.0,
            1023.0,
        ),
        Env::Pitch => (
            (0..5).map(|i| table.named(&format!("Level{i}"))).collect(),
            -511.0,
            511.0,
        ),
    };
    let times: Vec<Option<&'static Param>> = (1..=4).map(time).collect();
    let get = |ed: &Editor, p: Option<&'static Param>| p.and_then(|p| ed.value(t, block, p));
    let tv: Vec<f32> = times
        .iter()
        .map(|&p| get(ed, p).unwrap_or(0) as f32)
        .collect();
    let lv: Vec<f32> = levels
        .iter()
        .map(|&p| get(ed, p).map_or(if lmin < 0.0 { 0.0 } else { lmin }, |v| v as f32))
        .collect();

    let width = ui.available_width().clamp(260.0, 640.0);
    let (rect, _) = ui.allocate_exact_size(vec2(width, 150.0), Sense::hover());
    let inner = rect.shrink2(vec2(10.0, 12.0));
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 10.0, pal.raised);
    let gap = inner.width() * 0.12; // the sustain stretch
    let seg = (inner.width() - gap) / 4.0;
    let y_of = |l: f32| egui::remap(l, lmin..=lmax, inner.bottom()..=inner.top());
    let mut xs = vec![inner.left()];
    for (i, tt) in tv.iter().enumerate() {
        let x = xs[i] + tt / 1023.0 * seg + if i == 3 { gap } else { 0.0 };
        xs.push(x);
    }
    let pts: Vec<Pos2> = (0..5).map(|i| pos2(xs[i], y_of(lv[i]))).collect();
    if lmin < 0.0 {
        painter.line_segment(
            [
                pos2(inner.left(), y_of(0.0)),
                pos2(inner.right(), y_of(0.0)),
            ],
            Stroke::new(1.0, pal.border),
        );
    }
    // Sustain hold line.
    let sus_end = pos2(xs[3] + gap, pts[3].y);
    let mut line = pts[..4].to_vec();
    line.push(sus_end);
    line.push(pts[4]);
    let base = if lmin < 0.0 {
        y_of(0.0)
    } else {
        inner.bottom()
    };
    // Soft fill under the curve, as vertical strips.
    for w in line.windows(2) {
        let (a, b) = (w[0], w[1]);
        painter.add(Shape::convex_polygon(
            vec![a, b, pos2(b.x, base), pos2(a.x, base)],
            pal.accent.gamma_multiply(0.12),
            Stroke::NONE,
        ));
    }
    painter.add(Shape::line(line, Stroke::new(2.0, pal.accent)));
    painter.line_segment(
        [pos2(xs[3], inner.top()), pos2(xs[3], inner.bottom())],
        Stroke::new(1.0, pal.border),
    );
    painter.text(
        pos2(xs[3] + gap / 2.0, inner.top()),
        Align2::CENTER_TOP,
        "sustain",
        FontId::proportional(10.0),
        pal.weak,
    );

    // Draggable points 1..=4 (point 0 is the start).
    for i in 1..5 {
        let id = ui.scope_id().with((block, "envpt", i));
        let hit = Rect::from_center_size(pts[i], vec2(16.0, 16.0));
        let r = ui.interact(hit, id, Sense::drag());
        let hot = r.hovered() || r.dragged();
        if r.dragged() {
            let d = r.drag_delta();
            let tp = times[i - 1];
            let lp = levels[i];
            let acc = ui.data_mut(|m| {
                let a = m.get_temp_mut_or(id, (tv[i - 1], lv[i]));
                a.0 = (a.0 + d.x / seg * 1023.0).clamp(0.0, 1023.0);
                a.1 = (a.1 - d.y / inner.height() * (lmax - lmin)).clamp(lmin, lmax);
                *a
            });
            if let Some(tp) = tp {
                ed.set(t, block, tp, acc.0.round() as i32);
            }
            if let Some(lp) = lp {
                ed.set(t, block, lp, acc.1.round() as i32);
            }
        } else {
            ui.data_mut(|m| m.remove::<(f32, f32)>(id));
        }
        painter.circle(
            pts[i],
            if hot { 6.0 } else { 4.5 },
            if hot { pal.accent } else { pal.surface },
            Stroke::new(2.0, pal.accent),
        );
        if hot {
            let tl = times[i - 1].map(|p| format!("T{i} {}", p.show(tv[i - 1] as i32)));
            let ll = levels[i].map(|p| format!("L{i} {}", p.show(lv[i] as i32)));
            let text = [tl, ll]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join("  ");
            r.on_hover_text(text);
        }
    }
}

// ---- controller curve -----------------------------------------------------

/// Approximate shapes of the 12 assign curves (Linear, Exp L..H, Log L..H, S, Reverse S, Step).
pub fn curve(c: i32, x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    let s = x * x * (3.0 - 2.0 * x);
    match c {
        1 => x.powf(1.5),
        2 => x.powf(2.0),
        3 => x.powf(2.6),
        4 => x.powf(3.5),
        5 => 1.0 - (1.0 - x).powf(1.5),
        6 => 1.0 - (1.0 - x).powf(2.0),
        7 => 1.0 - (1.0 - x).powf(2.6),
        8 => 1.0 - (1.0 - x).powf(3.5),
        9 => s,
        10 => (2.0 * x - s).clamp(0.0, 1.0),
        11 => ((x * 4.0).floor() / 3.0).min(1.0),
        _ => x,
    }
}

/// Draw an assign slot's response: input range → output range through the curve.
pub fn curve_graph(
    ui: &mut Ui,
    in_min: i32,
    in_max: i32,
    out_min: i32,
    out_max: i32,
    shape: i32,
    live: Option<u8>,
) {
    let pal = palette(ui);
    let (rect, _) = ui.allocate_exact_size(vec2(120.0, 70.0), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 8.0, pal.raised);
    let r = rect.shrink(6.0);
    let map = |v: i32| -> f32 {
        let (lo, hi) = (in_min.min(in_max), in_max.max(in_min));
        if hi == lo {
            return 0.0;
        }
        ((v - lo) as f32 / (hi - lo) as f32).clamp(0.0, 1.0)
    };
    let out = |f: f32| out_min as f32 + (out_max - out_min) as f32 * curve(shape, f);
    let to_screen = |inp: f32, o: f32| {
        pos2(
            r.left() + inp / 127.0 * r.width(),
            r.bottom() - o / 127.0 * r.height(),
        )
    };
    let pts: Vec<Pos2> = (0..=64)
        .map(|i| {
            let v = i as f32 * 127.0 / 64.0;
            to_screen(v, out(map(v.round() as i32)))
        })
        .collect();
    painter.add(Shape::line(pts, Stroke::new(2.0, pal.accent)));
    if let Some(b) = live {
        let p = to_screen(f32::from(b), out(map(i32::from(b))));
        painter.circle_filled(p, 4.0, pal.amber);
    }
}
