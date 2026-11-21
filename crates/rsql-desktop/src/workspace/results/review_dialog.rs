use super::review_target::{ReviewStatus, ReviewTarget};
use super::*;
use crate::workspace::theme::dialog;
use gpui_kit::base::Button;
use gpui_kit::component::{
    ActiveTheme,
    dialog::{Cancel, Confirm},
};
use gpui_kit::prelude::FluentBuilder;

pub(super) const TITLE: &str = "Apply changes";
/// The shared source dialog keeps its default `max-w-md`.
pub(super) const WIDTH: f32 = 448.;

pub(super) fn summary(updates: usize, deletes: usize) -> String {
    let plural = |n: usize| if n == 1 { "" } else { "s" };
    let mut parts = Vec::new();
    if updates > 0 {
        parts.push(format!("{updates} update{}", plural(updates)));
    }
    if deletes > 0 {
        parts.push(format!("{deletes} deletion{}", plural(deletes)));
    }
    parts.join(" and ")
}

pub(super) fn content(
    (updates, deletes): (usize, usize),
    status: ReviewStatus,
    retry: Rc<ReviewTarget>,
    cx: &App,
) -> impl IntoElement {
    let theme = cx.theme();
    let summary = summary(updates, deletes);
    let message = match status {
        ReviewStatus::Checking => Some((
            "review-target-checking",
            "Checking the editor target…",
            Role::Status,
            theme.muted_foreground,
        )),
        ReviewStatus::Mismatch => Some((
            "review-target-mismatch",
            "Select the original table statement to apply these changes.",
            Role::Alert,
            theme.danger,
        )),
        ReviewStatus::Unavailable => Some((
            "review-target-unavailable",
            "Could not verify the table. Retry the check before applying.",
            Role::Alert,
            theme.danger,
        )),
        ReviewStatus::Matches => None,
    };
    let cancel =
        |_: &ClickEvent, w: &mut Window, cx: &mut App| w.dispatch_action(Box::new(Cancel), cx);
    let fill = if deletes > 0 {
        theme.danger
    } else {
        theme.success
    };
    let footer = div()
        .flex()
        .justify_end()
        .gap_2()
        .mt(px(if message.is_some() { 16. } else { 4. }))
        .child(quiet_button("review-cancel", "Cancel", cx).on_click(cancel))
        .when(status == ReviewStatus::Unavailable, |footer| {
            footer.child(
                quiet_button("Retry verification", "Retry verification", cx).on_click(
                    move |_, w, _| {
                        retry.retry();
                        w.refresh();
                    },
                ),
            )
        })
        .child(
            source_button("Apply", format!("Apply {summary}"), cx)
                .disabled(status != ReviewStatus::Matches)
                .bg(fill)
                .text_color(theme.danger_foreground)
                .hover(move |s| s.bg(fill.opacity(0.9)))
                .on_click(|_, w, cx| w.dispatch_action(Box::new(Confirm { secondary: false }), cx)),
        );
    div()
        .flex()
        .flex_col()
        .p_6()
        .text_color(theme.foreground)
        .child(dialog::header(
            "review-heading",
            TITLE,
            format!(
                "This will apply {summary} in a single transaction. Deletions cannot be undone. \
                 If any statement does not match exactly one row, nothing is changed."
            ),
            cx,
        ))
        .children(message.map(|(id, message, role, color)| {
            div()
                .id(id)
                .test_support()
                .role(role)
                .aria_label(message)
                .text_size(px(12.))
                .line_height(px(16.))
                .text_color(color)
                .child(message)
        }))
        .child(footer)
        .child(dialog::close_button("review-close", false, cx).on_click(cancel))
}

fn quiet_button(id: &'static str, label: &'static str, cx: &App) -> Button {
    let theme = cx.theme();
    let (foreground, accent) = (theme.foreground, theme.accent);
    source_button(id, label, cx)
        .text_color(theme.muted_foreground)
        .hover(move |s| s.bg(accent).text_color(foreground))
}

fn source_button(id: &'static str, label: impl Into<SharedString>, cx: &App) -> Button {
    let label = label.into();
    let theme = cx.theme();
    let focus = theme.ring;
    Button::new(id)
        .accessibility_label(label.clone())
        .flex()
        .items_center()
        .justify_center()
        .px_3()
        .py(px(6.))
        .rounded(px(8.))
        .text_size(px(12.))
        .line_height(px(16.))
        .focus_visible(move |s| s.shadow(vec![dialog::ring(focus, 2.)]))
        .styles(|styles| styles.disabled(|s| s.opacity(0.5)))
        .child(label)
}
