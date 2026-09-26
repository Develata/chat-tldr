use eframe::egui::{self, Color32, FontData, FontDefinitions, FontFamily, FontId, TextStyle};
pub const ACCENT: Color32 = Color32::from_rgb(44, 103, 180);

pub fn install(ctx: &egui::Context, dark: bool) {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "noto-sc".into(),
        std::sync::Arc::new(FontData::from_static(include_bytes!(
            "../assets/fonts/NotoSansCJKsc-Regular.otf"
        ))),
    );
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .push("noto-sc".into());
    }
    ctx.set_fonts(fonts);
    theme(ctx, dark);
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        ctx.style_mut_of(theme, |style| {
            style.spacing.item_spacing = egui::vec2(10.0, 10.0);
            style.spacing.button_padding = egui::vec2(12.0, 8.0);
            style.spacing.interact_size.y = 34.0;
            style
                .text_styles
                .insert(TextStyle::Body, FontId::proportional(16.0));
            style
                .text_styles
                .insert(TextStyle::Button, FontId::proportional(15.0));
            style
                .text_styles
                .insert(TextStyle::Small, FontId::proportional(13.0));
            style
                .text_styles
                .insert(TextStyle::Heading, FontId::proportional(24.0));
        });
    }
}

pub fn theme(ctx: &egui::Context, dark: bool) {
    ctx.set_theme(if dark {
        egui::Theme::Dark
    } else {
        egui::Theme::Light
    });
    let mut visuals = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    visuals.override_text_color = Some(if dark {
        Color32::from_rgb(224, 227, 233)
    } else {
        Color32::from_rgb(37, 46, 58)
    });
    visuals.weak_text_color = Some(if dark {
        Color32::from_rgb(168, 176, 188)
    } else {
        Color32::from_rgb(94, 106, 121)
    });
    if !dark {
        visuals.panel_fill = Color32::from_rgb(248, 249, 251);
        visuals.window_fill = Color32::from_rgb(252, 252, 253);
    }
    visuals.selection.bg_fill = if dark {
        Color32::from_rgb(39, 71, 110)
    } else {
        Color32::from_rgb(219, 232, 249)
    };
    visuals.selection.stroke.color = if dark { Color32::WHITE } else { ACCENT };
    ctx.set_visuals(visuals);
}
