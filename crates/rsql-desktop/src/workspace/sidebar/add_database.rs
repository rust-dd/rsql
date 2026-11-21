use super::*;
use gpui_kit::component::input::{InputEvent, InputState};
use rsql_core::storage::project;

const TITLE: &str = "Add Database";
const DATABASE: dialog::Field = dialog::Field {
    id: "add-database-name",
    label_id: "add-database-name-label",
    label: "Database Name",
};
const CONNECTION: dialog::Field = dialog::Field {
    id: "add-connection-name",
    label_id: "add-connection-name-label",
    label: "Connection Name",
};

struct AddDatabase {
    owner: WeakEntity<Workspace>,
    source: ConnectionProfile,
    draft_id: Option<String>,
    database: Entity<InputState>,
    name: Entity<InputState>,
    focus: FocusHandle,
    saving: bool,
    saved: bool,
    error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl Workspace {
    pub(super) fn open_add_database(
        &mut self,
        source: ConnectionProfile,
        previous_focus: Option<FocusHandle>,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let owner = cx.weak_entity();
        // Let the context menu finish restoring focus before the input takes it.
        w.defer(cx, move |w, cx| {
            let Some(owner) = owner.upgrade() else { return };
            if w.has_active_dialog(cx) {
                return;
            }
            previous_focus
                .unwrap_or_else(|| owner.read(cx).focus.clone())
                .focus(w, cx);
            let dialog = cx.new(|cx| AddDatabase::new(&owner, source, w, cx));
            let input = dialog.read(cx).database.clone();
            w.open_dialog(cx, move |host, w, cx| {
                dialog::surface(host, TITLE, dialog::compact_width(w), cx)
                    .on_cancel({
                        let dialog = dialog.clone();
                        move |_, _, cx| !dialog.read(cx).saving
                    })
                    .on_ok(|_, _, _| false)
                    .content({
                        let dialog = dialog.clone();
                        move |body, _, _| body.min_h_0().child(dialog.clone())
                    })
            });
            input.update(cx, |input, cx| input.focus(w, cx));
        });
    }
}

impl AddDatabase {
    fn new(
        owner: &Entity<Workspace>,
        source: ConnectionProfile,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let database = cx.new(|cx| InputState::new(w, cx).placeholder("analytics_db"));
        let name = cx.new(|cx| InputState::new(w, cx).placeholder("optional"));
        let focus = cx.focus_handle();
        let mut subscriptions = vec![
            cx.observe(owner, |_, _, cx| cx.notify()),
            cx.on_focus_in(&focus, w, |view: &mut Self, w, cx| {
                if view.saved {
                    view.close(w, cx);
                }
            }),
        ];
        for field in [&database, &name] {
            subscriptions.push(cx.subscribe_in(field, w, |view, input, event, w, cx| {
                match event {
                    InputEvent::Change if input == &view.database => {
                        let value = input.read(cx).value();
                        let placeholder = if value.is_empty() {
                            "optional".into()
                        } else {
                            value
                        };
                        view.name
                            .update(cx, |name, cx| name.set_placeholder(placeholder, w, cx));
                    }
                    InputEvent::PressEnter {
                        secondary: false,
                        shift: false,
                    } => view.submit(w, cx),
                    _ => {}
                }
                cx.notify();
            }));
        }
        Self {
            owner: owner.downgrade(),
            source,
            draft_id: None,
            database,
            name,
            focus,
            saving: false,
            saved: false,
            error: None,
            _subscriptions: subscriptions,
        }
    }

    fn close(&self, w: &mut Window, cx: &mut Context<Self>) {
        if !self.saving && self.focus.contains_focused(w, cx) {
            w.close_dialog(cx);
        }
    }

