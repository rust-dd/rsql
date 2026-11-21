use super::*;
use gpui_kit::{
    assets::IconName,
    base::{Button as ActionButton, TestTextExt},
    component::{
        ActiveTheme, Icon, Sizable, button::Button, input::Input, label::Label, tooltip::Tooltip,
    },
    prelude::FluentBuilder,
};

fn input(id: &'static str, state: &Entity<InputState>, cx: &App) -> impl IntoElement + Styled {
    div().id(id).test_support().h(px(28.)).min_w_0().child(
        Styled::h(Input::new(state).small().px_3().py_1(), px(28.))
            .text_size(px(12.))
            .bg(cx.theme().input.opacity(0.5))
            .rounded(px(8.))
            .border_color(cx.theme().border.opacity(0.5)),
    )
}

fn toolbar(cx: &App) -> Div {
    div()
        .flex()
        .items_center()
        .flex_shrink_0()
        .gap_2()
        .px_3()
        .py_2()
        .border_color(cx.theme().border.opacity(0.3))
}

fn action(id: &'static str, label: &'static str, icon: IconName, cx: &App) -> ActionButton {
    let clear = id == "notify-clear";
    let theme = cx.theme();
    let hover = if !clear {
        theme.accent
    } else if theme.is_dark() {
        white().opacity(0.06)
    } else {
        black().opacity(0.04)
    };
    let hover_fg = theme.accent_foreground;
    let ring = theme.ring;
    ActionButton::new(id)
        .h(px(28.))
        .px_2()
        .gap_1()
        .rounded(px(8.))
        .text_size(px(12.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(theme.foreground)
        .hover(|s| s.bg(hover).text_color(hover_fg))
        .focus_visible(|s| {
            s.shadow(vec![
                BoxShadow::new(px(0.), px(0.), ring).spread_radius(px(2.)),
            ])
        })
        .when(!clear, |s| s.border_1().border_color(theme.border))
        .accessibility_label(label)
        .child(Icon::new(icon).size(px(12.)))
        .when(!clear, |b| b.child(div().child(label).test_text(label)))
        .when(clear, |b| {
            b.tooltip(move |w, cx| Tooltip::new(label).build(w, cx))
        })
}

impl Panel {
    fn chips(&self, listening: bool, cx: &Context<Self>) -> Option<Div> {
        let names = if listening {
            self.channels
                .iter()
                .filter(|(_, entry)| entry.active)
                .map(|(name, _)| name)
                .collect::<Vec<_>>()
        } else {
            self.known.iter().collect()
        };
        if names.is_empty() {
            return None;
        }
        let theme = cx.theme();
        let primary = theme.primary;
        let danger = theme.danger;
        let ring = theme.ring;
        let row = div()
            .flex()
            .items_center()
            .flex_wrap()
            .gap_1()
            .px_3()
            .py(px(6.))
            .border_b_1()
            .border_color(theme.border.opacity(0.2))
            .when(!listening, |r| {
                r.child(
                    Icon::new(IconName::Radio)
                        .size(px(12.))
                        .mr(px(2.))
                        .text_color(theme.muted_foreground.opacity(0.5)),
                )
            })
            .child(
                Label::new(if listening {
                    "LISTENING:"
                } else {
                    "AVAILABLE:"
                })
                .id(if listening {
                    "listening-heading"
                } else {
                    "available-heading"
                })
                .text_size(px(10.))
                .mr_1()
                .text_color(theme.muted_foreground),
            );
        Some(row.children(names.into_iter().map(|name| {
            let active = self.channels.get(name).is_some_and(|entry| entry.active);
            let disabled = !listening && self.channels.contains_key(name);
            let id = format!(
                "{}-{name}",
                if listening {
                    "unsubscribe"
                } else {
                    "available"
                }
            );
            let (bg, fg) = if listening {
                (primary.opacity(0.1), primary)
            } else if active {
                (primary.opacity(0.1), primary.opacity(0.5))
            } else {
                (theme.muted.opacity(0.3), theme.muted_foreground)
            };
            let hover = if listening { danger } else { primary };
            let channel = name.clone();
            ActionButton::new(SharedString::from(id))
                .rounded_full()
                .px_2()
                .py(px(2.))
                .gap_1()
                .text_size(px(11.))
                .bg(bg)
                .text_color(fg)
                .disabled(disabled)
                .focus_visible(|s| {
                    s.shadow(vec![
                        BoxShadow::new(px(0.), px(0.), ring).spread_radius(px(2.)),
                    ])
                })
                .when(!disabled, |b| {
                    b.hover(|s| s.bg(hover.opacity(0.1)).text_color(hover))
                })
                .accessibility_label(format!(
                    "{} {name}",
                    if listening {
                        "Unsubscribe from"
                    } else {
                        "Subscribe to"
                    }
                ))
                .child(div().child(name.clone()).test_text(name.clone()))
                .when(active, |b| {
                    b.child(
                        Icon::new(if listening {
                            IconName::BellOff
                        } else {
                            IconName::Bell
                        })
                        .size(px(10.)),
                    )
                })
                .when(listening, |b| {
                    b.tooltip(|w, cx| Tooltip::new("Click to unsubscribe").build(w, cx))
                })
                .on_click(cx.listener(move |p, _, w, cx| {
                    if listening {
                        p.unsubscribe(&channel, cx);
                    } else {
                        p.subscribe(&channel, None, w, cx);
                    }
                }))
        })))
    }

    fn transcript(&self, cx: &Context<Self>) -> Div {
        let area = div().flex_1().min_h_0().min_w_0().p_3();
        if self.log.rows.is_empty() {
            return area.flex().items_center().justify_center().child(
                Label::new(if self.channels.values().any(|entry| entry.active) {
                    "Waiting for notifications..."
                } else {
                    "Subscribe to a channel to start"
                })
                .text_color(cx.theme().muted_foreground.opacity(0.4))
                .text_size(px(14.)),
            );
        }
        let owner = cx.entity();
        area.child(
            list(self.list.clone(), move |index, _, cx| {
                let panel = owner.read(cx);
                let Some(row) = panel.log.rows.get(index) else {
                    return div().into_any_element();
                };
                let theme = cx.theme();
                let empty = row.payload.is_empty();
                div()
                    .id(("notification", row.id))
                    .w_full()
                    .min_w_0()
                    .flex()
                    .items_start()
                    .gap_2()
                    .mb_1()
                    .px_2()
                    .py(px(6.))
                    .rounded(px(8.))
                    .hover(|s| s.bg(theme.muted.opacity(0.3)))
                    .child(
                        Label::new(row.time.format("%H:%M:%S").to_string())
                            .id("received-at")
                            .flex_shrink_0()
                            .text_color(theme.muted_foreground.opacity(0.5)),
                    )
                    .child(
                        Label::new(row.channel.clone())
                            .id("received-channel")
                            .flex_shrink_0()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme.primary),
                    )
                    .child(
                        Label::new(if empty {
                            "empty".into()
                        } else {
                            row.payload.clone()
                        })
                        .id("received-payload")
                        .flex_1()
                        .min_w_0()
                        .whitespace_normal()
                        .when(empty, |s| {
                            s.italic().text_color(theme.muted_foreground.opacity(0.4))
                        }),
                    )
                    .into_any_element()
            })
            .size_full(),
        )
    }
}

impl Render for Panel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("notify-panel")
            .test_support()
            .size_full()
            .min_w_0()
            .flex()
            .flex_col()
            .text_size(px(12.))
            .bg(cx.theme().background)
            .child(
                toolbar(cx)
                    .border_b_1()
                    .child(
                        Icon::new(IconName::Bell)
                            .size(px(14.))
                            .text_color(cx.theme().primary),
                    )
                    .child(input("notify-channel", &self.channel, cx).flex_1())
                    .child(
                        action("notify-subscribe", "Subscribe", IconName::Bell, cx)
                            .on_click(cx.listener(|p, _, w, cx| p.subscribe_manual(w, cx))),
                    ),
            )
            .children(self.chips(false, cx))
            .children(self.chips(true, cx))
            .when_some(self.discovery_error.as_ref(), |s, error| {
                s.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            Label::new(format!("Channel discovery: {error}"))
                                .text_color(cx.theme().danger),
                        )
                        .child(
                            theme::controls::compact_button(Button::new("notify-retry"))
                                .label("Retry")
                                .on_click(cx.listener(|p, _, _, cx| {
                                    if p.selected() {
                                        p.discover(cx);
                                        cx.notify();
                                    }
                                })),
                        ),
                )
            })
            .when_some(self.subscribe_error.as_ref(), |s, e| {
                s.child(Label::new(format!("Subscribe: {e}")).text_color(cx.theme().danger))
            })
            .when(self.log.dropped > 0, |s| {
                s.child(
                    Label::new(format!("Dropped entries: {}", self.log.dropped))
                        .px_3()
                        .text_color(cx.theme().muted_foreground),
                )
            })
            .child(self.transcript(cx))
            .when_some(self.send_error.as_ref(), |s, e| {
                s.child(Label::new(format!("Send: {e}")).text_color(cx.theme().danger))
            })
            .child(
                toolbar(cx)
                    .border_t_1()
                    .child(
                        input("notify-target", &self.target, cx)
                            .w(px(140.))
                            .flex_shrink_0(),
                    )
                    .child(input("notify-payload", &self.payload, cx).flex_1())
                    .child(
                        action("notify-send", "Send", IconName::Send, cx)
                            .disabled(self.sending.is_some())
                            .on_click(cx.listener(|p, _, w, cx| p.send(w, cx))),
                    )
                    .child(
                        action("notify-clear", "Clear", IconName::Trash, cx)
                            .on_click(cx.listener(|p, _, _, cx| p.clear(cx))),
                    ),
            )
    }
}
