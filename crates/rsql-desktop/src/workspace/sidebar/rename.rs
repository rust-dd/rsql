use super::*;
use gpui_kit::component::input::{InputEvent, InputState};
use rsql_core::storage::project;

const TITLE: &str = "Rename Server";
const NAME: &str = "Server Name";

struct RenameDialog {
    owner: WeakEntity<Workspace>,
    ids: Vec<String>,
    address: String,
    input: Entity<InputState>,
    focus: FocusHandle,
    saving: bool,
    saved: bool,
    error: Option<String>,
    _subscriptions: [Subscription; 3],
}

impl Workspace {
    pub(super) fn open_server_rename(
        &mut self,
        ids: Vec<String>,
        previous_focus: Option<FocusHandle>,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let owner = cx.weak_entity();
        // The context menu restores focus after its action. Open after that focus has settled.
        w.defer(cx, move |w, cx| {
            let _ = owner.update(cx, |view, cx| {
                view.present_server_rename(ids, previous_focus, w, cx);
            });
        });
    }

    fn present_server_rename(
        &self,
        ids: Vec<String>,
        previous_focus: Option<FocusHandle>,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if w.has_active_dialog(cx) {
            return;
        }
        let (address, alias) = {
            let connections = self.session.connections.borrow();
            let Some(first) = ids.first().and_then(|id| connections.resolve_id(id)) else {
                return;
            };
            (
                format!("{}:{}", first.host, first.port),
                server_group_alias(ids.iter().filter_map(|id| connections.resolve_id(id)))
                    .to_owned(),
            )
        };

        previous_focus.as_ref().unwrap_or(&self.focus).focus(w, cx);
        let owner = cx.entity();
        let input = cx.new(|cx| {
            InputState::new(w, cx)
                .placeholder(address.clone())
                .default_value(alias)
        });
        let dialog = cx.new(|cx| RenameDialog::new(&owner, ids, address, input.clone(), w, cx));
        w.open_dialog(cx, move |host, w, cx| {
            dialog::surface(host, TITLE, dialog::compact_width(w), cx)
                .on_cancel({
                    let dialog = dialog.clone();
                    move |_, _, cx| !dialog.read(cx).saving
                })
                // The input owns Enter, including its composition and pending-write guards.
                .on_ok(|_, _, _| false)
                .content({
                    let dialog = dialog.clone();
                    move |body, _, _| body.min_h_0().child(dialog.clone())
                })
        });
        input.update(cx, |input, cx| input.focus(w, cx));
    }
}

impl RenameDialog {
    fn new(
        owner: &Entity<Workspace>,
        ids: Vec<String>,
        address: String,
        input: Entity<InputState>,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        let dialog = cx.weak_entity();
        let enter = w.subscribe(&input, cx, move |_, event, w, cx| {
            if let InputEvent::PressEnter {
                secondary: false,
                shift: false,
            } = event
            {
                let _ = dialog.update(cx, |view, cx| view.submit(w, cx));
            }
        });
        let refocus = cx.on_focus_in(&focus, w, |view, w, cx| {
            if view.saved {
                view.close(w, cx);
            }
        });
        let busy = cx.observe(owner, |_, _, cx| cx.notify());
        Self {
            owner: owner.downgrade(),
            ids,
            address,
            input,
            focus,
            saving: false,
            saved: false,
            error: None,
            _subscriptions: [enter, refocus, busy],
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
        if self.saving
            || self.saved
            || owner.read(cx).navigation.busy
            || self
                .input
                .update(cx, |input, cx| input.marked_text_range(w, cx).is_some())
        {
            return;
        }

        self.saving = true;
        self.error = None;
        let alias = self.input.read(cx).value().trim().to_owned();
        let ids = self.ids.clone();
        let dialog = cx.weak_entity();
        let window = w.window_handle();
        owner.update(cx, |view, cx| {
            let svc = view.session.services.clone();
            let group = ids.clone();
            let name = alias.clone();
            view.run_maintenance(
                async move {
                    let db = svc.local_db.connect().map_err(|e| e.to_string())?;
                    project::set_server_alias(&db, &group, &name)
                        .await
                        .map_err(|e| e.to_string())
                },
                move |view, result, cx| {
                    if result.is_ok() {
                        for profile in &mut view.session.connections.borrow_mut().profiles {
                            if ids.contains(&profile.id) {
                                profile.server_alias.clone_from(&alias);
                            }
                        }
                    }
                    let _ = window.update(cx, |_, w, cx| {
                        let _ = dialog.update(cx, |view, cx| {
                            view.saving = false;
                            match result {
                                Ok(()) => {
                                    view.saved = true;
                                    view.close(w, cx);
                                }
                                Err(e) => {
                                    view.error = Some(format!(
                                        "Could not save the server name. Try again. {e}"
                                    ));
                                }
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

impl Render for RenameDialog {
    fn render(&mut self, w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let busy = self.saving
            || self.saved
            || self
                .owner
                .upgrade()
                .is_none_or(|owner| owner.read(cx).navigation.busy);
        let foreground = theme.foreground;
        let cancel = cx.listener(|view, _, w, cx| view.close(w, cx));

        clipboard_order::capture(
            div()
                .id("rename-server-content")
                .track_focus(&self.focus)
                .relative()
                .w_full()
                .min_h_0()
                .overflow_y_scroll()
                .text_color(foreground),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .p_6()
                .child(dialog::header(
                    "rename-server-title",
                    TITLE,
                    dialog::address_description(
                        "Name shown for ",
                        &self.address,
                        ". Leave it empty to go back to the default label.",
                        w,
                        cx,
                    ),
                    cx,
                ))
                .child(
                    dialog::Field {
                        id: "rename-server-name",
                        label_id: "rename-server-label",
                        label: NAME,
                    }
                    .render(&self.input, self.saving, foreground, w, cx),
                )
                .children(self.error.as_ref().map(|error| {
                    div()
                        .id("rename-server-error")
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
                            dialog::cancel_button("rename-server-cancel", self.saving, cx)
                                .on_click(cancel),
                        )
                        .child(
                            dialog::primary_button("rename-server-save", "Save", busy, cx)
                                .child(if self.saving { "Saving…" } else { "Save" })
                                .on_click(cx.listener(|view, _, w, cx| view.submit(w, cx))),
                        ),
                )
                .child(
                    dialog::close_button("rename-server-close", self.saving, cx)
                        .on_click(cx.listener(|view, _, w, cx| view.close(w, cx))),
                ),
        )
    }
}
