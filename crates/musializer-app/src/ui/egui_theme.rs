//! Shared dock and welcome appearance, derived solely from workstation roles.
use super::theme::UiTheme;
use egui::{Context, Stroke};

/// Apply the selected skin without changing the renderer or document styling.
///
/// Every colour and size here is read from [`super::theme`] (WX-5a): the dock and
/// the welcome picker draw with the numbers the raylib chrome
/// draws with, in the same face at the same type scale. Before WX-5a this
/// function carried its own hard-coded `Color32`s, and the dark ink, the light
/// rule and both selection fills had already drifted from the chrome's.
pub fn apply_theme(ctx: &Context, theme: UiTheme) {
    use super::theme::{color32::of, metric};
    install_interface_face(ctx);
    let p = theme.palette();
    let dark = theme == UiTheme::BlackAmber;
    let mut style = egui::Style::default();
    let mut visuals = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    let ink = of(p.ui_ink);
    let rule = of(p.ui_rule);
    let edge = of(p.ui_control_edge);
    let radius = egui::CornerRadius::same(metric::UI_RADIUS as u8);
    visuals.override_text_color = Some(ink);
    visuals.weak_text_color = Some(of(p.ui_muted));
    visuals.panel_fill = of(p.ui_surface);
    visuals.window_fill = of(p.ui_surface);
    visuals.extreme_bg_color = of(p.ui_raised);
    visuals.faint_bg_color = of(p.ui_raised);
    visuals.code_bg_color = of(p.ui_raised);
    // A selection is the selected tint, as in the raylib chrome; the accent is
    // not a selection colour. egui draws a selected label's *text* in
    // `selection.stroke`, so that is the ink, not the outline tone.
    visuals.selection.bg_fill = of(p.ui_selected);
    visuals.selection.stroke = Stroke::new(1.0, ink);
    visuals.hyperlink_color = ink;
    visuals.warn_fg_color = of(p.ui_warning);
    visuals.error_fg_color = of(p.ui_danger);
    visuals.text_cursor.stroke = Stroke::new(2.0, ink);
    visuals.window_corner_radius = radius;
    visuals.menu_corner_radius = radius;
    visuals.window_shadow = egui::epaint::Shadow::NONE;
    // Popups and menus (the theme picker, combo lists) are floating surfaces
    // and lift the way the raylib ones do (`widgets::floating_surface`): a
    // short soft shadow falling below. Windows stay flat; they are panels.
    visuals.popup_shadow = egui::epaint::Shadow {
        offset: [0, 2],
        blur: 8,
        spread: 0,
        color: egui::Color32::from_black_alpha(48),
    };
    visuals.window_stroke = Stroke::new(1.0, edge);
    for widgets in [
        &mut visuals.widgets.noninteractive,
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        widgets.corner_radius = radius;
        widgets.bg_stroke = Stroke::new(1.0, edge);
        widgets.fg_stroke = Stroke::new(1.0, ink);
        widgets.bg_fill = of(p.ui_control);
        widgets.weak_bg_fill = of(p.ui_control);
        widgets.expansion = 0.0;
    }
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, rule);
    visuals.widgets.noninteractive.bg_fill = of(p.ui_surface);
    visuals.widgets.noninteractive.weak_bg_fill = of(p.ui_surface);
    visuals.widgets.hovered.bg_fill = of(p.ui_hover);
    visuals.widgets.hovered.weak_bg_fill = of(p.ui_hover);
    visuals.widgets.active.bg_fill = of(p.ui_selected);
    visuals.widgets.active.weak_bg_fill = of(p.ui_selected);
    visuals.widgets.active.bg_stroke = Stroke::new(1.0, of(p.ui_selected_edge));
    visuals.widgets.open.bg_fill = of(p.ui_selected);
    visuals.widgets.open.weak_bg_fill = of(p.ui_selected);
    style.visuals = visuals;
    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.button_padding = egui::vec2(10.0, 5.0);
    style.spacing.interact_size.y = 26.0;
    for (text_style, size, family) in [
        (
            egui::TextStyle::Heading,
            metric::UI_FONT_HEADER,
            egui::FontFamily::Proportional,
        ),
        (
            egui::TextStyle::Body,
            metric::UI_FONT_LABEL,
            egui::FontFamily::Proportional,
        ),
        (
            egui::TextStyle::Button,
            metric::UI_FONT_LABEL,
            egui::FontFamily::Proportional,
        ),
        (
            egui::TextStyle::Small,
            metric::UI_FONT_CAPTION,
            egui::FontFamily::Proportional,
        ),
        (
            egui::TextStyle::Monospace,
            metric::UI_FONT_CAPTION,
            egui::FontFamily::Monospace,
        ),
    ] {
        style
            .text_styles
            .insert(text_style, egui::FontId::new(size, family));
    }
    let selected = if dark {
        egui::Theme::Dark
    } else {
        egui::Theme::Light
    };
    ctx.set_theme(selected);
    ctx.set_style_of(selected, style);
}

