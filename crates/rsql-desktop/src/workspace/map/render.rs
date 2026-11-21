use super::*;
use crate::workspace::theme::controls::compact_button;
use gpui_kit::component::{ActiveTheme, button::Button, label::Label, link::Link};

impl ResultMap {
    pub(super) fn control(
        &self,
        id: &'static str,
        label: &'static str,
        cx: &Context<Self>,
        action: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> Button {
        compact_button(Button::new(id).label(label).tooltip(id))
            .outline()
            .px_1()
            .h(px(26.))
            .on_click(cx.listener(move |this, _, w, cx| {
                action(this, w, cx);
                cx.notify();
            }))
    }

    pub(in crate::workspace) fn first_page_note(&self, cx: &App) -> Option<Div> {
        self.first_page.clone().map(|note| {
            div()
                .border_b_1()
                .border_color(cx.theme().border)
                .px_4()
                .py_2()
                .child(
                    Label::new(note)
                        .text_xs()
                        .text_color(cx.theme().muted_foreground),
                )
        })
    }

    fn status(&self) -> String {
        if self.read.is_some() {
            return "Waiting for the first result page…".into();
        }
        let Some(shapes) = &self.shapes else {
            return if self.page.strong_count() == 0 {
                "First result page unavailable".into()
            } else {
                "Reading geometry…".into()
            };
        };
        let mut status = match (
            shapes.geometries.is_empty(),
            self.rows.as_ref(),
            self.detected.flatten(),
        ) {
            (false, Some(rows), Some(column)) => format!(
                "{} geometries from \"{}\"",
                shapes.geometries.len().saturating_sub(self.unpainted.len()),
                super::super::results::Preview::new(&rows.columns[column]).text
            ),
            _ => "No geometry data detected in results".into(),
        };
        for (n, reason) in shapes.omitted.iter().zip([
            "unsupported",
            "malformed",
            "input byte limit",
            "coordinate limit",
            "geometry memory limit",
            "parser memory limit",
        ]) {
            if *n > 0 {
                status.push_str(&format!(" · {n} omitted ({reason})"));
            }
        }
        if !self.unpainted.is_empty() {
            status.push_str(&format!(
                " · {} omitted (native drawing limit)",
                self.unpainted.len()
            ));
        }
        status
    }
}

impl Render for ResultMap {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let owner = cx.entity();
        let painter = owner.clone();
        let surface = canvas(
            move |bounds, w, cx| owner.update(cx, |v, cx| v.layout(bounds, w, cx)),
            move |_, (), w, cx| {
                painter.update(cx, |v, cx| v.paint(w, cx));
                w.on_mouse_event(move |event: &MouseMoveEvent, phase, w, cx| {
                    if phase == DispatchPhase::Capture && painter.read(cx).drag.is_some() {
                        painter.update(cx, |v, cx| v.motion(event, w, cx));
                    }
                });
            },
        )
        .absolute()
        .inset_0();
        let has_geometry = self
            .shapes
            .as_ref()
            .is_some_and(|s| !s.geometries.is_empty());
        let status = self.status();
        let backdrop = div()
            .id("map-surface")
            .test_support()
            .absolute()
            .inset_0()
            .overflow_hidden()
            .on_mouse_down(MouseButton::Left, cx.listener(Self::press))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::release))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::release))
            .on_scroll(cx.listener(Self::scroll))
            .child(surface);
        let mut map = div()
            .relative()
            .flex_1()
            .min_h_0()
            .w_full()
            .overflow_hidden()
            .bg(cx.theme().muted)
            .child(backdrop);
        if has_geometry {
            let controls = div()
                .absolute()
                .top_2()
                .left_2()
                .flex()
                .flex_col()
                .items_start()
                .child(
                    self.control("Zoom in", "+", cx, |v, _, _| {
                        v.zoom(1., v.viewport().map(|n| n / 2.))
                    })
                    .w(px(26.)),
                )
                .child(
                    self.control("Zoom out", "−", cx, |v, _, _| {
                        v.zoom(-1., v.viewport().map(|n| n / 2.))
                    })
                    .w(px(26.)),
                )
                .child(self.control("Fit map", "Fit", cx, |v, _, _| v.fit()));
            map = map.child(controls).children(self.popup(cx));
        }
        let badge = div()
            .id("map-status")
            .test_support()
            .absolute()
            .bottom(px(12.))
            .left(px(12.))
            .max_w(relative(0.9))
            .bg(cx.theme().group_box.opacity(0.9))
            .border_1()
            .border_color(cx.theme().border)
            .rounded_sm()
            .px(px(8.))
            .py(px(4.))
            .text_size(px(12.))
            .text_color(cx.theme().muted_foreground)
            .child(Label::new(status).text_color(cx.theme().muted_foreground));
        let (attribution, url) = self.session.tiles.provider().attribution();
        let mut footer = div()
            .w_full()
            .flex()
            .flex_wrap()
            .justify_between()
            .items_center()
            .bg(cx.theme().group_box)
            .text_xs()
            .px_1()
            .child(
                Link::new("map-attribution").href(url).child(
                    Label::new(attribution)
                        .id("map-attribution-label")
                        .underline(),
                ),
            );
        if let Some(message) = self.upload_error.as_ref().or(self.tiles.message.as_ref()) {
            footer = footer.child(Label::new(message.clone()));
        }
        if self.tiles.can_retry || self.upload_error.is_some() {
            footer = footer
                .child(self.control("Retry tiles", "Retry", cx, |v, w, cx| v.tiles.retry(w, cx)));
        }
        let map = if has_geometry {
            map.child(badge)
        } else {
            map.flex().items_center().justify_center().child(
                Label::new(self.status())
                    .text_sm()
                    .text_color(cx.theme().muted_foreground),
            )
        };
        div()
            .id("result-map")
            .test_support()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .children(self.first_page_note(cx))
            .child(map)
            .child(footer)
    }
}
