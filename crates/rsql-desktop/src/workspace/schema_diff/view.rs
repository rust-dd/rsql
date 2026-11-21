use super::*;
use gpui_kit::{
    assets::IconName,
    component::{
        ActiveTheme, Disableable, Icon, Selectable,
        button::{Button, ButtonCustomVariant, ButtonVariants},
        label::Label,
    },
    prelude::FluentBuilder,
};
use theme::controls::{accent_gradient, compact_button};

impl Panel {
    fn config(&self, cx: &Context<Self>) -> impl IntoElement {
        let selector = |side: usize| {
            let label = if self.schemas[side].is_empty() {
                ["Left schema…", "Right schema…"][side].to_owned()
            } else {
                self.schemas[side].clone()
            };
            compact_button(Button::new(("schema-picker", side)))
                .accessibility_label(format!("{} schema: {label}", ["Left", "Right"][side]))
                .tooltip(label.clone())
                .label(label)
                .min_w(px(140.))
                .on_click(cx.listener(move |p, _, _, cx| {
                    p.picker = (p.picker != Some(side)).then_some(side);
                    cx.notify();
                }))
        };
        let disabled = self.pending.is_some() || self.schemas.iter().any(String::is_empty);
        let label = if self.pending.is_some() {
            "Comparing…"
        } else if self.result.is_some() || self.error.is_some() {
            "Refresh"
        } else {
            "Compare"
        };
        let tint = ButtonCustomVariant::new(cx)
            .foreground(white())
            .hover(white().opacity(0.1))
            .active(black().opacity(0.1));
        div()
            .id("schema-compare-config")
            .overflow_x_scroll()
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                Icon::new(IconName::GitCompare)
                    .size(px(16.))
                    .text_color(cx.theme().primary),
            )
            .child(selector(0))
            .child(
                Icon::new(IconName::ArrowLeftRight)
                    .size(px(14.))
                    .text_color(cx.theme().muted_foreground),
            )
            .child(selector(1))
            .child(
                div()
                    .rounded(px(8.))
                    .bg(accent_gradient())
                    .when(disabled, |d| d.opacity(0.5))
                    .child(
                        compact_button(Button::new("schema-compare"))
                            .icon(
                                if self.result.is_some()
                                    || self.pending.is_some()
                                    || self.error.is_some()
                                {
                                    IconName::RefreshCw
                                } else {
                                    IconName::GitCompare
                                },
                            )
                            .label(label)
                            .custom(tint)
                            .disabled(disabled)
                            .on_click(cx.listener(|p, _, _, cx| p.compare(cx))),
                    ),
            )
    }

    fn schema_options(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let project = self.capture.project.clone();
        let owner = self.owner.clone();
        let count = self
            .owner
            .read_with(cx, |v, _| {
                v.navigation.catalogs.get(&project).map(|c| {
                    c.schema_names()
                        .map(|n| n.map(<[_]>::len))
                        .map_err(str::to_owned)
                })
            })
            .ok()
            .flatten()?;
        let count = match count {
            Ok(Some(n)) => n,
            Ok(None) => {
                return Some(
                    div()
                        .p_2()
                        .child(Label::new("Loading schemas…"))
                        .into_any_element(),
                );
            }
            Err(error) => {
                return Some(
                    div()
                        .p_2()
                        .text_color(cx.theme().danger)
                        .child(Label::new(error))
                        .child(
                            compact_button(Button::new("retry-schemas"))
                                .label("Retry loading schemas")
                                .on_click(move |_, _, cx| {
                                    let _ = owner.update(cx, |v, cx| {
                                        v.ensure_schema_names(&project, true, cx)
                                    });
                                }),
                        )
                        .into_any_element(),
                );
            }
        };
        let side = self.picker?;
        if count == 0 {
            return Some(
                div()
                    .p_2()
                    .child(Label::new("No schemas available"))
                    .into_any_element(),
            );
        }
        let panel = cx.weak_entity();
        Some(
            div()
                .h(px(28. * count.min(7) as f32))
                .child(
                    uniform_list("schema-options", count, move |range, _, cx| {
                        let names = owner
                            .read_with(cx, |v, _| {
                                v.navigation
                                    .catalogs
                                    .get(&project)
                                    .and_then(|c| c.schema_names().ok().flatten())
                                    .and_then(|names| names.get(range))
                                    .unwrap_or_default()
                                    .to_vec()
                            })
                            .unwrap_or_default();
                        names
                            .into_iter()
                            .map(|name| {
                                let panel = panel.clone();
                                compact_button(Button::new(SharedString::from(format!(
                                    "schema-option-{name}"
                                ))))
                                .label(name.clone())
                                .on_click(move |_, _, cx| {
                                    let _ =
                                        panel.update(cx, |p, cx| p.choose(side, name.clone(), cx));
                                })
                            })
                            .collect()
                    })
                    .size_full(),
                )
                .into_any_element(),
        )
    }

    fn summary(&self, result: &Comparison, cx: &Context<Self>) -> impl IntoElement {
        let mut bar = div()
            .id("schema-counts")
            .overflow_x_scroll()
            .flex()
            .items_center()
            .gap_1()
            .px_3()
            .py_1()
            .border_b_1()
            .border_color(cx.theme().border);
        for (i, (filter, label)) in [
            (Filter::All, "All".into()),
            (Filter::Changes, "Changes".into()),
            (Filter::Status(Status::Modified), "Modified".into()),
            (
                Filter::Status(Status::OnlyLeft),
                format!("Only in {}", self.schemas[0]),
            ),
            (
                Filter::Status(Status::OnlyRight),
                format!("Only in {}", self.schemas[1]),
            ),
            (Filter::Status(Status::Identical), "Identical".into()),
        ]
        .into_iter()
        .enumerate()
        {
            let count = result.range(filter).len();
            if count == 0 && matches!(filter, Filter::Status(_)) && filter != self.filter {
                continue;
            }
            let color = match filter {
                Filter::Status(s) => status_color(s, cx),
                _ => cx.theme().foreground,
            };
            bar = bar.child(
                compact_button(Button::new(("schema-filter", i)))
                    .ghost()
                    .label(format!("{label} ({count})"))
                    .text_size(px(11.))
                    .text_color(color)
                    .selected(filter == self.filter)
                    .when(filter == self.filter, |b| b.bg(color.opacity(0.12)))
                    .on_click(cx.listener(move |p, _, _, cx| p.filter(filter, cx))),
            );
        }
        bar
    }

    fn object_list(&self, cx: &Context<Self>) -> Div {
        let list = div()
            .w(px(320.))
            .min_w_0()
            .max_w(relative(0.5))
            .h_full()
            .border_r_1()
            .border_color(cx.theme().border);
        let hint = if let Some(error) = &self.error {
            Some(error.as_str())
        } else if self.pending.is_some() {
            Some("Comparing schemas…")
        } else if self.result.is_none() {
            Some("Select schemas and compare")
        } else {
            None
        };
        if let Some(message) = hint {
            return list.p_3().child(Label::new(message.to_owned()));
        }
        let result = self.result.as_ref().unwrap().clone();
        let rows = result.range(self.filter);
        if rows.is_empty() {
            let message = if matches!(self.filter, Filter::Status(_)) {
                "No objects match this filter."
            } else {
                "No differences found in ordinary table columns, views, materialized views, routines or indexes. Table comparison excludes partitioned roots and foreign tables."
            };
            return list.p_3().child(Label::new(message));
        }
        let selected = self.selected.as_ref().map(|d| d.index);
        let panel = cx.weak_entity();
        list.child(
            uniform_list("schema-objects", rows.len(), move |visible, _, cx| {
                visible
                    .map(|i| {
                        let index = rows.start + i;
                        let entry = &result.entries[index];
                        let owner = panel.clone();
                        let comparison = result.clone();
                        let kind = match entry.kind.as_str() {
                            "table" => IconName::Table,
                            "view" | "matview" => IconName::Eye,
                            "index" => IconName::Hash,
                            _ => IconName::Code,
                        };
                        let (status, icon) = match entry.status {
                            Status::Modified => ("Modified", IconName::RefreshCw),
                            Status::OnlyLeft => ("Only left", IconName::Minus),
                            Status::OnlyRight => ("Only right", IconName::Plus),
                            Status::Identical => ("Identical", kind),
                        };
                        compact_button(Button::new(("schema-object", index)))
                            .ghost()
                            .w_full()
                            .justify_start()
                            .selected(selected == Some(index))
                            .child(
                                Icon::new(icon)
                                    .size(px(12.))
                                    .text_color(status_color(entry.status, cx)),
                            )
                            .child(Icon::new(kind).size(px(12.)))
                            .child(div().min_w_0().truncate().child(entry.name.clone()))
                            .child(
                                div()
                                    .ml_auto()
                                    .text_size(px(9.))
                                    .text_color(cx.theme().muted_foreground)
                                    .child(entry.kind.clone()),
                            )
                            .accessibility_label(format!(
                                "{status} · {} · {}",
                                entry.kind, entry.name
                            ))
                            .on_click(move |_, w, cx| {
                                let _ =
                                    owner.update(cx, |p, cx| p.select(&comparison, index, w, cx));
                            })
                    })
                    .collect()
            })
            .size_full(),
        )
    }
}

fn status_color(status: Status, cx: &App) -> Hsla {
    match status {
        Status::Modified => cx.theme().warning,
        Status::OnlyLeft => cx.theme().danger,
        Status::OnlyRight => cx.theme().success,
        Status::Identical => cx.theme().muted_foreground,
    }
}

impl Render for Panel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("schema-diff-panel")
            .test_support()
            .size_full()
            .min_w_0()
            .flex()
            .flex_col()
            .text_size(px(12.))
            .child(self.config(cx))
            .when_some(
                self.picker.filter(|s| !self.schemas[*s].is_empty()),
                |panel, side| {
                    panel.child(
                        div().child(
                            compact_button(Button::new("clear-schema"))
                                .label("Clear")
                                .on_click(cx.listener(move |p, _, _, cx| {
                                    p.choose(side, String::new(), cx)
                                })),
                        ),
                    )
                },
            )
            .children(self.schema_options(cx))
            .children(self.result.as_ref().map(|r| self.summary(r, cx)))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(self.object_list(cx))
                    .child(self.detail(cx)),
            )
    }
}