/// Puts the interface face — Space Grotesk, the one the raylib chrome draws in
/// — at the front of egui's proportional family, once per context.
///
/// egui's bundled faces stay behind it as fallbacks, so a glyph Space Grotesk
/// lacks still draws. The monospace family is untouched: the terminal's grid
/// depends on Hack's exact one-cell advance.
fn install_interface_face(ctx: &Context) {
    let flag = egui::Id::new("musializer_interface_face");
    if ctx
        .data(|data| data.get_temp::<bool>(flag))
        .unwrap_or(false)
    {
        return;
    }
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "space-grotesk".to_owned(),
        std::sync::Arc::new(egui::FontData::from_static(
            musializer_runtime::font::INTERFACE_FACE,
        )),
    );
    fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default()
        .insert(0, "space-grotesk".to_owned());
    ctx.set_fonts(fonts);
    ctx.data_mut(|data| data.insert_temp(flag, true));
}

fn theme_picker(ui: &mut egui::Ui, id: &str, theme: &mut UiTheme) -> bool {
    let previous = *theme;
    egui::ComboBox::from_id_salt(id)
        .selected_text(theme.label())
        .show_ui(ui, |ui| {
            ui.selectable_value(theme, UiTheme::BlackAmber, UiTheme::BlackAmber.label());
            ui.selectable_value(theme, UiTheme::LightBlue, UiTheme::LightBlue.label());
        });
    *theme != previous
}

/// The same appearance control is available before any project is opened.
pub fn welcome_theme(
    ctx: &Context,
    window: (f32, f32),
    theme: &mut UiTheme,
) -> (Option<UiTheme>, bool) {
    apply_theme(ctx, *theme);
    let mut changed = false;
    let popup_was_open = egui::Popup::is_any_open(ctx);
    let area = egui::Area::new(egui::Id::new("welcome_appearance"))
        .fixed_pos(egui::pos2((window.0 - 232.0).max(0.0), 24.0))
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            ui.set_width(200.0);
            ui.horizontal(|ui| {
                ui.label("Theme");
                changed = theme_picker(ui, "welcome_theme", theme);
            });
        });
    let over_picker = ctx
        .input(|input| input.pointer.interact_pos())
        .is_some_and(|pos| area.response.rect.contains(pos));
    (
        changed.then_some(*theme),
        over_picker || popup_was_open || egui::Popup::is_any_open(ctx),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn welcome_picker_only_owns_its_header_not_the_open_audio_button() {
        let ctx = Context::default();
        let mut theme = UiTheme::LightBlue;
        for (point, owns) in [
            (egui::pos2(835.0, 40.0), true),
            (egui::pos2(184.0, 308.0), false),
        ] {
            // Match the real low-level backend; run_ui creates a background UI
            // and would conceal the whole-viewport pointer-ownership regression.
            for frame in 0..2 {
                ctx.begin_pass(egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(960.0, 640.0),
                    )),
                    events: vec![egui::Event::PointerMoved(point)],
                    ..Default::default()
                });
                let (change, blocked) = welcome_theme(&ctx, (960.0, 640.0), &mut theme);
                let _ = ctx.end_pass();
                assert!(change.is_none());
                // Areas measure themselves in an invisible first pass.
                if frame > 0 {
                    assert_eq!(blocked, owns, "pointer at {point:?}");
                }
            }
        }
    }

    #[test]
    fn both_skins_keep_body_selected_and_hovered_text_readable() {
        let packed =
            |color: egui::Color32| u32::from_be_bytes([color.r(), color.g(), color.b(), 255]);
        let ctx = Context::default();
        for theme in [UiTheme::LightBlue, UiTheme::BlackAmber] {
            apply_theme(&ctx, theme);
            let style = ctx.style_of(if theme == UiTheme::BlackAmber {
                egui::Theme::Dark
            } else {
                egui::Theme::Light
            });
            let v = &style.visuals;
            for (name, ink, fill) in [
                ("body", v.text_color(), v.panel_fill),
                ("selection", v.selection.stroke.color, v.selection.bg_fill),
                (
                    "button",
                    v.widgets.inactive.fg_stroke.color,
                    v.widgets.inactive.weak_bg_fill,
                ),
                (
                    "active button",
                    v.widgets.active.fg_stroke.color,
                    v.widgets.active.weak_bg_fill,
                ),
                (
                    "hovered",
                    v.widgets.hovered.fg_stroke.color,
                    v.widgets.hovered.bg_fill,
                ),
            ] {
                let ratio = musializer_core::ui::contrast::ratio(packed(ink), packed(fill));
                assert!(ratio >= 4.5, "{theme:?} {name}: {ratio}");
            }
        }
    }
}
