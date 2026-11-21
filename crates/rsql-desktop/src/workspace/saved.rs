use super::{theme::controls::compact_button, *};
use gpui_kit::{
    assets::IconName,
    base::{StyledExt, TestTextExt},
    component::{
        ActiveTheme, Icon,
        menu::{ContextMenuExt, ContextMenuItemStyle, PopupMenuItem},
    },
};
use rsql_desktop::{connection::Connections, saved::SavedQueryKey};

pub(super) fn connection_label(key: &SavedQueryKey, profiles: &Connections) -> String {
    // Quote stored fields so empty values and separators remain distinguishable.
    let SavedQueryKey {
        database,
        driver,
        project,
        ..
    } = key;
    key.connection(profiles)
        .map(|profile| {
            let name = profile.display_name();
            if profile.driver != *driver {
                format!("{name} · {database:?} · {driver:?}")
            } else if profile.database != *database {
                format!("{name} · {database:?}")
            } else {
                name.to_owned()
            }
        })
        .unwrap_or_else(|| format!("{database:?} · {driver:?} · Missing connection: {project:?}"))
}

enum Retry {
    Save,
    Delete,
}

impl Workspace {
    pub(super) fn open_saved_query(
        &mut self,
        key: &SavedQueryKey,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let saved = self.session.saved_queries().rows.get(key).cloned();
        let Some(saved) = saved else { return };
        self.session.apply_removals();
        let project = saved
            .key
            .connection(&self.session.connections.borrow())
            .map(|profile| profile.id.clone());
        let title = if project.is_some() {
            saved.key.title.clone()
        } else if saved.key.title.is_empty() {
            "Saved query (missing connection)".into()
        } else {
            format!("{} (missing connection)", saved.key.title)
        };
        self.update_documents(w, cx, |docs| {
            let tab = docs.open(Kind::Query, project);
            if !title.is_empty() {
                tab.title = title;
            }
            tab.main.draft.text = saved.sql.clone();
        });
    }

    fn copy_saved_query(&self, key: &SavedQueryKey, cx: &mut App) {
        let saved = self.session.saved_queries().rows.get(key).cloned();
        if let Some(saved) = saved {
            clipboard_order::ClipboardWrite::begin(cx);
            crate::desktop::DesktopServices::write_clipboard(
                cx,
                ClipboardItem::new_string(saved.sql.clone()),
            );
        }
    }

