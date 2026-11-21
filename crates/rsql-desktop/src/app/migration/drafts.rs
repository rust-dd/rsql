use super::*;
use crate::workspace::theme::controls::compact_button;
use gpui_kit::component::{ActiveTheme, Disableable, button::Button, label::Label};
use gpui_kit::prelude::FluentBuilder;
use rsql_desktop::persistence;

struct DraftRecovery {
    migration: Rc<Migration>,
    session: Rc<Session>,
    reservation: uuid::Uuid,
    original_error: String,
    message: String,
    picking: bool,
}

impl Migration {
    pub(in crate::app) fn retain_failed_save(
        self: &Rc<Self>,
        id: uuid::Uuid,
        session: &Rc<Session>,
        w: &mut Window,
        cx: &mut App,
    ) -> bool {
        let Some(error) = session.save_status().borrow().clone() else {
            return false;
        };
        if self.copy_saved.get() {
            return false;
        }
        let screen = cx.new(|_| DraftRecovery {
            migration: self.clone(),
            session: session.clone(),
            reservation: id,
            original_error: error,
            message: "Choose a folder for a copy of the current workspace and editor drafts."
                .into(),
            picking: false,
        });
        w.replace_root(cx, |w, cx| gpui_kit::base::Root::new(screen, w, cx));
        true
    }
}

impl DraftRecovery {
    fn save_copy(&mut self, w: &Window, cx: &mut Context<Self>) {
        if self.picking || self.migration.copy_saved.get() {
            return;
        }
        self.picking = true;
        self.message = "Saving workspace copy…".into();
        let folder = (self.migration.picker)(w, false);
        let bytes = persistence::encode(&self.session.documents.borrow());
        let runtime = self.session.runtime.clone();
        let lease = self.migration.live_lease.borrow().upgrade();
        cx.spawn(async move |view, cx| {
            let result = if let Some(directory) = folder.await {
                Some(
                    runtime
                        .spawn_blocking(move || {
                            use rsql_core::migration::DirectoryId;
                            let lease = lease.ok_or_else(|| {
                                std::io::Error::other("Profile lease unavailable")
                            })?;
                            if DirectoryId::read(&directory)? == lease.original_directory_id() {
                                return Err(std::io::Error::other(
                                    "Choose a different folder from the active profile",
                                ));
                            }
                            persistence::save_workspace_copy(&directory, &bytes?)
                        })
                        .await
                        .map_err(|e| e.to_string())
                        .and_then(|r| r.map_err(|e| e.to_string())),
                )
            } else {
                None
            };
            let _ = view.update(cx, |view, cx| {
                view.picking = false;
                view.message = match result {
                    Some(Ok(path)) => {
                        view.migration.copy_saved.set(true);
                        view.migration.finish(view.reservation, cx);
                        format!(
                            "Workspace and editor drafts saved to {}. You can now restart or quit.",
                            path.display()
                        )
                    }
                    Some(Err(error)) => {
                        format!("Could not save workspace copy: {error}. Choose another folder.")
                    }
                    None => "Selection canceled. Your editor drafts are still here.".into(),
                };
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

impl Render for DraftRecovery {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let saved = self.migration.copy_saved.get();
        div()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .p_4()
            .flex()
            .flex_col()
            .gap_2()
            .child(Label::new(format!(
                "Unable to save the current profile. {}",
                self.original_error
            )))
            .child(Label::new(self.message.clone()))
            .when(!saved, |view| {
                view.child(
                    compact_button(Button::new("Save workspace copy").label("Save workspace copy"))
                        .disabled(self.picking)
                        .on_click(cx.listener(|view, _, w, cx| view.save_copy(w, cx))),
                )
            })
            .when(saved, |view| {
                view.child(
                    compact_button(Button::new("Restart RSQL").label("Restart RSQL"))
                        .on_click(|_, w, cx| w.dispatch_action(Box::new(Restart), cx)),
                )
            })
    }
}
