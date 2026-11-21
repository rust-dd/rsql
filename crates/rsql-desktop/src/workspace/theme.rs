pub(crate) mod catalog;
pub(crate) mod controls;
pub(crate) mod dialog;
pub(super) mod editor;
pub(crate) mod grid;
pub(super) use editor::SuggestionColors;

use gpui_kit::{
    component::{Theme, ThemeMode, theme::oklch},
    *,
};

struct SystemMono(Option<SharedString>);
impl Global for SystemMono {}

// The legacy WebView resolved `ui-monospace` to SF Mono, which CoreText does not enumerate.
fn system_mono(cx: &mut App) -> Option<SharedString> {
    if let Some(mono) = cx.try_global::<SystemMono>() {
        return mono.0.clone();
    }
    let family = cfg!(target_os = "macos")
        .then(|| {
            // Terminal ships static weights; the variable system file exposes only Regular to GPUI.
            let [dir, variable] = [
                "/System/Applications/Utilities/Terminal.app/Contents/Resources/Fonts",
                "/System/Library/Fonts",
            ];
            let load = |dir: &str, family: &'static str, keep: fn(&str) -> bool| {
                let fonts = std::fs::read_dir(dir)
                    .ok()?
                    .flatten()
                    .filter(|e| keep(&e.file_name().to_string_lossy()))
                    .filter_map(|e| std::fs::read(e.path()).ok())
                    .map(std::borrow::Cow::Owned)
                    .collect::<Vec<_>>();
                (!fonts.is_empty() && cx.text_system().add_fonts(fonts).is_ok())
                    .then(|| SharedString::from(family))
            };
            load(dir, "SF Mono", |n| {
                n.starts_with("SF-Mono-") && n.ends_with(".otf")
            })
            .or_else(|| load(variable, ".SF NS Mono", |n| n.starts_with("SFNSMono")))
        })
        .flatten();
    cx.set_global(SystemMono(family.clone()));
    family
}

pub(crate) fn apply(mode: ThemeMode, window: Option<&mut Window>, cx: &mut App) {
    Theme::change(mode, window, cx);
    let mono = system_mono(cx);
    Theme::update(cx, |theme| {
        if let Some(mono) = mono {
            theme.mono_font_family = mono;
        }
        let dark = mode.is_dark();
        let color = |light, night| {
            let (l, c, h) = if dark { night } else { light };
            oklch(l, c, h)
        };
        theme.background = color((0.97, 0.005, 260.), (0.13, 0.015, 260.));
        theme.foreground = color((0.15, 0.01, 260.), (0.93, 0.01, 260.));
        theme.group_box = color((1., 0., 0.), (0.16, 0.015, 260.));
        theme.group_box_foreground = theme.foreground;
        theme.popover = color((1., 0., 0.), (0.18, 0.015, 260.));
        theme.popover_foreground = theme.foreground;
        theme.primary = color((0.5, 0.2, 265.), (0.65, 0.2, 265.));
        theme.primary_foreground = oklch(0.98, 0., 0.);
        theme.secondary = color((0.94, 0.005, 260.), (0.2, 0.02, 260.));
        theme.secondary_foreground = theme.foreground;
        theme.muted = color((0.93, 0.005, 260.), (0.22, 0.015, 260.));
        theme.muted_foreground = color((0.45, 0.01, 260.), (0.55, 0.01, 260.));
        theme.accent = color((0.95, 0.005, 260.), (0.22, 0.02, 265.));
        theme.accent_foreground = theme.foreground;
        theme.danger = oklch(0.55, 0.22, 25.);
        theme.danger_foreground = color((0.98, 0., 0.), (0.93, 0.01, 260.));
        theme.border = color((0.88, 0.01, 260.), (0.24, 0.015, 260.));
        theme.input = color((0.96, 0.005, 260.), (0.19, 0.015, 260.));
        theme.ring = color((0.5, 0.2, 280.), (0.65, 0.2, 280.));
        theme.sidebar = color((0.96, 0.005, 260.), (0.11, 0.015, 260.));
        theme.sidebar_foreground = color((0.2, 0.01, 260.), (0.85, 0.01, 260.));
        theme.sidebar_primary = theme.primary;
        theme.sidebar_primary_foreground = theme.primary_foreground;
        theme.sidebar_accent = color((0.92, 0.005, 260.), (0.18, 0.02, 260.));
        theme.sidebar_accent_foreground = theme.foreground;
        theme.sidebar_border = color((0.88, 0.01, 260.), (0.2, 0.015, 260.));
        theme.success = color((0.52, 0.18, 150.), (0.65, 0.18, 150.));
        theme.warning = color((0.6, 0.18, 85.), (0.75, 0.18, 85.));
        theme.button = theme.group_box;
        theme.button_foreground = theme.foreground;
        theme.button_hover = theme.accent;
        theme.button_active = theme.secondary;
        theme.button_primary = theme.primary;
        theme.button_primary_foreground = theme.primary_foreground;
        theme.button_secondary = theme.secondary;
        theme.button_secondary_foreground = theme.secondary_foreground;
        theme.button_danger = theme.danger;
        theme.button_danger_foreground = theme.danger_foreground;
        theme.overlay = black().opacity(0.6);
        theme.radius = px(8.);
        theme.mono_font_size = px(13.);
        theme.notification.placement = Anchor::BottomRight;
        theme.notification.width = px(356.);
        theme.notification.margins.right = px(24.);
        theme.notification.margins.bottom = px(24.);
        theme.notification.max_items = 3;
        editor::apply(theme, dark);
        grid::apply(theme, dark);
    });
    editor::scrollbar(mode, cx);
}
