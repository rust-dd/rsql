use super::*;
use gpui_kit::{
    assets::IconName,
    component::{ActiveTheme, Icon, Sizable, label::Label, spinner::Spinner, tooltip::Tooltip},
    prelude::FluentBuilder,
};
use rsql_desktop::erd::LoadStatus;

impl Render for Panel {
    fn render(&mut self, w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let owner = self.model.owner.borrow();
        let root = div()
            .id("erd-panel")
            .test_support()
            .relative()
            .flex_1()
            .min_w_0()
            .overflow_hidden()
            .track_focus(&self.focus)
            .bg(cx.theme().background)
            .text_color(cx.theme().muted_foreground)
            .text_size(px(14.));
        let Some(owner) = owner.as_ref() else {
            grid::Tile::release(&mut self.grid, w);
            return root.child(Label::new("Schema diagram unavailable"));
        };
        let diagram = owner.diagram();
        let ready = diagram.filter(|d| !d.graph().tables().is_empty());
        let mut root = match ready {
            Some(d) => {
                let grid = grid::Tile::prepare(&mut self.grid, self.camera, w, cx).layer(
                    self.camera,
                    self.viewport.size,
                    camera::extent(d),
                );
                let mut surface = div()
                    .id("erd-surface")
                    .test_support()
                    .absolute()
                    .inset_0()
                    .font_family(cx.theme().mono_font_family.clone())
                    .cursor(CursorStyle::OpenHand)
                    .when(self.gesture.is_some(), |s| {
                        s.cursor(CursorStyle::ClosedHand)
                    })
                    .child(grid)
                    .child(self.drawing(cx))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|p, e: &MouseDownEvent, w, cx| {
                            p.press(e.position, w, cx);
                        }),
                    )
                    .on_mouse_exit(cx.listener(|p, _, _, cx| {
                        if p.hover.take().is_some() {
                            cx.notify();
                        }
                    }))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|p, _, _, cx| {
                            p.gesture = None;
                            cx.notify();
                        }),
                    )
                    .on_mouse_up_out(
                        MouseButton::Left,
                        cx.listener(|p, _, _, cx| {
                            if p.gesture.take().is_some() {
                                cx.notify();
                            }
                        }),
                    )
                    .on_scroll(cx.listener(|p, e: &ScrollEvent, _, cx| {
                        let y = e.delta.pixel_delta(px(1.)).y;
                        if y != px(0.) {
                            p.zoom(if y > px(0.) { 1.1 } else { 0.9 }, Some(e.position), cx);
                            cx.stop_propagation();
                        }
                    }));
                if let Some(connections) = self.hover.and_then(|id| d.graph().connections(id)) {
                    for e in d
                        .graph()
                        .edges()
                        .iter()
                        .filter(|e| connections.edges.contains(&e.id))
                    {
                        if let Some(g) = d.geometry(e.id) {
                            surface = surface.children(self.edge_label(d, e, g, w, cx));
                        }
                    }
                }
                let clip = Bounds::new(Point::default(), self.viewport.size);
                let connections = self.hover.and_then(|id| d.graph().connections(id));
                for b in d.boxes().filter(|b| {
                    self.camera
                        .bounds(*b)
                        .dilate(self.camera.length(6.))
                        .intersects(&clip)
                }) {
                    if let Some(t) = d.graph().table(b.table) {
                        surface = surface.child(cards::card(
                            self,
                            b,
                            t,
                            connections.is_some_and(|c| c.neighbors.contains(&b.table)),
                            cx,
                        ));
                    }
                }
                root.child(surface)
                    .child(self.toolbar(cx))
                    .child(self.status(d, cx))
            }
            None => {
                grid::Tile::release(&mut self.grid, w);
                root.flex().items_center().justify_center()
            }
        };
        let status = owner.status();
        let message = match status {
            LoadStatus::Loading | LoadStatus::Idle => "Loading ERD...".into(),
            LoadStatus::Retired => "Schema diagram unavailable".into(),
            LoadStatus::Failed(message) => format!("Could not load schema diagram: {message}"),
            LoadStatus::Partial(failures) => {
                let failure = &failures[self.failure_index];
                format!(
                    "Incomplete schema metadata ({} of {}): {} · {:?} · {}",
                    self.failure_index + 1,
                    failures.len(),
                    failure.table.name,
                    failure.section,
                    failure.message
                )
            }
            LoadStatus::Ready if ready.is_none() => {
                format!("No tables found in schema \"{}\"", self.schema)
            }
            LoadStatus::Ready => return root,
        };
        let banner = div()
            .id("erd-load-state")
            .occlude()
            .test_support()
            .flex()
            .items_center()
            .gap(px(8.))
            .when(ready.is_some(), |s| {
                s.absolute()
                    .left(px(12.))
                    .top(px(12.))
                    .right(px(58.))
                    .bg(cx.theme().group_box)
                    .p_1()
                    .rounded(px(4.))
            })
            .when(
                matches!(status, LoadStatus::Loading | LoadStatus::Idle),
                |s| {
                    s.child(
                        Spinner::new()
                            .with_size(px(20.))
                            .color(cx.theme().muted_foreground),
                    )
                },
            )
            .child(
                Label::new(message)
                    .id("message")
                    .min_w_0()
                    .text_size(px(14.))
                    .text_color(cx.theme().muted_foreground),
            )
            .children(match status {
                LoadStatus::Partial(failures) if failures.len() > 1 => {
                    let published = std::sync::Arc::downgrade(failures);
                    Some(
                        div()
                            .id("Next failure")
                            .test_support()
                            .role(Role::Button)
                            .aria_label("Next failure")
                            .tab_index(0)
                            .cursor_pointer()
                            .px_2()
                            .child("Next failure")
                            .on_click(cx.listener(move |p, _, _, cx| {
                                let owner = p.model.owner.borrow();
                                if let Some(LoadStatus::Partial(current)) =
                                    owner.as_ref().map(DiagramOwner::status)
                                    && published.ptr_eq(&std::sync::Arc::downgrade(current))
                                {
                                    p.failure_index = (p.failure_index + 1) % current.len();
                                    cx.notify();
                                }
                            })),
                    )
                }
                _ => None,
            })
            .when(
                matches!(status, LoadStatus::Partial(_) | LoadStatus::Failed(_)),
                |s| {
                    s.child(
                        div()
                            .id("erd-retry")
                            .test_support()
                            .role(Role::Button)
                            .aria_label("Retry")
                            .tab_index(0)
                            .cursor_pointer()
                            .px_2()
                            .border_1()
                            .border_color(cx.theme().border)
                            .rounded(px(4.))
                            .child("Retry")
                            .on_click(cx.listener(|p, _, _, cx| p.refresh(cx))),
                    )
                },
            );
        root = root.child(banner);
        root
    }
}

