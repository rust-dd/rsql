use super::*;
use gpui_kit::component::{
    label::Label,
    menu::{DropdownMenu, PopupMenuItem},
};
use std::collections::BTreeSet;
use theme::controls::setting_context_colors;
use view::{badge, heading, secondary};

#[derive(Clone, Copy)]
enum Picker {
    Category,
    Context,
}

impl Picker {
    fn value(self, filters: &mut Filters) -> &mut Option<String> {
        match self {
            Self::Category => &mut filters.category,
            Self::Context => &mut filters.context,
        }
    }
}

impl Panel {
    fn picker(
        &self,
        picker: Picker,
        choices: Vec<(String, String)>,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let (id, all, selected) = match picker {
            Picker::Category => ("setting-category", "All categories", &self.filters.category),
            Picker::Context => ("setting-context", "All contexts", &self.filters.context),
        };
        let value = selected.clone();
        let selected_label = choices
            .iter()
            .find(|(k, _)| Some(k) == selected.as_ref())
            .map(|(_, label)| label.as_str())
            .or(selected.as_deref())
            .unwrap_or(all)
            .to_owned();
        let revision = self.filter_revision;
        let owner = cx.weak_entity();
        compact_button(Button::new(id))
            .label(selected_label)
            .dropdown_menu(move |mut menu, _, _| {
                menu = menu.scrollable(true).max_h(px(300.));
                for (key, label) in std::iter::once((None, all.to_owned()))
                    .chain(choices.iter().map(|(k, v)| (Some(k.clone()), v.clone())))
                {
                    let owner = owner.clone();
                    menu = menu.item(PopupMenuItem::new(label).checked(key == value).on_click(
                        move |_, _, cx| {
                            let _ = owner.update(cx, |p, cx| {
                                if p.current(cx) && p.filter_revision == revision {
                                    *picker.value(&mut p.filters) = key.clone();
                                    p.filter_changed(cx);
                                }
                            });
                        },
                    ));
                }
                menu
            })
    }

    fn filter_bar(&self, rows: &[Setting], cx: &Context<Self>) -> Div {
        let categories = rows
            .iter()
            .map(|s| &s.category)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .map(|c| (c.clone(), c.clone()))
            .collect();
        let contexts = [
            ("user", "user (SET)"),
            ("superuser", "superuser"),
            ("sighup", "sighup (reload)"),
            ("postmaster", "postmaster (restart)"),
            ("internal", "internal"),
        ]
        .map(|(k, v)| (k.into(), v.into()))
        .to_vec();
        div()
            .flex()
            .flex_shrink_0()
            .flex_wrap()
            .items_center()
            .gap_2()
            .px_4()
            .py_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(self.search_input(224.))
            .child(self.picker(Picker::Category, categories, cx))
            .child(self.picker(Picker::Context, contexts, cx))
            .when(self.filters.applied(), |d| {
                d.child(
                    compact_button(Button::new("clear-setting-filters"))
                        .ghost()
                        .label("Clear filters")
                        .on_click(cx.listener(|p, _, w, cx| {
                            if p.current(cx) {
                                p.filters = Filters::default();
                                p.search.update(cx, |s, cx| s.set_value("", w, cx));
                                p.filter_changed(cx);
                            }
                        })),
                )
            })
    }

    pub(super) fn index_settings(&mut self) {
        self.filtered_settings.clear();
        if let Some(Rows::Settings(rows)) = &self.rows {
            self.filtered_settings
                .extend((0..rows.len()).filter(|&i| self.filters.setting(&rows[i])));
            self.filtered_settings
                .sort_by(|&a, &b| rows[a].category.cmp(&rows[b].category));
        }
        self.settings_list
            .reset_with_uniform_height(self.filtered_settings.len(), px(80.));
    }

