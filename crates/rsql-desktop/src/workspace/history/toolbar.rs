use super::*;
use filter::{Range, bytes};
use gpui_kit::base::TestTextExt;
use gpui_kit::component::{
    Disableable, Icon, Sizable,
    button::{Button, ButtonCustomVariant},
    checkbox::Checkbox,
    input::Input,
    menu::{DropdownMenu, PopupMenuItem},
};

#[derive(Clone, PartialEq, Eq)]
enum Choice {
    Project(Option<String>),
    Database(Option<String>),
    Status(Option<bool>),
    Time(Range),
}

impl Panel {
    fn picker(
        &self,
        id: &'static str,
        label: &'static str,
        selected: Choice,
        options: Vec<(String, Choice)>,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let value = options
            .iter()
            .find(|(_, v)| *v == selected)
            .map(|(label, _)| label.as_str())
            .unwrap_or_else(|| match &selected {
                Choice::Database(Some(value)) => value.as_str(),
                _ => label,
            })
            .to_owned();
        let panel = cx.weak_entity();
        let generation = self.generation;
        compact_button(Button::new(id))
            .max_w(px(160.))
            .child(div().truncate().child(value.clone()).test_text(value))
            .accessibility_label(label)
            .dropdown_menu(move |mut menu, _, _| {
                for (label, choice) in &options {
                    let panel = panel.clone();
                    let chosen = choice.clone();
                    menu = menu.item(
                        PopupMenuItem::new(label.clone())
                            .checked(*choice == selected)
                            .on_click(move |_, _, cx| {
                                let _ = panel.update(cx, |p, cx| {
                                    if !p.current(generation) {
                                        return;
                                    }
                                    match &chosen {
                                        Choice::Project(project) => {
                                            if p.form.project != *project {
                                                p.form.project.clone_from(project);
                                                p.form.database = None;
                                            }
                                        }
                                        Choice::Database(database) => {
                                            p.form.database.clone_from(database)
                                        }
                                        Choice::Status(success) => p.form.success = *success,
                                        Choice::Time(range) => p.form.range = *range,
                                    }
                                    p.reload(cx);
                                });
                            }),
                    );
                }
                menu
            })
    }

    fn filters(&self, cx: &Context<Self>) -> Div {
        let mut projects = self
            .session
            .connections
            .borrow()
            .profiles
            .iter()
            .map(|p| {
                (
                    p.display_name().to_owned(),
                    Choice::Project(Some(p.id.clone())),
                )
            })
            .collect::<Vec<_>>();
        projects.sort_by(|a, b| a.0.cmp(&b.0));
        projects.insert(0, ("All connections".into(), Choice::Project(None)));
        let mut databases = vec![("All databases".into(), Choice::Database(None))];
        databases.extend(
            self.databases
                .iter()
                .filter(|d| !d.is_empty())
                .map(|d| (d.clone(), Choice::Database(Some(d.clone())))),
        );
        div()
            .flex()
            .items_center()
            .flex_wrap()
            .gap_2()
            .child(
                div().min_w(px(192.)).flex_1().child(Styled::h(
                    Input::new(&self.search)
                        .id("history-search")
                        .small()
                        .prefix(
                            div().id("history-search-icon").test_support().child(
                                Icon::default()
                                    .path("icons/search.svg")
                                    .size(px(12.))
                                    .text_color(cx.theme().muted_foreground),
                            ),
                        ),
                    px(28.),
                )),
            )
            .child(self.picker(
                "history-project",
                "Connection",
                Choice::Project(self.form.project.clone()),
                projects,
                cx,
            ))
            .child(self.picker(
                "history-database",
                "Database",
                Choice::Database(self.form.database.clone()),
                databases,
                cx,
            ))
            .child(
                self.picker(
                    "history-status",
                    "Status",
                    Choice::Status(self.form.success),
                    [
                        ("Any status", None),
                        ("Succeeded", Some(true)),
                        ("Failed", Some(false)),
                    ]
                    .into_iter()
                    .map(|(l, v)| (l.into(), Choice::Status(v)))
                    .collect(),
                    cx,
                ),
            )
            .child(
                self.picker(
                    "history-time",
                    "Time",
                    Choice::Time(self.form.range),
                    [Range::Any, Range::Day, Range::Week, Range::Month]
                        .into_iter()
                        .map(|v| (v.label().into(), Choice::Time(v)))
                        .collect(),
                    cx,
                ),
            )
            .child(
                compact_button(Button::new("history-clear-filters").label("Clear filters"))
                    .ghost()
                    .on_click(cx.listener(|p, _, w, cx| {
                        p.debounce = None;
                        p.search.update(cx, |s, cx| s.set_value("", w, cx));
                        p.form = Form::default();
                        p.reload(cx);
                    })),
            )
    }

