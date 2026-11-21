use super::*;
use crate::workspace::theme::dialog;
use gpui_kit::{
    assets::IconName,
    component::{
        ActiveTheme, Icon,
        dialog::{Cancel, Confirm},
        input::{InputEvent, InputState},
    },
};

pub(super) const TITLE: &str = "Save Query";
const PREVIEW_UNITS: usize = 300;

pub(super) struct SaveDialog {
    input: Entity<InputState>,
    preview: SharedString,
    _change: Subscription,
}

impl SaveDialog {
    pub(super) fn new(input: Entity<InputState>, sql: &str, cx: &mut Context<Self>) -> Self {
        let change = cx.subscribe(&input, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        });
        Self {
            input,
            preview: preview(sql),
            _change: change,
        }
    }
}

/// Mirrors the source `slice(0, 300)` in UTF-16 units without splitting a character.
fn preview(sql: &str) -> SharedString {
    let mut units = 0;
    let cut = sql.char_indices().find_map(|(at, c)| {
        units += c.len_utf16();
        (units > PREVIEW_UNITS).then_some(at)
    });
    match cut {
        Some(at) => format!("{}...", &sql[..at]).into(),
        None => sql.to_owned().into(),
    }
}

impl Render for SaveDialog {
    fn render(&mut self, w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let blank = self.input.read(cx).value().trim().is_empty();
        let target = self.input.clone();
        let field = div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .id("save-query-title-label")
                    .self_start()
                    .text_size(px(12.))
                    .line_height(px(16.))
                    .text_color(theme.muted_foreground)
                    .child("Query Name")
                    .on_click(move |_, w, cx| target.focus_handle(cx).focus(w, cx)),
            )
            .child(dialog::text_input(
                &self.input,
                "saved-query-title",
                "Query Name",
                false,
                px(36.),
                w,
                cx,
            ));
        let preview = div()
            .id("save-query-preview")
            .max_h(px(96.))
            .overflow_y_scroll()
            .rounded(px(8.))
            .bg(theme.muted.opacity(0.5))
            .p_2()
            .child(
                div()
                    .text_size(px(11.))
                    .line_height(px(16.5))
                    .text_color(theme.muted_foreground)
                    .child(self.preview.clone()),
            );
        let cancel =
            |_: &ClickEvent, w: &mut Window, cx: &mut App| w.dispatch_action(Box::new(Cancel), cx);
        let footer = div()
            .flex()
            .justify_end()
            .gap_2()
            .child(dialog::cancel_button("save-query-cancel", false, cx).on_click(cancel))
            .child(
                dialog::primary_button("save-query-submit", TITLE, blank, cx)
                    .child(
                        Icon::new(IconName::Save)
                            .size(px(14.))
                            .mr_1()
                            .flex_shrink_0(),
                    )
                    .child(TITLE)
                    .on_click(|_, w, cx| {
                        w.dispatch_action(Box::new(Confirm { secondary: false }), cx)
                    }),
            );
        clipboard_order::capture(
            div()
                .id("save-query-content")
                .relative()
                .w_full()
                .min_h_0()
                .overflow_y_scroll()
                .text_color(theme.foreground),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .p_6()
                .child(dialog::header(
                    "save-query-heading",
                    TITLE,
                    "Save the current query for quick access later.",
                    cx,
                ))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_4()
                        .child(field)
                        .child(preview)
                        .child(footer),
                )
                .child(dialog::close_button("save-query-close", false, cx).on_click(cancel)),
        )
    }
}