    fn submit(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        let Some(owner) = self.owner.upgrade() else {
            return;
        };
        let database = self.database.read(cx).value().trim().to_owned();
        if database.is_empty()
            || self.saving
            || self.saved
            || owner.read(cx).navigation.busy
            || [&self.database, &self.name]
                .iter()
                .any(|field| field.update(cx, |state, cx| state.marked_text_range(w, cx).is_some()))
        {
            return;
        }

        let name = self.name.read(cx).value();
        let profile = ConnectionProfile {
            id: self
                .draft_id
                .get_or_insert_with(|| uuid::Uuid::new_v4().to_string())
                .clone(),
            name: if name.trim().is_empty() {
                database.clone()
            } else {
                name.trim().into()
            },
            database,
            ..self.source.clone()
        };
        self.saving = true;
        self.error = None;
        let dialog = cx.weak_entity();
        let window = w.window_handle();
        let source = self.source.clone();
        owner.update(cx, |workspace, cx| {
            let services = workspace.session.services.clone();
            let draft = profile.clone();
            workspace.run_maintenance(
                async move { save(&services, &source, &draft).await },
                move |workspace, result, cx| {
                    if result.is_ok() {
                        workspace.session.connections.borrow_mut().put(profile);
                    }
                    let _ = window.update(cx, |_, w, cx| {
                        let _ = dialog.update(cx, |view, cx| {
                            view.saving = false;
                            match result {
                                Ok(()) => {
                                    view.saved = true;
                                    view.close(w, cx);
                                }
                                Err(error) => view.error = Some(error),
                            }
                            cx.notify();
                        });
                    });
                },
                cx,
            );
        });
        cx.notify();
    }
}

// Runs inside the session's profile write queue, after any earlier edit or deletion.
async fn save(
    services: &rsql_core::ApplicationServices,
    source: &ConnectionProfile,
    draft: &ConnectionProfile,
) -> Result<(), String> {
    let storage_error = |_| "Could not save the connection. Try again.".to_owned();
    let db = services.local_db.connect().map_err(storage_error)?;
    let stored = project::select_project(&db, &source.id)
        .await
        .map_err(storage_error)?
        .ok_or("The source connection was removed. Choose a saved server and try again.")?;
    let current = ConnectionProfile::from_row(source.id.clone(), &stored);
    if !source.same_route(&current) || source.server_alias != current.server_alias {
        return Err(
            "The source connection changed. Reopen Add Database to use its current settings."
                .into(),
        );
    }
    project::upsert_project(&db, &draft.id, &draft.details())
        .await
        .map_err(storage_error)
}

impl Render for AddDatabase {
    fn render(&mut self, w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let disabled = self.database.read(cx).value().trim().is_empty()
            || self.saving
            || self.saved
            || self
                .owner
                .upgrade()
                .is_none_or(|owner| owner.read(cx).navigation.busy);
        let address = format!("{}:{}", self.source.host, self.source.port);
        clipboard_order::capture(
            div()
                .id("add-database-content")
                .track_focus(&self.focus)
                .relative()
                .w_full()
                .min_h_0()
                .overflow_y_scroll()
                .text_color(theme.foreground),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .p_6()
                .child(dialog::header(
                    "add-database-title",
                    TITLE,
                    dialog::address_description("Add a database to ", &address, "", w, cx),
                    cx,
                ))
                .child(DATABASE.render(&self.database, self.saving, theme.foreground, w, cx))
                .child(div().mt_3().child(CONNECTION.render(
                    &self.name,
                    self.saving,
                    theme.muted_foreground,
                    w,
                    cx,
                )))
                .children(self.error.as_ref().map(|error| {
                    div()
                        .id("add-database-error")
                        .role(Role::Alert)
                        .aria_label(error.clone())
                        .mt_3()
                        .text_size(px(12.))
                        .line_height(px(16.))
                        .text_color(theme.danger)
                        .child(error.clone())
                }))
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap_2()
                        .mt(px(20.))
                        .child(
                            dialog::cancel_button("add-database-cancel", self.saving, cx)
                                .on_click(cx.listener(|view, _, w, cx| view.close(w, cx))),
                        )
                        .child(
                            dialog::primary_button("add-database-submit", "Add", disabled, cx)
                                .child("Add")
                                .on_click(cx.listener(|view, _, w, cx| view.submit(w, cx))),
                        ),
                )
                .child(
                    dialog::close_button("add-database-close", self.saving, cx)
                        .on_click(cx.listener(|view, _, w, cx| view.close(w, cx))),
                ),
        )
    }
}
