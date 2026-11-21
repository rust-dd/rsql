use super::*;

#[derive(Clone)]
struct SidebarDrag;

impl Workspace {
    pub(super) fn sidebar_divider(&self, width: f32, cx: &Context<Self>) -> impl IntoElement {
        let primary = cx.theme().primary.opacity(0.5);
        div()
            .id("sidebar-divider")
            .test_support()
            .absolute()
            .top_0()
            .bottom_0()
            .left(px(width - 3.))
            .w(px(7.))
            .flex()
            .items_center()
            .justify_center()
            .group("sidebar-resize")
            .cursor_col_resize()
            .occlude()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |v, e: &MouseDownEvent, w, _| {
                    w.prevent_default();
                    v.resize_origin = (e.position.x, width);
                }),
            )
            .on_drag(SidebarDrag, |_, _, _, cx| cx.new(|_| Empty))
            .on_drag_move(cx.listener(|v, e: &DragMoveEvent<SidebarDrag>, _, cx| {
                let (x, width) = v.resize_origin;
                v.sidebar_width = (width + f32::from(e.event.position.x - x)).clamp(180., 700.);
                cx.notify();
            }))
            .child(
                div()
                    .w(px(1.))
                    .h_full()
                    .bg(cx.theme().border)
                    .group_hover("sidebar-resize", |s| s.opacity(0.)),
            )
            .child(
                div()
                    .absolute()
                    .w(px(4.))
                    .h_full()
                    .group_hover("sidebar-resize", |s| s.bg(primary)),
            )
            .child(
                div()
                    .absolute()
                    .w(px(4.))
                    .h(px(48.))
                    .rounded_full()
                    .group_hover("sidebar-resize", |s| s.bg(primary)),
            )
    }

    pub(super) fn editor_split(
        &self,
        side: Side,
        toolbar: Option<gpui_kit::base::ObservedElement<Stateful<Div>>>,
        editor: Div,
        result: impl IntoElement,
        column: Option<Pixels>,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let split = column.is_none();
        let id = match side {
            Side::Main => "editor-divider-main",
            Side::Right => "editor-divider-right",
        };
        let primary = cx.theme().primary.opacity(0.5);
        let fraction = self.editor_fraction;
        let handle = div()
            .id(id)
            .test_support()
            .group(id)
            .absolute()
            .top(px(-4.))
            .w_full()
            .h(px(7.))
            .flex()
            .items_center()
            .justify_center()
            .cursor_row_resize()
            .occlude()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |v, e: &MouseDownEvent, w, _| {
                    w.prevent_default();
                    v.resize_origin = (e.position.y, fraction);
                }),
            )
            .on_drag(side, |_, _, _, cx| cx.new(|_| Empty))
            .child(
                div()
                    .w_full()
                    .h(px(1.))
                    .bg(cx.theme().border)
                    .group_hover(id, |s| s.opacity(0.)),
            )
            .child(
                div()
                    .absolute()
                    .w_full()
                    .h(px(4.))
                    .group_hover(id, |s| s.bg(primary)),
            )
            .child(
                div()
                    .absolute()
                    .w(px(48.))
                    .h(px(4.))
                    .rounded_full()
                    .group_hover(id, |s| s.bg(primary)),
            );
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_hidden()
            .when(split && side == Side::Main, |pane| {
                pane.border_r_1()
                    .border_color(cx.theme().border.opacity(0.3))
            })
            .on_drag_move(cx.listener(move |v, e: &DragMoveEvent<Side>, _, cx| {
                if *e.drag(cx) == side && e.bounds.size.height > px(0.) {
                    let (y, fraction) = v.resize_origin;
                    let height = column.unwrap_or(e.bounds.size.height);
                    v.editor_fraction =
                        (fraction + (e.event.position.y - y) / height).clamp(0.2, 0.8);
                    cx.notify();
                }
            }))
            .children(toolbar)
            .child(
                div()
                    .id(if side == Side::Main {
                        "main-editor-area"
                    } else {
                        "right-editor-area"
                    })
                    .test_support()
                    .map(|area| match column {
                        Some(column) => area.h(column * fraction),
                        None => area.h(relative(fraction)),
                    })
                    .flex_shrink_0()
                    .min_h_0()
                    .child(editor),
            )
            .child(div().h(px(1.)).flex_shrink_0())
            .child(
                div()
                    .relative()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .child(result)
                    .child(handle),
            )
    }
}
