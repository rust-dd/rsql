use super::*;
use gpui_kit::component::label::Label;
use view::{heading, secondary};

impl Panel {
    pub(super) fn enums(&self, cx: &Context<Self>) -> Div {
        let mut body = div()
            .id("enum-groups")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p_4();
        let mut schema = None;
        if let Some(Rows::Enums(rows)) = &self.rows {
            for (index, e) in rows
                .iter()
                .enumerate()
                .filter(|(_, e)| self.filters.enum_type(e))
            {
                if schema != Some(&e.schema) {
                    body = body.child(heading(("enum-schema", index), &e.schema, cx).my_2());
                    schema = Some(&e.schema);
                }
                let title = div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        Icon::new(IconName::List)
                            .size(px(14.))
                            .text_color(cx.theme().primary),
                    )
                    .child(
                        Label::new(e.name.clone())
                            .text_size(px(14.))
                            .font_weight(FontWeight::MEDIUM),
                    )
                    .child(secondary(
                        "enum-count",
                        format!("{} values", e.labels.len()),
                        cx,
                    ));
                body = body.child(
                    div()
                        .id(("enum-type", index))
                        .mb_2()
                        .rounded_md()
                        .border_1()
                        .border_color(cx.theme().border)
                        .p_3()
                        .child(title)
                        .child(div().mt_2().flex().flex_wrap().gap_1().children(
                            e.labels.iter().enumerate().map(|(i, s)| {
                                div()
                                    .rounded_full()
                                    .px_2()
                                    .py(px(2.))
                                    .bg(cx.theme().primary.opacity(0.1))
                                    .child(
                                        Label::new(s.clone())
                                            .id(("enum-label", i))
                                            .text_size(px(11.))
                                            .text_color(cx.theme().primary),
                                    )
                            }),
                        )),
                );
            }
        }
        if schema.is_none() && self.main.ready() {
            body = body.child(
                Label::new(if self.filters.applied() {
                    "No matching enum types"
                } else {
                    "No enum types found"
                })
                .text_color(cx.theme().muted_foreground),
            );
        }
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                self.header(IconName::List, "Enum Types", cx)
                    .child(self.search_input(192.))
                    .child(self.refresh_button(cx)),
            )
            .children(self.status(Lane::Main, &self.main, cx))
            .child(body)
    }
}
