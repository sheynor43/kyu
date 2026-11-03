use egui::{Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, Stroke, TextStyle};
use std::sync::Arc;

use crate::model::Queue;

pub const BG_DEEP: Color32 = Color32::from_rgb(0x0f, 0x0f, 0x10);
pub const BG: Color32 = Color32::from_rgb(0x14, 0x14, 0x15);
pub const CARD: Color32 = Color32::from_rgb(0x19, 0x19, 0x1b);
pub const RAISED: Color32 = Color32::from_rgb(0x23, 0x23, 0x25);
pub const TEXT: Color32 = Color32::from_rgb(0xed, 0xed, 0xed);
pub const TEXT2: Color32 = Color32::from_rgb(0xa1, 0xa1, 0xa6);
pub const MUTED: Color32 = Color32::from_rgb(0x85, 0x85, 0x8b);
pub const HAIRLINE: Color32 = Color32::from_rgba_premultiplied(19, 19, 19, 20);
/// osu! pink, used for the selection and the "watched" action only.
pub const ACCENT: Color32 = Color32::from_rgb(0xff, 0x66, 0xaa);
pub const ON_ACCENT: Color32 = Color32::from_rgb(0x14, 0x14, 0x15);
pub const POSITIVE: Color32 = Color32::from_rgb(0x4c, 0xc3, 0x8a);
pub const NEGATIVE: Color32 = Color32::from_rgb(0xe5, 0x48, 0x4d);

pub fn queue_dot(q: Queue) -> Color32 {
    match q {
        Queue::Paid => Color32::from_rgb(0xe0, 0xa4, 0x58),
        Queue::Points => Color32::from_rgb(0x6c, 0xc4, 0xb8),
        Queue::Free => MUTED,
    }
}

pub fn hairline() -> Stroke {
    Stroke::new(1.0, HAIRLINE)
}

pub mod icon {
    pub const CHECK: &str = "\u{e070}";
    pub const X: &str = "\u{e1b2}";
    pub const SETTINGS: &str = "\u{e158}";
    pub const TROPHY: &str = "\u{e377}";
    pub const USER: &str = "\u{e19f}";
    pub const FOLDER: &str = "\u{e247}";
    pub const PLUS: &str = "\u{e141}";
    pub const UNDO: &str = "\u{e2a1}";
    pub const EYE: &str = "\u{e0be}";
    pub const EYE_OFF: &str = "\u{e0bf}";
    pub const COPY: &str = "\u{e0a2}";
    pub const TRASH: &str = "\u{e18e}";
}

pub fn medium() -> FontFamily {
    FontFamily::Name("medium".into())
}

pub fn icons() -> FontFamily {
    FontFamily::Name("icons".into())
}

pub fn icon_font(size: f32) -> FontId {
    FontId::new(size, icons())
}

pub fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let defaults = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let add = |f: &mut FontDefinitions, name: &str, data: &'static [u8]| {
        f.font_data.insert(name.into(), Arc::new(FontData::from_static(data)));
    };
    add(&mut fonts, "inter", include_bytes!("../assets/fonts/Inter-Regular.ttf"));
    add(&mut fonts, "inter-m", include_bytes!("../assets/fonts/Inter-Medium.ttf"));
    add(&mut fonts, "mono", include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf"));
    add(&mut fonts, "lucide", include_bytes!("../assets/fonts/lucide.ttf"));

    let chain = |names: &[&str]| -> Vec<String> {
        names.iter().map(|s| s.to_string()).chain(defaults.iter().cloned()).collect()
    };
    fonts.families.insert(FontFamily::Proportional, chain(&["inter"]));
    fonts.families.insert(medium(), chain(&["inter-m"]));
    fonts.families.insert(FontFamily::Monospace, chain(&["mono", "inter"]));
    fonts.families.insert(icons(), vec!["lucide".into()]);
    ctx.set_fonts(fonts);

    ctx.all_styles_mut(|s| {
        s.text_styles = [
            (TextStyle::Small, FontId::proportional(12.0)),
            (TextStyle::Body, FontId::proportional(14.0)),
            (TextStyle::Button, FontId::proportional(14.0)),
            (TextStyle::Heading, FontId::new(20.0, medium())),
            (TextStyle::Monospace, FontId::monospace(12.0)),
        ]
        .into();
        s.spacing.item_spacing = egui::vec2(8.0, 8.0);
        s.spacing.button_padding = egui::vec2(12.0, 6.0);
        s.spacing.interact_size.y = 32.0;
        s.spacing.window_margin = Margin::same(24);

        let v = &mut s.visuals;
        v.dark_mode = true;
        v.panel_fill = BG;
        v.window_fill = CARD;
        v.window_stroke = hairline();
        v.window_corner_radius = CornerRadius::same(16);
        v.window_shadow = egui::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(96) };
        v.popup_shadow = egui::Shadow { offset: [0, 4], blur: 12, spread: 0, color: Color32::from_black_alpha(80) };
        v.menu_corner_radius = CornerRadius::same(10);
        v.extreme_bg_color = BG_DEEP;
        v.faint_bg_color = CARD;
        v.override_text_color = None;
        v.hyperlink_color = TEXT2;
        v.selection.bg_fill = ACCENT.linear_multiply(0.35);
        v.selection.stroke = Stroke::new(1.0, ACCENT);
        v.text_cursor.stroke = Stroke::new(1.5, ACCENT);

        let w = &mut v.widgets;
        for (st, bg, fg) in [
            (&mut w.noninteractive, CARD, TEXT2),
            (&mut w.inactive, RAISED, TEXT),
            (&mut w.hovered, Color32::from_rgb(0x2b, 0x2b, 0x2e), TEXT),
            (&mut w.active, Color32::from_rgb(0x30, 0x30, 0x33), TEXT),
            (&mut w.open, RAISED, TEXT),
        ] {
            st.bg_fill = bg;
            st.weak_bg_fill = bg;
            st.bg_stroke = hairline();
            st.fg_stroke = Stroke::new(1.5, fg);
            st.corner_radius = CornerRadius::same(8);
            st.expansion = 0.0;
        }
        w.noninteractive.bg_stroke = hairline();
        w.hovered.bg_stroke = Stroke::new(1.0, Color32::from_white_alpha(28));
    });
}
