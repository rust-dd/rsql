use super::*;
use gpui_kit::{
    assets::IconName,
    base::Button as Segment,
    component::{
        Disableable, Icon, Sizable,
        button::{Button, ButtonVariants},
        input::Input,
    },
    prelude::FluentBuilder,
};
use rsql_desktop::results::{
    ViewChoice,
    diff::{Pin, SnapshotKey},
};

pub(in crate::workspace) fn result_bar(cx: &App) -> Div {
    div()
        .flex()
        .flex_wrap()
        .items_center()
        .justify_between()
        .gap_2()
        .flex_shrink_0()
        .px_4()
        .py_2()
        .border_b_1()
        .border_color(cx.theme().border.opacity(0.5))
        .bg(cx.theme().group_box.opacity(0.8))
}

pub(in crate::workspace) fn segments(cx: &App) -> gpui_kit::base::ObservedElement<Stateful<Div>> {
    div()
        .id("result-segments")
        .test_support()
        .flex()
        .items_center()
        .p_0p5()
        .rounded(px(8.))
        .bg(cx.theme().muted)
}

pub(in crate::workspace) fn segment(
    label: &'static str,
    icon: Option<IconName>,
    selected: bool,
    disabled: bool,
    cx: &App,
) -> Segment {
    let theme = cx.theme();
    let mut button = Segment::new(label)
        .accessibility_label(label)
        .selected(selected)
        .disabled(disabled)
        .h(px(20.))
        .px_2()
        .gap_1()
        .rounded(px(6.))
        .flex()
        .flex_shrink_0()
        .items_center()
        .text_size(px(12.))
        .line_height(px(16.))
        .text_color(theme.muted_foreground)
        .on_mouse_down(MouseButton::Left, |_, w, _| w.prevent_default())
        .hover(|s| s.text_color(theme.foreground));
    if selected {
        button = button
            .bg(theme.accent)
            .text_color(theme.foreground)
            .shadow(vec![
                BoxShadow::new(px(0.), px(1.), black().opacity(0.1)).blur_radius(px(2.)),
            ]);
    }
    button
        .children(icon.map(|icon| Icon::new(icon).size(px(12.))))
        .child(label)
}

pub(super) fn result_action(button: Button, glyph: IconName, cx: &App) -> Button {
    button
        .small()
        .text_size(px(12.))
        .ghost()
        .text_color(cx.theme().muted_foreground)
        .h(px(20.))
        .px_2()
        .rounded(px(4.))
        .icon(Icon::new(glyph))
}

pub(in crate::workspace) fn result_stop(cx: &App) -> Segment {
    let theme = cx.theme();
    let danger = theme.danger;
    Segment::new("Stop result")
        .accessibility_label("Stop")
        .flex()
        .items_center()
        .gap_1()
        .h(px(24.))
        .px_2p5()
        .rounded(px(4.))
        .border_1()
        .border_color(danger.opacity(0.5))
        .text_size(px(12.))
        .line_height(px(16.))
        .text_color(danger)
        .hover(move |style| style.bg(danger.opacity(0.1)))
        .child(Icon::new(IconName::Square).size(px(12.)))
        .child("Stop")
}

pub(in crate::workspace) fn status_line(
    label: String,
    elapsed_ms: f64,
    capped: bool,
    state: IconName,
    cx: &App,
) -> impl IntoElement {
    let theme = cx.theme();
    let color = match state {
        IconName::CircleCheck => theme.success,
        IconName::CircleX => theme.danger,
        IconName::LoaderCircle => theme.primary,
        _ => theme.muted_foreground,
    };
    div()
        .id("result-stats")
        .test_support()
        .aria_label(label.clone())
        .flex()
        .items_center()
        .gap_2()
        .text_size(px(12.))
        .line_height(px(16.))
        .text_color(theme.muted_foreground)
        .child(Icon::new(state).size(px(12.)).text_color(color))
        .child(label)
        .children(capped.then(|| div().text_color(theme.warning).child("(truncated)")))
        .child(
            div()
                .text_color(theme.muted_foreground.opacity(0.5))
                .child("•"),
        )
        .child(Icon::new(IconName::Clock).size(px(12.)))
        .child(format!("{elapsed_ms:.0}ms"))
}

