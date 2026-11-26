//! The editor's look: a calm charcoal (or paper) background, cards, and a
//! "breath" teal accent; amber marks unsaved changes.

use std::sync::Arc;

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Frame, Margin,
    RichText, Shadow, Stroke, TextStyle, Theme, Ui, Visuals,
};

#[derive(Clone, Copy)]
pub struct Palette {
    pub bg: Color32,
    pub surface: Color32,
    pub raised: Color32,
    pub control: Color32,
    pub hover: Color32,
    pub border: Color32,
    pub text: Color32,
    pub weak: Color32,
    pub accent: Color32,
    pub accent_soft: Color32,
    pub on_accent: Color32,
    pub amber: Color32,
    pub good: Color32,
    pub bad: Color32,
    pub shadow: Color32,
}

pub const DARK: Palette = Palette {
    bg: Color32::from_rgb(0x0F, 0x11, 0x14),
    surface: Color32::from_rgb(0x17, 0x1A, 0x1F),
    raised: Color32::from_rgb(0x1E, 0x22, 0x28),
    control: Color32::from_rgb(0x27, 0x2C, 0x33),
    hover: Color32::from_rgb(0x31, 0x37, 0x40),
    border: Color32::from_rgb(0x2A, 0x2F, 0x37),
    text: Color32::from_rgb(0xE8, 0xEA, 0xED),
    weak: Color32::from_rgb(0x8E, 0x96, 0xA3),
    accent: Color32::from_rgb(0x2D, 0xC6, 0xB4),
    accent_soft: Color32::from_rgb(0x16, 0x3D, 0x3A),
    on_accent: Color32::from_rgb(0x05, 0x1A, 0x18),
    amber: Color32::from_rgb(0xF2, 0xA9, 0x3B),
    good: Color32::from_rgb(0x3D, 0xD6, 0x7E),
    bad: Color32::from_rgb(0xF0, 0x5D, 0x5E),
    shadow: Color32::from_black_alpha(100),
};

pub const LIGHT: Palette = Palette {
    bg: Color32::from_rgb(0xF1, 0xF2, 0xF4),
    surface: Color32::WHITE,
    raised: Color32::from_rgb(0xF8, 0xF9, 0xFA),
    control: Color32::from_rgb(0xE9, 0xEC, 0xEF),
    hover: Color32::from_rgb(0xDD, 0xE2, 0xE7),
    border: Color32::from_rgb(0xDC, 0xE0, 0xE5),
    text: Color32::from_rgb(0x1A, 0x1E, 0x24),
    weak: Color32::from_rgb(0x63, 0x6B, 0x78),
    accent: Color32::from_rgb(0x0E, 0x9A, 0x8A),
    accent_soft: Color32::from_rgb(0xD5, 0xF2, 0xEE),
    on_accent: Color32::WHITE,
    amber: Color32::from_rgb(0xC2, 0x6B, 0x00),
    good: Color32::from_rgb(0x16, 0xA3, 0x4A),
    bad: Color32::from_rgb(0xD9, 0x2D, 0x2D),
    shadow: Color32::from_black_alpha(26),
};

/// The user's appearance choices (Preferences).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    /// `None` uses the default teal (tuned for light and dark).
    pub accent: Option<Color32>,
    /// Corner radius of controls in points; cards and windows are rounder.
    pub corners: u8,
    /// Size of the whole interface, 1.0 = 100%.
    pub scale: f32,
    /// Fill slider tracks up to the handle.
    pub slider_fill: bool,
}

impl Default for Look {
    fn default() -> Self {
        Look {
            accent: None,
            corners: DEFAULT_CORNERS,
            scale: 1.0,
            slider_fill: true,
        }
    }
}

pub const DEFAULT_CORNERS: u8 = 7;
pub const MAX_CORNERS: u8 = 14;
pub const SCALES: [u32; 5] = [90, 100, 110, 125, 140];