    pub(super) fn settings(&self, cx: &Context<Self>) -> Div {
        let rows = match &self.rows {
            Some(Rows::Settings(r)) => r.as_slice(),
            _ => &[],
        };
        let count = self.filtered_settings.len();
        let mut panel = div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                self.header(IconName::Settings, "PostgreSQL Settings", cx)
                    .child(secondary(
                        "settings-count",
                        format!("{count}/{}", rows.len()),
                        cx,
                    ))
                    .child(self.refresh_button(cx)),
            )
            .child(self.filter_bar(rows, cx))
            .children(self.status(Lane::Main, &self.main, cx));
        if count == 0 {
            if self.main.ready() {
                panel = panel.child(
                    Label::new(if self.filters.applied() {
                        "No matching settings"
                    } else {
                        "No settings loaded"
                    })
                    .text_color(cx.theme().muted_foreground),
                );
            }
            return panel;
        }
        let top = self
            .settings_list
            .logical_scroll_top()
            .item_ix
            .min(count - 1);
        let category = &rows[self.filtered_settings[top]].category;
        let owner = cx.entity();
        let rows_view = list(self.settings_list.clone(), move |index, _, cx| {
            let p = owner.read(cx);
            let Some(Rows::Settings(rows)) = &p.rows else {
                return div().into_any_element();
            };
            let Some(&row) = p.filtered_settings.get(index) else {
                return div().into_any_element();
            };
            let setting = &rows[row];
            let starts_category =
                index == 0 || rows[p.filtered_settings[index - 1]].category != setting.category;
            div()
                .id(("setting-item", row))
                .when(starts_category, |d| {
                    if index == top {
                        d.child(div().h(px(28.)))
                    } else {
                        d.child(category_label(&setting.category, cx))
                    }
                })
                .child(setting_row(setting, cx))
                .into_any_element()
        });
        panel.child(
            div()
                .id("setting-groups")
                .test_support()
                .relative()
                .flex_1()
                .min_h_0()
                .child(rows_view.size_full())
                .child(
                    category_label(category, cx)
                        .id("sticky-setting-category")
                        .test_support()
                        .absolute()
                        .top_0()
                        .left_0()
                        .right_0(),
                ),
        )
    }
}

fn category_label(category: &str, cx: &App) -> Div {
    heading("category-heading", category, cx)
        .h(px(28.))
        .bg(cx.theme().muted)
        .px_4()
        .py_1()
}

fn setting_row(s: &Setting, cx: &App) -> Stateful<Div> {
    let (color, background) = setting_context_colors(&s.context, cx);
    let title = div()
        .flex()
        .items_center()
        .flex_wrap()
        .gap_2()
        .child(Label::new(s.name.clone()).font_weight(FontWeight::MEDIUM))
        .child(badge("setting-context-value", s.context.clone(), color).bg(background))
        .when(s.modified(), |d| {
            d.child(badge("setting-modified", "modified", cx.theme().primary))
        })
        .child(secondary("setting-source", format!("Source: {}", s.source), cx).text_size(px(9.)));
    let value = |v: &str| {
        if s.unit.is_empty() {
            v.to_owned()
        } else {
            format!("{v} {}", s.unit)
        }
    };
    div()
        .id(SharedString::from(s.name.clone()))
        .px_4()
        .py_2()
        .border_b_1()
        .border_color(cx.theme().border.opacity(0.3))
        .when(s.modified(), |d| d.bg(cx.theme().primary.opacity(0.02)))
        .child(title)
        .child(
            div()
                .flex()
                .flex_wrap()
                .gap_3()
                .child(
                    Label::new(value(&s.value))
                        .text_size(px(14.))
                        .font_weight(FontWeight::BOLD),
                )
                .child(secondary(
                    "setting-default",
                    format!("Default: {}", value(&s.boot)),
                    cx,
                ))
                .child(secondary(
                    "setting-reset",
                    format!("Reset: {}", value(&s.reset)),
                    cx,
                )),
        )
        .child(secondary("setting-description", s.description.clone(), cx))
}
