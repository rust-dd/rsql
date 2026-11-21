use gpui_kit::{
    component::{Sizable, button::Button, theme::oklch},
    *,
};

// GPUI has no letter-spacing, so each glyph is laid out with the legacy CSS tracking as a gap.
pub(crate) fn tracked(text: impl Into<SharedString>, size: f32, em: f32) -> Div {
    let text = text.into();
    div()
        .flex()
        .flex_row()
        .gap(px(size * em))
        .children(text.chars().map(|c| SharedString::from(c.to_string())))
}

pub(crate) fn accent_gradient() -> Background {
    linear_gradient(
        135.,
        linear_color_stop(oklch(0.6, 0.22, 255.), 0.),
        linear_color_stop(oklch(0.55, 0.22, 290.), 1.),
    )
}

/// The gradient variant's `hover:brightness-110`, which scales each sRGB channel.
pub(crate) fn accent_gradient_hover() -> Background {
    let brighten = |color: Hsla| {
        let rgb: Rgba = color.into();
        Rgba {
            r: (rgb.r * 1.1).min(1.),
            g: (rgb.g * 1.1).min(1.),
            b: (rgb.b * 1.1).min(1.),
            ..rgb
        }
    };
    linear_gradient(
        135.,
        linear_color_stop(brighten(oklch(0.6, 0.22, 255.)), 0.),
        linear_color_stop(brighten(oklch(0.55, 0.22, 290.)), 1.),
    )
}

pub(crate) fn compact_button(button: Button) -> Button {
    button
        .small()
        .text_size(px(12.))
        .h(px(28.))
        .px_2()
        .rounded(px(8.))
}

pub(crate) fn context_menu(
    menu: gpui_kit::component::menu::PopupMenu,
    window: &Window,
) -> gpui_kit::component::menu::PopupMenu {
    menu.context_menu_appearance()
        .scrollable(true)
        .max_h((window.viewport_size().height - px(16.)).max(px(30.)))
}

pub(crate) fn role_superuser_color() -> Hsla {
    oklch(0.769, 0.188, 70.08)
}

pub(crate) fn setting_context_colors(context: &str, cx: &App) -> (Hsla, Hsla) {
    use gpui_kit::component::ActiveTheme;
    let dark = cx.theme().is_dark();
    let (foreground, background) = match (context, dark) {
        ("user", false) => (oklch(0.527, 0.154, 150.069), oklch(0.962, 0.044, 156.743)),
        ("user", true) => (oklch(0.792, 0.209, 151.711), oklch(0.393, 0.095, 152.535)),
        ("superuser", false) => (oklch(0.488, 0.243, 264.376), oklch(0.932, 0.032, 255.585)),
        ("superuser", true) => (oklch(0.707, 0.165, 254.624), oklch(0.379, 0.146, 265.522)),
        ("postmaster", false) => (oklch(0.505, 0.213, 27.518), oklch(0.936, 0.032, 17.717)),
        ("postmaster", true) => (oklch(0.704, 0.191, 22.216), oklch(0.396, 0.141, 25.723)),
        ("sighup", false) => (oklch(0.554, 0.135, 66.442), oklch(0.973, 0.071, 103.193)),
        ("sighup", true) => (oklch(0.852, 0.199, 91.936), oklch(0.421, 0.095, 57.708)),
        _ => return (cx.theme().muted_foreground, cx.theme().muted),
    };
    (
        foreground,
        if dark {
            background.opacity(0.3)
        } else {
            background
        },
    )
}

pub(crate) fn extension_check_color() -> Hsla {
    oklch(0.723, 0.219, 149.579)
}

pub(crate) fn extension_action_colors(
    action: rsql_core::services::extension_action::ExtensionAction,
    cx: &App,
) -> (Hsla, Hsla, Hsla) {
    use gpui_kit::component::ActiveTheme;
    use rsql_core::services::extension_action::ExtensionAction;
    let theme = cx.theme();
    match action {
        ExtensionAction::Install => (theme.foreground, theme.accent_foreground, theme.accent),
        ExtensionAction::Drop => (
            theme.muted_foreground,
            theme.danger,
            if theme.is_dark() {
                white().opacity(0.06)
            } else {
                black().opacity(0.04)
            },
        ),
        ExtensionAction::Update if theme.is_dark() => (
            oklch(0.852, 0.199, 91.936),
            theme.accent_foreground,
            oklch(0.421, 0.095, 57.708).opacity(0.3),
        ),
        ExtensionAction::Update => (
            oklch(0.554, 0.135, 66.442),
            theme.accent_foreground,
            oklch(0.973, 0.071, 103.193),
        ),
    }
}