    pub(super) fn saved_sidebar(&self, cx: &Context<Self>) -> impl IntoElement {
        let library = self.session.saved_queries();
        let profiles = self.session.connections.borrow();
        let count = library.rows.len().to_string();
        let mut section = div()
            .id("saved-queries")
            .flex_shrink_0()
            .border_t_1()
            .border_color(cx.theme().sidebar_border)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .h(px(32.))
                    .px_3()
                    .text_size(px(10.))
                    .child(theme::controls::tracked("SAVED QUERIES", 10., 0.1).font_semibold())
                    .children((!library.rows.is_empty()).then(|| {
                        div()
                            .id("saved-query-count")
                            .text_color(cx.theme().muted_foreground)
                            .child(count.clone())
                            .test_text(count)
                    })),
            );
        if !library.rows.is_empty() {
            section = section.child(
                div()
                    .id("saved-query-list")
                    .max_h(px(192.))
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .p_1()
                    .children(
                        library
                            .rows
                            .keys()
                            .map(|key| self.saved_row(key, connection_label(key, &profiles), cx)),
                    ),
            );
        } else if library.loading {
            section = section.child(self.saved_message("Loading saved queries…", cx));
        } else if library.load_error.is_none() {
            section = section.child(self.saved_message(
                "No saved queries yet. Use the Save button in the toolbar to save the current query.",
                cx,
            ));
        }
        if let Some(error) = &library.load_error {
            section = section
                .child(self.saved_error(
                    "saved-query-load-error",
                    format!("Could not load saved queries: {error}"),
                    cx,
                ))
                .child(compact_button(self.button(
                    "Retry loading saved queries",
                    cx,
                    |v, _, cx| {
                        v.session.refresh_saved_queries();
                        cx.notify();
                    },
                )));
        }
        if !library.save_failures.is_empty() || !library.delete_failures.is_empty() {
            section = section.child(
                div()
                    .id("saved-query-write-failures")
                    .max_h(px(144.))
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .children(library.save_failures.iter().map(|(key, failure)| {
                        self.saved_retry(key, &profiles, &failure.error, Retry::Save, cx)
                    }))
                    .children(library.delete_failures.iter().map(|(key, error)| {
                        self.saved_retry(key, &profiles, error, Retry::Delete, cx)
                    })),
            );
        }
        section
    }

    fn saved_retry(
        &self,
        key: &SavedQueryKey,
        profiles: &Connections,
        error: &str,
        retry: Retry,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let (operation, button) = match retry {
            Retry::Save => ("save", "Retry save"),
            Retry::Delete => ("delete", "Retry delete"),
        };
        let scope = connection_label(key, profiles);
        let title = format!("Could not {operation} “{}”", key.title);
        let details = format!("{title} · {scope}\n{error}");
        let accessibility = format!("{button} {} · {scope}", key.title);
        let id = SharedString::from(format!("saved-query-{operation}-error-{}", key.encode()));
        let key = key.clone();
        div()
            .id(id)
            .test_support()
            .flex()
            .items_center()
            .gap_1()
            .flex_shrink_0()
            .px_3()
            .py_1()
            .text_size(px(11.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(div().truncate().text_color(cx.theme().danger).child(title))
                    .child(
                        div()
                            .truncate()
                            .text_color(cx.theme().muted_foreground)
                            .child(scope),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_color(cx.theme().muted_foreground)
                            .child(error.to_owned()),
                    ),
            )
            .child(
                compact_button(self.button(button, cx, move |v, _, _| match retry {
                    Retry::Save => v.session.retry_saved_save(key.clone()),
                    Retry::Delete => v.session.retry_saved_delete(key.clone()),
                }))
                .flex_shrink_0()
                .accessibility_label(accessibility)
                .tooltip(details),
            )
    }

    fn saved_message(&self, text: &'static str, cx: &App) -> impl IntoElement {
        div()
            .px_3()
            .pb_2()
            .text_size(px(11.))
            .text_color(cx.theme().muted_foreground)
            .child(text)
            .test_text(text)
    }

    fn saved_error(&self, id: &'static str, text: String, cx: &App) -> impl IntoElement {
        div()
            .id(id)
            .test_support()
            .px_3()
            .pb_1()
            .text_size(px(11.))
            .text_color(cx.theme().danger)
            .child(text.clone())
            .test_text(text)
    }

    fn saved_row(
        &self,
        key: &SavedQueryKey,
        label: String,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let open = key.clone();
        let key = key.clone();
        let view = cx.weak_entity();
        let target = format!("{} · {label}", key.title);
        let selected = self.navigation.selected_saved_query.as_ref() == Some(&key);
        let theme = cx.theme();
        let mut row = super::sidebar::row::TreeRow::new(
            0,
            key.title.clone(),
            Icon::new(IconName::FileText)
                .size(px(14.))
                .text_color(theme.primary.opacity(0.6)),
        );
        row.selected = selected;
        row.tooltip = Some(target.clone().into());
        row.paint(format!("saved-query-{}", key.encode()), cx)
            .aria_label(format!("Open {target}"))
            .child(
                div()
                    .max_w(relative(0.45))
                    .flex_shrink_0()
                    .truncate()
                    .text_size(px(10.))
                    .text_color(theme.muted_foreground)
                    .child(label.clone())
                    .test_text(label),
            )
            .on_click(cx.listener(move |v, _, w, cx| {
                v.navigation.selected_saved_query = Some(open.clone());
                v.open_saved_query(&open, w, cx);
                cx.notify();
            }))
            .context_menu(move |menu, w, cx| {
                let menu = theme::controls::context_menu(menu, w);
                let _ = view.update(cx, |v, cx| {
                    v.navigation.selected_saved_query = Some(key.clone());
                    cx.notify();
                });
                let open = key.clone();
                let copy = key.clone();
                let delete = key.clone();
                let open_view = view.clone();
                let copy_view = view.clone();
                let delete_view = view.clone();
                menu.item(
                    PopupMenuItem::new("Open in Tab")
                        .icon(IconName::FileText)
                        .on_click(move |_, w, cx| {
                            let _ = open_view.update(cx, |v, cx| v.open_saved_query(&open, w, cx));
                        }),
                )
                .item(
                    PopupMenuItem::new("Copy SQL")
                        .icon(IconName::Copy)
                        .on_click(move |_, _, cx| {
                            let _ = copy_view.update(cx, |v, cx| v.copy_saved_query(&copy, cx));
                        }),
                )
                .separator()
                .context_item(
                    PopupMenuItem::new("Delete")
                        .icon(IconName::Trash)
                        .on_click(move |_, _, cx| {
                            let _ = delete_view
                                .update(cx, |v, _| v.session.delete_query(delete.clone()));
                        }),
                    ContextMenuItemStyle {
                        destructive: true,
                        ..Default::default()
                    },
                )
            })
    }
}