    pub(super) fn toolbar(&self, cx: &Context<Self>) -> Div {
        let log = self.session.history();
        let read_only = !self.owns_idle_pane();
        let mut bar = div()
            .flex()
            .flex_col()
            .gap_2()
            .px_4()
            .py_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(self.filters(cx));
        let mut settings = div()
            .flex()
            .items_center()
            .flex_wrap()
            .gap_x_4()
            .gap_y_1()
            .text_color(cx.theme().muted_foreground)
            .text_size(px(11.));
        if let Some(status) = &log.status {
            settings = settings.child(format!(
                "{} saved on this device ({}). Kept for {} days, up to {} queries / {}.",
                status.entries,
                bytes(status.bytes),
                status.max_age_days,
                status.max_entries,
                bytes(status.max_bytes)
            ));
        }
        settings = settings.child(
            Checkbox::new("history-recording")
                .label("Record history")
                .small()
                .checked(log.status.as_ref().is_none_or(|s| s.enabled))
                .disabled(read_only || log.next_recording_setting().is_none())
                .on_click(cx.listener(|p, _, _, _| {
                    let next = p.session.history().next_recording_setting();
                    if let Some(enabled) = next.filter(|_| p.owns_idle_pane()) {
                        p.session.set_history_enabled(enabled);
                    }
                })),
        );
        let mut actions = div().flex().items_center().gap_1().ml_auto();
        if self.confirming {
            actions = actions
                .child("Delete all saved history?")
                .child(
                    compact_button(Button::new("history-delete").label("Delete"))
                        .h(px(24.))
                        .custom(
                            ButtonCustomVariant::new(cx)
                                .foreground(cx.theme().danger)
                                .hover(cx.theme().danger.opacity(0.1))
                                .active(cx.theme().danger.opacity(0.2)),
                        )
                        .on_click(cx.listener(|p, _, _, cx| {
                            if p.confirming && p.owns_idle_pane() {
                                p.confirming = false;
                                p.session.clear_history();
                                cx.notify();
                            }
                        })),
                )
                .child(
                    compact_button(Button::new("history-cancel").label("Cancel"))
                        .ghost()
                        .h(px(24.))
                        .on_click(cx.listener(|p, _, _, cx| {
                            p.confirming = false;
                            cx.notify();
                        })),
                );
        } else {
            let label = if log.clear_error.is_some() {
                "Retry clear"
            } else {
                "Clear"
            };
            actions = actions.child(
                compact_button(Button::new("history-clear"))
                    .h(px(24.))
                    .accessibility_label(label)
                    .ghost()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(
                                div()
                                    .id("history-trash-icon")
                                    .test_support()
                                    .child(Icon::default().path("icons/trash.svg").size(px(12.))),
                            )
                            .child(label),
                    )
                    .disabled(
                        read_only
                            || (log.clear_error.is_none()
                                && log.entries.is_empty()
                                && log.status.as_ref().is_none_or(|s| s.entries == 0)),
                    )
                    .on_click(cx.listener(|p, _, _, cx| {
                        if p.owns_idle_pane() {
                            p.confirming = true;
                            cx.notify();
                        }
                    })),
            );
        }
        bar = div().child(bar.child(settings.child(actions)));
        if log.status.as_ref().is_some_and(|s| !s.enabled) {
            bar = bar.child(
                div()
                    .id("history-off")
                    .test_support()
                    .w_full()
                    .flex_shrink_0()
                    .px_4()
                    .py_1p5()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().muted.opacity(0.4))
                    .text_color(cx.theme().muted_foreground)
                    .child(
                        div()
                            .id("history-off-text")
                            .test_support()
                            .child("History recording is off. New queries are not saved."),
                    ),
            );
        }
        if let Some(error) = &self.databases_error {
            bar = bar
                .child(self.failure("Loading history databases", error, cx))
                .child(
                    compact_button(
                        Button::new("history-database-retry").label("Retry loading databases"),
                    )
                    .on_click(cx.listener(|p, _, _, cx| p.load_databases(cx))),
                );
        }
        if let Some(error) = &log.status_error {
            bar = bar
                .child(self.failure("Loading history settings", error, cx))
                .child(
                    compact_button(
                        Button::new("history-status-retry").label("Retry loading settings"),
                    )
                    .on_click(cx.listener(|p, _, _, _| p.session.refresh_history_status())),
                );
        }
        if let Some((_, error)) = &log.setting_error {
            bar = bar
                .child(self.failure("Changing history setting", error, cx))
                .child(
                    compact_button(
                        Button::new("history-setting-retry").label("Retry changing setting"),
                    )
                    .on_click(cx.listener(|p, _, _, _| p.session.retry_history_setting())),
                );
        }
        if let Some(error) = &log.clear_error {
            bar = bar.child(self.failure("Clearing history", error, cx));
        }
        let failures = log
            .entries
            .iter()
            .filter_map(|e| e.error.as_ref().map(|error| (e.sequence, error)));
        if failures.clone().next().is_some() {
            bar = bar.child(
                div()
                    .id("history-record-failures")
                    .max_h(px(84.))
                    .overflow_y_scroll()
                    .children(failures.map(|(sequence, error)| {
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(self.failure(&format!("Query {sequence}"), error, cx))
                            .child(
                                compact_button(
                                    Button::new(("history-record-retry", sequence as usize))
                                        .label("Retry save")
                                        .disabled(log.status.as_ref().is_some_and(|s| !s.enabled)),
                                )
                                .on_click(cx.listener(
                                    move |p, _, _, _| p.session.retry_history_record(sequence),
                                )),
                            )
                    })),
            );
        }
        bar
    }

    fn failure(&self, action: &str, error: &str, cx: &App) -> Div {
        div()
            .text_size(px(11.))
            .text_color(cx.theme().danger)
            .child(format!("{action}: {error}"))
    }
}