/// Accent colour choices besides the default teal.
pub const ACCENTS: [(&str, Color32); 7] = [
    ("Blue", Color32::from_rgb(0x0A, 0x84, 0xFF)),
    ("Purple", Color32::from_rgb(0x8E, 0x5C, 0xF5)),
    ("Pink", Color32::from_rgb(0xE0, 0x4B, 0x8B)),
    ("Red", Color32::from_rgb(0xE5, 0x48, 0x4D)),
    ("Orange", Color32::from_rgb(0xF0, 0x8A, 0x24)),
    ("Green", Color32::from_rgb(0x2F, 0xB3, 0x4F)),
    ("Graphite", Color32::from_rgb(0x7D, 0x85, 0x91)),
];

pub fn default_accent(dark: bool) -> Color32 {
    if dark {
        DARK.accent
    } else {
        LIGHT.accent
    }
}

/// Black or white, whichever reads better on `bg`.
pub fn text_on(bg: Color32) -> Color32 {
    let lum = 0.2126 * f32::from(bg.r()) + 0.7152 * f32::from(bg.g()) + 0.0722 * f32::from(bg.b());
    if lum > 150.0 {
        Color32::from_rgb(0x10, 0x12, 0x14)
    } else {
        Color32::WHITE
    }
}

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let m = |x: u8, y: u8| (f32::from(x) * (1.0 - t) + f32::from(y) * t).round() as u8;
    Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

/// The palette for whichever theme `ui` shows, with the user's accent.
pub fn palette(ui: &Ui) -> Palette {
    let mut p = if ui.visuals().dark_mode { DARK } else { LIGHT };
    with_accent(&mut p, ui.visuals().selection.bg_fill);
    p
}

fn with_accent(p: &mut Palette, accent: Color32) {
    p.accent = accent;
    p.on_accent = text_on(accent);
    p.accent_soft = mix(p.surface, accent, 0.2);
}

pub fn semibold() -> FontFamily {
    FontFamily::Name("semibold".into())
}

/// Load the fonts (once) and style both themes for `look`.
pub fn install(ctx: &egui::Context, look: Look) {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "Inter".into(),
        Arc::new(FontData::from_static(include_bytes!(
            "../../assets/fonts/Inter-Regular.ttf"
        ))),
    );
    fonts.font_data.insert(
        "Inter-SemiBold".into(),
        Arc::new(FontData::from_static(include_bytes!(
            "../../assets/fonts/Inter-SemiBold.ttf"
        ))),
    );
    let fallbacks = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, "Inter".into());
    let mut bold = vec!["Inter-SemiBold".to_string()];
    bold.extend(fallbacks);
    fonts.families.insert(semibold(), bold);
    ctx.set_fonts(fonts);
    apply(ctx, look);
}

/// Restyle both themes for `look` (cheap; call whenever it changes).
pub fn apply(ctx: &egui::Context, look: Look) {
    for (theme, p) in [(Theme::Dark, DARK), (Theme::Light, LIGHT)] {
        ctx.style_mut_of(theme, |style| {
            style.visuals = visuals(theme, p, look);
            style.text_styles = [
                (TextStyle::Small, FontId::proportional(11.0)),
                (TextStyle::Body, FontId::proportional(13.5)),
                (TextStyle::Button, FontId::proportional(13.5)),
                (TextStyle::Heading, FontId::new(18.0, semibold())),
                (TextStyle::Monospace, FontId::monospace(12.5)),
            ]
            .into();
            let s = &mut style.spacing;
            s.item_spacing = egui::vec2(8.0, 6.0);
            s.button_padding = egui::vec2(10.0, 4.0);
            s.interact_size = egui::vec2(36.0, 24.0);
            s.slider_width = 150.0;
            s.slider_rail_height = 5.0;
            s.combo_width = 150.0;
            s.window_margin = Margin::same(18);
            s.menu_margin = Margin::same(6);
        });
    }
    ctx.set_zoom_factor(look.scale.clamp(0.75, 1.75));
}