impl Panel {
    fn toolbar(&self, cx: &Context<Self>) -> Div {
        div()
            .absolute()
            .top(px(12.))
            .right(px(12.))
            .flex()
            .flex_col()
            .gap(px(4.))
            .children(
                [
                    ("Zoom in", IconName::ZoomIn, Some(1.2)),
                    ("Zoom out", IconName::ZoomOut, Some(0.8)),
                    ("Fit to view", IconName::Maximize, None),
                    ("Export SVG", IconName::Download, None),
                ]
                .map(|(label, icon, factor)| {
                    div()
                        .id(label)
                        .occlude()
                        .test_support()
                        .role(Role::Button)
                        .aria_label(label)
                        .tab_index(0)
                        .cursor_pointer()
                        .p(px(6.))
                        .border_1()
                        .border_color(cx.theme().border)
                        .rounded(px(4.))
                        .bg(cx.theme().group_box)
                        .text_color(cx.theme().foreground)
                        .shadow_sm()
                        .flex()
                        .items_center()
                        .justify_center()
                        .hover(|s| s.bg(cx.theme().accent))
                        .tooltip(move |w, cx| Tooltip::new(label).build(w, cx))
                        .child(Icon::new(icon).size(px(16.)))
                        .on_click(cx.listener(move |p, _, _, cx| {
                            if let Some(factor) = factor {
                                p.zoom(factor, None, cx)
                            } else if icon == IconName::Download {
                                p.export_svg(cx)
                            } else {
                                p.fit(cx)
                            }
                        }))
                }),
            )
    }
    fn status(&self, d: &DiagramState, cx: &App) -> impl IntoElement + use<> {
        div()
            .id("erd-counts")
            .occlude()
            .test_support()
            .absolute()
            .right(px(12.))
            .bottom(px(12.))
            .px(px(8.))
            .py(px(2.))
            .rounded(px(4.))
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().group_box.opacity(0.8))
            .flex()
            .items_center()
            .gap(px(8.))
            .children(self.export_message.clone().map(|message| {
                Label::new(message)
                    .id("erd-export-status")
                    .text_size(px(12.))
            }))
            .children(
                [
                    format!("{} tables", d.graph().tables().len()),
                    format!("{} FKs", d.graph().edges().len()),
                    format!("{}%", (self.camera.zoom * 100.).round()),
                ]
                .into_iter()
                .enumerate()
                .map(|(i, s)| {
                    Label::new(s)
                        .id(i)
                        .text_size(px(12.))
                        .line_height(px(16.))
                        .text_color(cx.theme().muted_foreground)
                }),
            )
    }
}
