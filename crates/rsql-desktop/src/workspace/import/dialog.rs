use super::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    assets::IconName,
    base::Button,
    component::{
        ActiveTheme, Icon, Sizable, dialog::DialogEntrance, label::Label, spinner::Spinner,
    },
};

const TITLE: &str = "Import CSV";

pub(super) fn frame_width(w: &Window) -> Pixels {
    px(if w.viewport_size().width >= px(640.) {
        700.
    } else {
        448.
    })
}

impl Dialog {
    pub(super) fn present(dialog: Entity<Self>, w: &mut Window, cx: &mut App) {
        let focus = if dialog.read(cx).can_choose() {
            dialog.read(cx).picker_focus.clone()
        } else {
            dialog.read(cx).initial_focus.clone()
        };
        w.open_dialog(cx, move |frame, w, cx| {
            let owner = dialog.clone();
            frame
                .accessibility_label(TITLE)
                .centered()
                .w(frame_width(w))
                .entrance(DialogEntrance::Fade)
                .p_0()
                .gap_0()
                .rounded(px(16.))
                .overflow_hidden()
                .bg(cx.theme().group_box.opacity(0.95))
                .border_color(cx.theme().border.opacity(0.4))
                .backdrop_color(black().opacity(0.6))
                .shadow(vec![
                    ring(white().opacity(0.05), 1.),
                    BoxShadow::new(px(0.), px(25.), black().opacity(0.5))
                        .blur_radius(px(50.))
                        .spread_radius(px(-12.)),
                ])
                .close_button(false)
                .on_cancel(move |_, _, cx| owner.read(cx).phase != Phase::Importing)
                .on_ok(|_, _, _| false)
                .content({
                    let dialog = dialog.clone();
                    move |body, _, _| body.min_h_0().child(dialog.clone())
                })
        });
        focus.focus(w, cx);
    }

    fn close(&self, w: &mut Window, cx: &mut Context<Self>) {
        if self.phase != Phase::Importing {
            w.close_dialog(cx);
        }
    }