fn visuals(theme: Theme, mut p: Palette, look: Look) -> Visuals {
    let mut v = match theme {
        Theme::Dark => Visuals::dark(),
        Theme::Light => Visuals::light(),
    };
    if let Some(a) = look.accent {
        with_accent(&mut p, a);
    }
    let r = CornerRadius::same;
    let c = look.corners.min(MAX_CORNERS);
    v.panel_fill = p.bg;
    v.window_fill = p.surface;
    v.window_stroke = Stroke::new(1.0, p.border);
    v.window_corner_radius = r(c + 7);
    v.menu_corner_radius = r(c + 3);
    v.window_shadow = Shadow {
        offset: [0, 12],
        blur: 32,
        spread: 0,
        color: p.shadow,
    };
    v.popup_shadow = Shadow {
        offset: [0, 6],
        blur: 18,
        spread: 0,
        color: p.shadow,
    };
    v.extreme_bg_color = match theme {
        Theme::Dark => Color32::from_rgb(0x0B, 0x0D, 0x10),
        Theme::Light => Color32::WHITE,
    };
    v.faint_bg_color = p.raised;
    v.warn_fg_color = p.amber;
    v.error_fg_color = p.bad;
    v.hyperlink_color = p.accent;
    v.selection.bg_fill = p.accent;
    v.selection.stroke = Stroke::new(1.0, p.on_accent);
    v.slider_trailing_fill = look.slider_fill;
    v.handle_shape = egui::style::HandleShape::Circle;
    v.striped = false;

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = p.surface;
    w.noninteractive.weak_bg_fill = p.surface;
    w.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    // Cards read their rounding from here (see `card`).
    w.noninteractive.corner_radius = r(c + 5);
    for (state, fill, stroke) in [
        (&mut w.inactive, p.control, Stroke::NONE),
        (
            &mut w.hovered,
            p.hover,
            Stroke::new(1.0, p.accent.gamma_multiply(0.5)),
        ),
        (&mut w.active, p.hover, Stroke::new(1.0, p.accent)),
        (&mut w.open, p.hover, Stroke::new(1.0, p.border)),
    ] {
        state.bg_fill = fill;
        state.weak_bg_fill = fill;
        state.bg_stroke = stroke;
        state.fg_stroke = Stroke::new(1.5, p.text);
        state.corner_radius = r(c);
        state.expansion = 0.0;
    }
    v
}

/// Corner radius of controls, from the user's setting.
pub fn control_radius(ui: &Ui) -> u8 {
    ui.visuals().widgets.inactive.corner_radius.nw
}

/// A card: the panel every group of settings sits on.
pub fn card(ui: &Ui) -> Frame {
    let p = palette(ui);
    Frame::new()
        .fill(p.surface)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(ui.visuals().widgets.noninteractive.corner_radius)
        .inner_margin(Margin::same(12))
        .shadow(Shadow {
            offset: [0, 2],
            blur: 10,
            spread: 0,
            color: p.shadow.gamma_multiply(0.45),
        })
}

pub fn title(text: impl Into<String>, size: f32) -> RichText {
    RichText::new(text).family(semibold()).size(size)
}

/// A small upper-case section heading.
pub fn section(ui: &Ui, text: &str) -> RichText {
    RichText::new(text.to_uppercase())
        .size(10.5)
        .family(semibold())
        .color(palette(ui).weak)
}

pub fn primary(ui: &Ui, text: &str) -> egui::Button<'static> {
    let p = palette(ui);
    egui::Button::new(
        RichText::new(text.to_string())
            .color(p.on_accent)
            .family(semibold()),
    )
    .fill(p.accent)
}

pub fn dot(ui: &mut Ui, color: Color32, size: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), size / 2.0, color);
}