impl Workspace {
    pub(in crate::workspace) fn pin_result(
        &mut self,
        tab: &str,
        key: SnapshotKey,
        cx: &mut Context<Self>,
    ) {
        if *self.session.preflight_stop.borrow() {
            return;
        }
        let rows = self
            .session
            .documents
            .borrow()
            .active()
            .filter(|t| t.id == tab)
            .and_then(|t| t.main.view())
            .filter(|v| v.key() == key)
            .and_then(|v| v.snapshot(ResultAction::Pin));
        if let Some(rows) = rows {
            let label = format!("{} rows", rows.len());
            self.replace_pin(
                Some(Pin {
                    id: Uuid::new_v4(),
                    rows,
                    label,
                }),
                cx,
            );
        }
    }

    pub(in crate::workspace) fn can_clear_pin(&self, id: Uuid) -> bool {
        self.session
            .pin
            .borrow()
            .as_ref()
            .is_some_and(|p| p.id == id)
            && self
                .session
                .documents
                .borrow()
                .active()
                .and_then(|t| t.main.view())
                .is_none_or(|v| v.edit.is_none())
    }

    pub(in crate::workspace) fn clear_pin(&mut self, id: Uuid, cx: &mut Context<Self>) {
        if self.can_clear_pin(id) {
            self.replace_pin(None, cx);
        }
    }
    fn replace_pin(&mut self, pin: Option<Pin>, cx: &mut Context<Self>) {
        let clearing = pin.is_none();
        self.session.pin.replace(pin);
        let mut docs = self.session.documents.borrow_mut();
        for tab in &mut docs.tabs {
            if let Some(v) = tab.main.view_mut() {
                v.comparison = None;
                if v.choice() == ViewChoice::Diff {
                    v.field_detail = None;
                }
                if clearing && v.choice() == ViewChoice::Diff {
                    v.choose(ViewChoice::Grid);
                }
            }
        }
        docs.sync_views();
        cx.notify();
    }