    fn header(&self, cx: &Context<Self>) -> Div {
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let foreground = theme.foreground;
        let hover = theme.muted.opacity(0.5);
        let focus = theme.ring;
        div()
            .relative()
            .px_5()
            .pt_5()
            .pb_6()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .pr_6()
                    .gap_1()
                    .child(
                        div()
                            .id("csv-heading")
                            .role(Role::Heading)
                            .aria_label(TITLE)
                            .flex()
                            .items_center()
                            .gap_2()
                            .text_size(px(18.))
                            .line_height(px(28.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(
                                Icon::new(IconName::FileUp)
                                    .size(px(16.))
                                    .text_color(theme.primary),
                            )
                            .child(TITLE),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(
                                Label::new("Import data into")
                                    .line_height(px(16.))
                                    .text_color(muted),
                            )
                            .child(
                                Label::new(format!("{}.{}", self.schema, self.name))
                                    .truncate()
                                    .line_height(px(16.)),
                            ),
                    ),
            )
            .child(
                Button::new("csv-close")
                    .accessibility_label("Close")
                    .disabled(self.phase == Phase::Importing)
                    .absolute()
                    .right_3()
                    .top_3()
                    .size(px(28.))
                    .rounded(px(8.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(muted.opacity(0.6))
                    .hover(move |s| s.bg(hover).text_color(foreground))
                    .focus_visible(move |s| s.shadow(vec![ring(focus, 1.)]))
                    .child(Icon::new(IconName::X).size(px(16.)))
                    .on_click(cx.listener(|d, _, w, cx| d.close(w, cx))),
            )
    }

    fn picker_button(&self, cx: &Context<Self>) -> Button {
        let name: SharedString = self
            .path
            .as_deref()
            .and_then(Path::file_name)
            .map(|name| name.to_string_lossy().into_owned().into())
            .unwrap_or_else(|| "Choose CSV file...".into());
        let hover = cx.theme().accent;
        let fg = cx.theme().accent_foreground;
        button("Choose file", name.clone(), !self.can_choose(), cx)
            .track_focus(&self.picker_focus)
            .w_full()
            .border_1()
            .border_color(cx.theme().border)
            .hover(move |s| s.bg(hover).text_color(fg))
            .child(Icon::new(IconName::FileUp).size(px(14.)))
            .child(div().min_w_0().truncate().child(name))
            .on_click(cx.listener(|d, _, w, cx| d.choose(w, cx)))
    }

    fn status(&self, cx: &App) -> Option<impl IntoElement> {
        let t = cx.theme();
        let (message, color, icon, role) = if !self.current()
            && !matches!(self.phase, Phase::Importing | Phase::Finished { .. })
        {
            (
                "Connection closed.",
                t.danger,
                Some(IconName::CircleAlert),
                Role::Alert,
            )
        } else if let Some(problem) = self
            .preview
            .as_ref()
            .filter(|_| self.phase == Phase::Ready)
            .and_then(|preview| preview.problem())
        {
            // Show why Import is disabled now rather than an older retryable write failure.
            (problem, t.danger, Some(IconName::CircleAlert), Role::Alert)
        } else if !self.message.is_empty() {
            match self.phase {
                Phase::Loading | Phase::Reading | Phase::Importing => {
                    (&*self.message, t.muted_foreground, None, Role::Status)
                }
                Phase::Finished { committed: true } => (
                    &*self.message,
                    t.success,
                    Some(IconName::Check),
                    Role::Status,
                ),
                _ => (
                    &*self.message,
                    t.danger,
                    Some(IconName::CircleAlert),
                    Role::Alert,
                ),
            }
        } else {
            return None;
        };
        Some(
            div()
                .id("import-status")
                .role(role)
                .aria_label(message.to_owned())
                .flex()
                .items_center()
                .gap_2()
                .rounded(px(8.))
                .px_3()
                .py_2()
                .bg(color.opacity(0.1))
                .text_color(color)
                .children(icon.map(|icon| Icon::new(icon).size(px(14.)).flex_shrink_0()))
                .child(div().min_w_0().child(message.to_owned())),
        )
    }

    fn actions(&self, cx: &Context<Self>) -> Div {
        let busy = self.phase == Phase::Importing;
        let finished = matches!(self.phase, Phase::Finished { .. });
        let close = if finished { "Done" } else { "Cancel" };
        let hover = if cx.theme().is_dark() {
            white().opacity(0.06)
        } else {
            black().opacity(0.04)
        };
        div()
            .flex()
            .flex_shrink_0()
            .justify_end()
            .gap_2()
            .px_5()
            .pt_6()
            .pb_5()
            .child(
                button(close, close, busy, cx)
                    .hover(move |s| s.bg(hover))
                    .child(close)
                    .on_click(cx.listener(|d, _, w, cx| d.close(w, cx))),
            )
            .when(!finished, |actions| {
                actions.child(
                    button("Import", "Import", !self.can_import(), cx)
                        .bg(theme::controls::accent_gradient())
                        .text_color(white())
                        .hover(|s| {
                            s.bg(theme::controls::accent_gradient_hover()).shadow(vec![
                                BoxShadow::new(px(0.), px(0.), rgba(0x7850dc4d).into())
                                    .blur_radius(px(20.)),
                            ])
                        })
                        .when(busy, |button| {
                            button.child(Spinner::new().with_size(px(14.)).color(white()))
                        })
                        .child("Import")
                        .on_click(cx.listener(|d, _, _, cx| d.accept(cx))),
                )
            })
    }
}

fn ring(color: Hsla, width: f32) -> BoxShadow {
    BoxShadow::new(px(0.), px(0.), color).spread_radius(px(width))
}

fn button(id: &'static str, label: impl Into<SharedString>, disabled: bool, cx: &App) -> Button {
    let focus = cx.theme().ring;
    Button::new(id)
        .accessibility_label(label)
        .disabled(disabled)
        .flex()
        .items_center()
        .justify_center()
        .gap_2()
        .flex_shrink_0()
        .px_3()
        .h(px(36.))
        .rounded(px(8.))
        .font_weight(FontWeight::MEDIUM)
        .focus_visible(move |s| s.shadow(vec![ring(focus, 2.)]))
        .styles(|s| s.disabled(|s| s.opacity(0.5)))
}

impl Render for Dialog {
    fn render(&mut self, w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut body = div()
            .flex()
            .flex_col()
            .gap_4()
            .px_5()
            .child(self.picker_button(cx));
        if let Some(preview) = &self.preview {
            body = body
                .child(self.preview_table(preview, w, cx))
                .child(self.mapping_list(preview, cx));
        }
        body = body.children(self.status(cx));
        div()
            .id("csv-import-dialog")
            .track_focus(&self.initial_focus)
            .flex()
            .flex_col()
            .min_h_0()
            .max_h((w.viewport_size().height - px(36.)).max(px(0.)))
            .text_size(px(12.))
            .line_height(px(16.))
            .text_color(cx.theme().foreground)
            .child(
                div()
                    .id("csv-import-scroll")
                    .track_scroll(&self.body_scroll)
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(self.header(cx))
                    .child(body),
            )
            .child(self.actions(cx))
    }
}
