use super::controls::{accent_gradient, accent_gradient_hover};
use gpui_kit::{
    assets::IconName,
    base::Button,
    component::{
        ActiveTheme, Icon,
        dialog::{Dialog, DialogEntrance},
        input::{Input, InputState},
    },
    *,
};

/// Width of a source dialog with `sm:max-w-[400px]` over the default `max-w-md`.
pub(crate) fn compact_width(w: &Window) -> Pixels {
    px(if w.viewport_size().width >= px(640.) {
        400.
    } else {
        448.
    })
}

pub(crate) fn surface(host: Dialog, title: &'static str, width: Pixels, cx: &App) -> Dialog {
    host.accessibility_label(title)
        .centered()
        .w(width)
        .entrance(DialogEntrance::Fade)
        .p_0()
        .gap_0()
        .rounded(px(16.))
        .overflow_hidden()
        // Opaque: the legacy 95% surface relied on a backdrop blur GPUI cannot draw.
        .bg(cx.theme().group_box)
        .border_color(cx.theme().border.opacity(0.4))
        .backdrop_color(black().opacity(0.6))
        .shadow(surface_shadow())
        .close_button(false)
}

pub(crate) fn address_description(
    prefix: &str,
    address_text: &str,
    suffix: &str,
    w: &Window,
    cx: &App,
) -> StyledText {
    let mut style = w.text_style();
    style.color = cx.theme().muted_foreground;
    let mut address = style.to_run(address_text.len());
    address.font.weight = FontWeight::SEMIBOLD;
    address.color = cx.theme().foreground;
    StyledText::new(format!("{prefix}{address_text}{suffix}")).with_runs(vec![
        style.to_run(prefix.len()),
        address,
        style.to_run(suffix.len()),
    ])
}

pub(crate) fn header(
    id: &'static str,
    title: &'static str,
    description: impl IntoElement,
    cx: &App,
) -> impl IntoElement {
    let theme = cx.theme();
    div()
        .flex()
        .flex_col()
        .gap_1()
        .pr_6()
        .mb_3()
        .child(
            div()
                .id(id)
                .role(Role::Heading)
                .aria_label(title)
                .text_size(px(18.))
                .line_height(px(28.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(title),
        )
        .child(
            div()
                .text_size(px(12.))
                .line_height(px(16.))
                .text_color(theme.muted_foreground)
                .child(description),
        )
}

pub(crate) struct Field {
    pub id: &'static str,
    pub label_id: &'static str,
    pub label: &'static str,
}

impl Field {
    pub(crate) fn render(
        &self,
        input: &Entity<InputState>,
        readonly: bool,
        label_color: Hsla,
        w: &Window,
        cx: &App,
    ) -> impl IntoElement {
        let Self {
            id,
            label_id,
            label,
        } = *self;
        let target = input.clone();
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .id(label_id)
                    .self_start()
                    .text_size(px(12.))
                    .line_height(px(16.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(label_color)
                    .child(label)
                    .on_click(move |_, w, cx| target.focus_handle(cx).focus(w, cx)),
            )
            .child(text_input(input, id, label, readonly, px(32.), w, cx))
    }
}

pub(crate) fn text_input(
    input: &Entity<InputState>,
    id: &'static str,
    label: &'static str,
    readonly: bool,
    height: Pixels,
    w: &Window,
    cx: &App,
) -> Input {
    let theme = cx.theme();
    let mut shadow =
        vec![BoxShadow::new(px(0.), px(1.), black().opacity(0.05)).blur_radius(px(2.))];
    if input.focus_handle(cx).is_focused(w) {
        shadow.push(ring(theme.primary.opacity(0.4), 2.));
    }
    Styled::h(
        Input::new(input)
            .id(id)
            .aria_label(label)
            .readonly(readonly)
            .focus_bordered(false),
        height,
    )
    .w_full()
    .px_3()
    .py_1()
    .text_size(px(14.))
    .line_height(px(20.))
    .rounded(px(8.))
    .border_color(theme.border.opacity(0.5))
    .bg(theme.input)
    .shadow(shadow)
}

pub(crate) fn cancel_button(id: &'static str, disabled: bool, cx: &App) -> Button {
    let ghost = ghost_hover(cx);
    dialog_button(id, "Cancel", disabled, cx)
        .hover(move |style| style.bg(ghost))
        .child("Cancel")
}

fn ghost_hover(cx: &App) -> Hsla {
    if cx.theme().is_dark() {
        white().opacity(0.06)
    } else {
        black().opacity(0.04)
    }
}

pub(crate) fn primary_button(
    id: &'static str,
    label: &'static str,
    disabled: bool,
    cx: &App,
) -> Button {
    dialog_button(id, label, disabled, cx)
        .bg(accent_gradient())
        .text_color(white())
        .hover(|style| {
            style.bg(accent_gradient_hover()).shadow(vec![
                BoxShadow::new(px(0.), px(0.), rgba(0x7850dc4d).into()).blur_radius(px(20.)),
            ])
        })
}

pub(crate) fn close_button(id: &'static str, disabled: bool, cx: &App) -> Button {
    let theme = cx.theme();
    let foreground = theme.foreground;
    let close_hover = theme.muted.opacity(0.5);
    let focus_ring = theme.ring;
    Button::new(id)
        .accessibility_label("Close")
        .absolute()
        .top_3()
        .right_3()
        .flex()
        .items_center()
        .justify_center()
        .size(px(28.))
        .rounded(px(8.))
        .disabled(disabled)
        .text_color(theme.muted_foreground.opacity(0.6))
        .hover(move |style| style.bg(close_hover).text_color(foreground))
        .focus_visible(move |style| style.shadow(vec![ring(focus_ring, 1.)]))
        .child(Icon::new(IconName::Close).size(px(16.)))
}

fn dialog_button(
    id: &'static str,
    label: impl Into<SharedString>,
    disabled: bool,
    cx: &App,
) -> Button {
    let focus_ring = cx.theme().ring;
    Button::new(id)
        .accessibility_label(label)
        .h(px(36.))
        .px_3()
        .rounded(px(8.))
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(12.))
        .line_height(px(16.))
        .font_weight(FontWeight::MEDIUM)
        .focus_visible(move |s| s.shadow(vec![ring(focus_ring, 2.)]))
        .disabled(disabled)
        .styles(|styles| styles.disabled(|s| s.opacity(0.5)))
}

pub(crate) fn ring(color: Hsla, width: f32) -> BoxShadow {
    BoxShadow::new(px(0.), px(0.), color).spread_radius(px(width))
}

fn surface_shadow() -> Vec<BoxShadow> {
    vec![
        ring(white().opacity(0.05), 1.),
        BoxShadow::new(px(0.), px(25.), black().opacity(0.5))
            .blur_radius(px(50.))
            .spread_radius(px(-12.)),
    ]
}