    pub(in crate::workspace) fn inspection_toolbar(
        &self,
        owner: Owner,
        view: &ResultView,
        plan: bool,
        cx: &Context<Self>,
    ) -> Div {
        let bar = result_bar(cx);
        let mut controls = div().flex().flex_wrap().items_center().gap_2();
        let mut choices = segments(cx);
        let pin = self.session.pin.borrow();
        for (label, icon, choice, enabled) in [
            ("Grid", None, ViewChoice::Grid, true),
            (
                "Record",
                None,
                ViewChoice::Record,
                view.allows(ResultAction::Record),
            ),
            (
                "Explain",
                Some(IconName::GitBranch),
                ViewChoice::Explain,
                plan,
            ),
            (
                "History",
                Some(IconName::RotateCcwClock),
                ViewChoice::History,
                true,
            ),
            (
                "Map",
                None,
                ViewChoice::Map,
                self.editors
                    .get(&owner)
                    .and_then(|e| e.map.as_ref())
                    .is_some_and(|m| m.read(cx).available(view)),
            ),
            (
                "Diff",
                Some(IconName::Diff),
                ViewChoice::Diff,
                pin.is_some() && view.allows(ResultAction::Diff),
            ),
        ] {
            if (choice == ViewChoice::Map && !enabled)
                || (choice == ViewChoice::Explain && !plan)
                || (choice == ViewChoice::Diff && pin.is_none())
            {
                continue;
            }
            let owner = owner.clone();
            let id = view.data.id;
            let key = view.key();
            let enabled = enabled && view.edit.is_none();
            let change = cx.listener(move |v, _: &ClickEvent, _, cx| {
                let mut docs = v.session.documents.borrow_mut();
                let Some(tab) = docs
                    .selected
                    .map(|i| &mut docs.tabs[i])
                    .filter(|t| t.id == owner.0)
                else {
                    return;
                };
                let plan = tab
                    .main
                    .outcome()
                    .is_some_and(|o| matches!(o.status, rsql_desktop::execution::Status::Plan(_)));
                if let Some(view) = tab
                    .main
                    .view_mut()
                    .filter(|v| v.data.id == id && v.edit.is_none())
                {
                    if choice == ViewChoice::Map {
                        if v.editors
                            .get(&owner)
                            .and_then(|e| e.map.as_ref())
                            .is_some_and(|m| m.read(cx).available(view))
                        {
                            view.choose_main_map(key);
                        }
                    } else {
                        let choice = if choice == ViewChoice::Diff && view.choice() == choice {
                            ViewChoice::Grid
                        } else {
                            choice
                        };
                        view.choose_available(choice, plan, v.session.pin.borrow().is_some());
                    }
                    docs.sync_views();
                    cx.notify();
                }
            });
            if choice == ViewChoice::Diff {
                if view.choice() != ViewChoice::History {
                    let button = result_action(
                        Button::new(label).label(label).on_click(change),
                        IconName::Diff,
                        cx,
                    )
                    .disabled(!enabled);
                    controls = controls.child(if view.choice() == choice {
                        button.primary()
                    } else {
                        button
                    });
                }
            } else {
                choices = choices.child(
                    segment(label, icon, view.choice() == choice, !enabled, cx).on_click(change),
                );
            }
        }
        let mut left = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_3()
            .child(choices);
        if view.choice() == ViewChoice::History {
            return bar.child(left);
        }
        let rows = &view.data;
        let count = match &view.filtered_rows {
            Some(matches) => format!("{} / {} rows", matches.len(), rows.total_rows),
            None => format!("{} rows", rows.total_rows),
        };
        left = left.child(status_line(
            count,
            f64::from(rows.elapsed_ms),
            rows.capped,
            IconName::CircleCheck,
            cx,
        ));
        if view.edit.is_some() {
            return bar.child(left).child(self.edit_toolbar(owner, view, cx));
        }
        if view.choice() == ViewChoice::Grid && view.data.is_rowset {
            controls = div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_2()
                .child(self.edit_toolbar(owner.clone(), view, cx))
                .child(controls);
        }
        if view.allows(ResultAction::Pin) {
            let tab = owner.0.clone();
            let key = view.key();
            controls = controls.child(result_action(
                self.button("Pin result", cx, move |v, _, cx| {
                    v.pin_result(&tab, key, cx)
                })
                .label(if pin.is_some() { "Re-pin" } else { "Pin" }),
                IconName::Pin,
                cx,
            ));
        }
        if let Some(pin) = pin.as_ref() {
            let id = pin.id;
            let theme = cx.theme();
            controls = controls.child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .px_2()
                    .rounded(px(4.))
                    .h(px(22.))
                    .text_xs()
                    .bg(theme.primary.opacity(0.1))
                    .text_color(theme.primary)
                    .border_1()
                    .border_color(theme.primary.opacity(0.2))
                    .child(Icon::new(IconName::Pin).size(px(12.)))
                    .child(format!("Pinned: {}", pin.label))
                    .child(
                        Button::new("Clear pin")
                            .ghost()
                            .small()
                            .size(px(16.))
                            .accessibility_label("Clear pin")
                            .icon(Icon::new(IconName::X))
                            .on_click(cx.listener(move |v, _, _, cx| v.clear_pin(id, cx))),
                    ),
            );
        }
        controls = controls.child(self.export_toolbar(owner.clone(), view, cx));
        if let Some(search) = self
            .editors
            .get(&owner)
            .and_then(|l| l.search.as_ref())
            .filter(|s| s.id == view.data.id && view.allows(ResultAction::Search))
        {
            let search_id = search.id;
            let search_owner = owner.clone();
            let clear_owner = owner.clone();
            let has_text = search.state.read(cx).text().len() != 0;
            let theme = cx.theme();
            controls = controls.child(
                div()
                    .w(px(192.))
                    .capture_key_down(cx.listener(move |v, e: &KeyDownEvent, w, cx| {
                        if e.keystroke.key == "escape" {
                            v.clear_search(&search_owner, search_id, w, cx);
                            cx.stop_propagation();
                        }
                    }))
                    .child(Styled::h(
                        Input::new(&search.state)
                            .id("result-search")
                            .small()
                            .text_size(px(12.))
                            .rounded(px(4.))
                            .bg(theme.input)
                            .prefix(
                                Icon::new(IconName::Search)
                                    .size(px(12.))
                                    .text_color(theme.muted_foreground),
                            )
                            .when(has_text, |input| {
                                input.suffix(
                                    Button::new("Clear search")
                                        .ghost()
                                        .small()
                                        .size(px(16.))
                                        .accessibility_label("Clear search")
                                        .icon(Icon::new(IconName::X))
                                        .on_click(cx.listener(move |v, _, w, cx| {
                                            v.clear_search(&clear_owner, search_id, w, cx)
                                        })),
                                )
                            }),
                        px(28.),
                    )),
            );
        }
        bar.child(left).child(controls)
    }
}
