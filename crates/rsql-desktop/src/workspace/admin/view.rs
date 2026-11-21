use super::*;
use gpui_kit::component::label::Label;

pub(super) fn badge(id: impl Into<ElementId>, label: impl Into<SharedString>, color: Hsla) -> Div {
    div()
        .rounded_full()
        .px(px(6.))
        .py(px(2.))
        .text_size(px(9.))
        .bg(color.opacity(0.1))
        .child(Label::new(label).id(id).text_color(color))
}

pub(super) fn secondary(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    cx: &App,
) -> Label {
    Label::new(label)
        .id(id)
        .text_size(px(10.))
        .text_color(cx.theme().muted_foreground)
}

pub(super) fn heading(id: impl Into<ElementId>, label: &str, cx: &App) -> Div {
    div().child(secondary(id, label.to_uppercase(), cx).font_weight(FontWeight::SEMIBOLD))
}

impl Panel {
    pub(super) fn status(&self, lane: Lane, section: &Section, cx: &Context<Self>) -> Option<Div> {
        if let Some(error) = &section.error {
            let id = match lane {
                Lane::Main => "retry-admin",
                Lane::Available => "retry-available-extensions",
                Lane::Database => "retry-database-grants",
                Lane::Table => "retry-table-grants",
            };
            return Some(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .p_2()
                    .child(
                        Label::new(error.clone())
                            .id(("admin-error", lane as usize))
                            .text_color(cx.theme().danger),
                    )
                    .child(
                        compact_button(Button::new(id))
                            .label("Retry")
                            .on_click(cx.listener(move |p, _, _, cx| {
                                if p.current(cx) {
                                    p.request(lane, cx);
                                }
                            })),
                    ),
            );
        }
        section.pending.as_ref().map(|_| {
            div()
                .p_2()
                .child(Label::new("Loading…").text_color(cx.theme().muted_foreground))
        })
    }

    pub(super) fn search_input(&self, width: f32) -> impl IntoElement {
        div()
            .id("admin-search")
            .test_support()
            .w(px(width))
            .child(Input::new(&self.search).small().h(px(28.)))
    }

    pub(super) fn refresh_button(&self, cx: &Context<Self>) -> Button {
        compact_button(Button::new("refresh-admin"))
            .ghost()
            .icon(IconName::RefreshCw)
            .w(px(28.))
            .tooltip("Refresh")
            .accessibility_label("Refresh")
            .disabled(
                self.main.pending.is_some() || self.extensions.available_section.pending.is_some(),
            )
            .on_click(cx.listener(|p, _, _, cx| p.refresh(cx)))
    }

    pub(super) fn header(&self, icon: IconName, title: &'static str, cx: &App) -> Div {
        div()
            .flex()
            .flex_shrink_0()
            .items_center()
            .gap_2()
            .px_4()
            .py_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(Icon::new(icon).size(px(16.)).text_color(cx.theme().primary))
            .child(
                Label::new(title)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(px(14.)),
            )
            .child(Label::new(self.database.clone()).text_color(cx.theme().muted_foreground))
            .child(div().flex_1())
    }
}

impl Render for Panel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("admin-panel")
            .test_support()
            .size_full()
            .min_w_0()
            .flex()
            .flex_col()
            .text_size(px(12.))
            .child(match self.kind {
                Kind::Roles => self.roles(cx),
                Kind::Enums => self.enums(cx),
                Kind::Extensions => self.extensions_view(cx),
                Kind::PgSettings => self.settings(cx),
                _ => unreachable!(),
            })
    }
}
