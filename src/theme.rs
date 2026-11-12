//! Visual language for Syncora: a calm, compact interface that works in both
//! light and dark mode.

use egui::{Color32, CornerRadius, FontId, Shadow, Stroke, TextStyle, Vec2};

#[derive(Clone, Copy)]
pub struct Palette {
    pub dark: bool,
    pub bg: Color32,
    pub sidebar: Color32,
    pub card: Color32,
    pub card_hover: Color32,
    pub border: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub faint: Color32,
    pub accent: Color32,
    pub accent_soft: Color32,
    pub success: Color32,
    pub warning: Color32,
    pub danger: Color32,
    pub track: Color32,
    pub input: Color32,
}

pub fn palette(dark: bool) -> Palette {
    if dark {
        Palette {
            dark,
            bg: Color32::from_rgb(0x10, 0x12, 0x16),
            sidebar: Color32::from_rgb(0x15, 0x18, 0x1E),
            card: Color32::from_rgb(0x1A, 0x1E, 0x25),
            card_hover: Color32::from_rgb(0x20, 0x25, 0x2E),
            border: Color32::from_rgb(0x27, 0x2D, 0x38),
            text: Color32::from_rgb(0xE8, 0xEB, 0xF0),
            muted: Color32::from_rgb(0x9A, 0xA3, 0xB2),
            faint: Color32::from_rgb(0x6B, 0x74, 0x84),
            accent: Color32::from_rgb(0x6C, 0x8C, 0xFF),
            accent_soft: Color32::from_rgb(0x22, 0x2B, 0x45),
            success: Color32::from_rgb(0x4A, 0xDE, 0x80),
            warning: Color32::from_rgb(0xFB, 0xBF, 0x24),
            danger: Color32::from_rgb(0xF8, 0x71, 0x71),
            track: Color32::from_rgb(0x2A, 0x31, 0x3D),
            input: Color32::from_rgb(0x12, 0x15, 0x1A),
        }
    } else {
        Palette {
            dark,
            bg: Color32::from_rgb(0xF4, 0xF6, 0xFA),
            sidebar: Color32::from_rgb(0xFF, 0xFF, 0xFF),
            card: Color32::from_rgb(0xFF, 0xFF, 0xFF),
            card_hover: Color32::from_rgb(0xF7, 0xF9, 0xFD),
            border: Color32::from_rgb(0xE3, 0xE7, 0xEF),
            text: Color32::from_rgb(0x1B, 0x22, 0x30),
            muted: Color32::from_rgb(0x66, 0x70, 0x85),
            faint: Color32::from_rgb(0x98, 0xA2, 0xB3),
            accent: Color32::from_rgb(0x3B, 0x6C, 0xF6),
            accent_soft: Color32::from_rgb(0xE8, 0xEE, 0xFF),
            success: Color32::from_rgb(0x16, 0xA3, 0x4A),
            warning: Color32::from_rgb(0xD9, 0x77, 0x06),
            danger: Color32::from_rgb(0xDC, 0x26, 0x26),
            track: Color32::from_rgb(0xEA, 0xED, 0xF3),
            input: Color32::from_rgb(0xF7, 0xF9, 0xFC),
        }
    }
}

/// Install the style for both themes, then select the requested one.
///
/// egui 0.37 keeps separate light/dark styles, so we configure both. This makes
/// the manual theme switch instant and keeps popups/menus consistent.
pub fn apply(ctx: &egui::Context, dark: bool) {
    for (theme, is_dark) in [(egui::Theme::Light, false), (egui::Theme::Dark, true)] {
        let mut style = (*ctx.style_of(theme)).clone();
        style.visuals = build_visuals(is_dark);
        style.animation_time = 0.12;
        style.spacing.item_spacing = Vec2::new(8.0, 8.0);
        style.spacing.button_padding = Vec2::new(12.0, 6.0);
        style.spacing.interact_size = Vec2::new(40.0, 30.0);
        style.spacing.window_margin = egui::Margin::same(16);
        style.spacing.menu_margin = egui::Margin::same(8);
        style.spacing.slider_width = 140.0;
        style.spacing.combo_width = 180.0;
        style.text_styles = [
            (TextStyle::Heading, FontId::proportional(24.0)),
            (TextStyle::Body, FontId::proportional(14.0)),
            (TextStyle::Monospace, FontId::monospace(12.5)),
            (TextStyle::Button, FontId::proportional(14.0)),
            (TextStyle::Small, FontId::proportional(11.5)),
        ]
        .into();
        ctx.set_style_of(theme, std::sync::Arc::new(style));
    }

    ctx.set_theme(if dark {
        egui::ThemePreference::Dark
    } else {
        egui::ThemePreference::Light
    });
}

fn build_visuals(dark: bool) -> egui::Visuals {
    let p = palette(dark);
    let mut visuals = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };

    visuals.panel_fill = p.bg;
    visuals.window_fill = p.card;
    visuals.extreme_bg_color = p.input;
    visuals.faint_bg_color = p.card_hover;
    visuals.code_bg_color = p.input;
    visuals.text_edit_bg_color = Some(p.input);
    visuals.window_stroke = Stroke::new(1.0, p.border);
    visuals.window_corner_radius = CornerRadius::same(14);
    visuals.menu_corner_radius = CornerRadius::same(10);
    visuals.hyperlink_color = p.accent;
    visuals.warn_fg_color = p.warning;
    visuals.error_fg_color = p.danger;
    visuals.selection.bg_fill = p.accent.gamma_multiply(0.35);
    visuals.selection.stroke = Stroke::new(1.0, p.text);
    visuals.striped = false;
    visuals.window_highlight_topmost = false;

    let widgets = &mut visuals.widgets;
    widgets.noninteractive.bg_fill = p.card;
    widgets.noninteractive.weak_bg_fill = p.card;
    widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    widgets.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    widgets.noninteractive.corner_radius = CornerRadius::same(8);

    widgets.inactive.bg_fill = p.card;
    widgets.inactive.weak_bg_fill = p.card;
    widgets.inactive.bg_stroke = Stroke::new(1.0, p.border);
    widgets.inactive.fg_stroke = Stroke::new(1.0, p.text);
    widgets.inactive.corner_radius = CornerRadius::same(8);

    widgets.hovered.bg_fill = p.card_hover;
    widgets.hovered.weak_bg_fill = p.card_hover;
    widgets.hovered.bg_stroke = Stroke::new(1.0, p.accent.gamma_multiply(0.7));
    widgets.hovered.fg_stroke = Stroke::new(1.0, p.text);
    widgets.hovered.corner_radius = CornerRadius::same(8);

    widgets.active.bg_fill = p.accent_soft;
    widgets.active.weak_bg_fill = p.accent_soft;
    widgets.active.bg_stroke = Stroke::new(1.0, p.accent);
    widgets.active.fg_stroke = Stroke::new(1.0, p.text);
    widgets.active.corner_radius = CornerRadius::same(8);

    widgets.open = widgets.hovered;

    visuals.window_shadow = Shadow {
        offset: [0, 10],
        blur: 30,
        spread: 0,
        color: Color32::from_black_alpha(if dark { 120 } else { 36 }),
    };
    visuals.popup_shadow = Shadow {
        offset: [0, 6],
        blur: 18,
        spread: 0,
        color: Color32::from_black_alpha(if dark { 110 } else { 30 }),
    };
    visuals
}
