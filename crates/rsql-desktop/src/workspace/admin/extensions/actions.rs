use super::*;
use gpui_kit::component::{
    button::ButtonCustomVariant,
    dialog::{Cancel, Confirm, DialogFooter},
};
use rsql_core::services::{WriteOutcome, object_action::ActionOutcome};

fn footer(
    action: ExtensionAction,
    label: &'static str,
    anchor: &FocusHandle,
    cx: &App,
) -> DialogFooter {
    let cancel = anchor.clone();
    let confirm = anchor.clone();
    let affirmative = Button::new("ok")
        .small()
        .h(px(36.))
        .px_3()
        .label(label)
        .on_click(move |_, w, cx| {
            confirm.dispatch_action(&Confirm { secondary: false }, w, cx);
        });
    let confirm = if action == ExtensionAction::Drop {
        div().child(
            affirmative
                .custom(
                    ButtonCustomVariant::new(cx)
                        .color(cx.theme().danger.opacity(0.5))
                        .foreground(cx.theme().danger)
                        .hover(cx.theme().danger.opacity(0.1)),
                )
                .outline(),
        )
    } else {
        div()
            .rounded(cx.theme().radius)
            .bg(theme::controls::accent_gradient())
            .child(
                affirmative.custom(
                    ButtonCustomVariant::new(cx)
                        .foreground(white())
                        .hover(white().opacity(0.1))
                        .active(black().opacity(0.1)),
                ),
            )
    };
    DialogFooter::new()
        .child(div().absolute().size_0().track_focus(anchor))
        .child(
            Button::new("cancel")
                .small()
                .h(px(36.))
                .px_3()
                .ghost()
                .label("Cancel")
                .on_click(move |_, w, cx| cancel.dispatch_action(&Cancel, w, cx)),
        )
        .child(confirm)
}

impl Panel {
    pub(super) fn choose_extension(
        &mut self,
        action: ExtensionAction,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ext = &self.extensions;
        let eligible = match action {
            ExtensionAction::Install => {
                ext.available_section.ready() && ext.available.iter().any(|e| e.name == name)
            }
            ExtensionAction::Drop => {
                self.main.ready() && ext.installed.iter().any(|e| e.name == name)
            }
            ExtensionAction::Update => {
                self.main.ready()
                    && ext
                        .installed
                        .iter()
                        .any(|e| e.name == name && e.update_version().is_some())
            }
        };
        if !eligible || !self.current(cx) || ext.running.is_some() || ext.review.is_some() {
            return;
        }
        let id = Uuid::new_v4();
        let statement = action.review(&self.capture.route, name);
        let preview = statement.sql().to_owned();
        self.extensions.review = Some(Review {
            id,
            action,
            statement,
            name: name.into(),
        });
        if action == ExtensionAction::Update {
            self.submit_extension(id, cx);
            return;
        }
        let (title, description, label) = if action == ExtensionAction::Drop {
            (
                "Drop Extension",
                format!(
                    "Are you sure you want to drop {name}? This will also drop all objects that depend on it (CASCADE)."
                ),
                "Drop Extension",
            )
        } else {
            (
                "Install Extension",
                format!("Install {name} into the current database?"),
                "Install",
            )
        };
        let panel = cx.weak_entity();
        let anchor = cx.focus_handle();
        window.open_alert_dialog(cx, move |dialog, _, cx| {
            let confirm = panel.clone();
            let close = panel.clone();
            dialog
                .confirm()
                .centered()
                .width(px(400.))
                .title(Label::new(title).font_weight(FontWeight::SEMIBOLD))
                .description(
                    Label::new(description.clone())
                        .id("extension-description")
                        .text_color(cx.theme().muted_foreground),
                )
                .child(
                    div()
                        .rounded_md()
                        .px_3()
                        .py_2()
                        .bg(cx.theme().muted.opacity(0.5))
                        .child(
                            Label::new(preview.clone())
                                .id("extension-preview")
                                .text_size(px(12.))
                                .text_color(cx.theme().muted_foreground),
                        ),
                )
                .footer(footer(action, label, &anchor, cx))
                .on_ok(move |_, _, cx| {
                    let _ = confirm.update(cx, |p, cx| p.submit_extension(id, cx));
                    true
                })
                .on_close(move |_, _, cx| {
                    let _ = close.update(cx, |p, cx| {
                        if p.extensions.review.as_ref().is_some_and(|r| r.id == id) {
                            p.extensions.review = None;
                            cx.notify();
                        }
                    });
                })
        });
        cx.notify();
    }

    fn submit_extension(&mut self, id: Uuid, cx: &mut Context<Self>) {
        if !self.current(cx)
            || self.extensions.running.is_some()
            || !self.extensions.review.as_ref().is_some_and(|r| r.id == id)
        {
            return;
        }
        let review = self.extensions.review.take().unwrap();
        let operation = match review.action {
            ExtensionAction::Install => "Install",
            ExtensionAction::Drop => "Drop",
            ExtensionAction::Update => "Update",
        };
        let label = format!("{operation} {}", review.name);
        let execution = Uuid::new_v4();
        let (cancel, mut cancelled) = watch::channel(());
        self.extensions.running = Some((execution, cancel));
        self.extensions.receipt = Some((format!("Running {label}…"), false));
        let capture = self.capture.clone();
        let mut generation = capture.generation.clone();
        let mut stop = self.session.preflight_stop.subscribe();
        let services = self.session.services.clone();
        let job = self.session.runtime.spawn(async move {
            let closed = async {
                tokio::select! {
                    _ = cancelled.changed() => (),
                    _ = generation.changed() => (),
                    _ = stop.wait_for(|v| *v) => (),
                }
            };
            services
                .run_extension_action(&execution.to_string(), review.statement, closed)
                .await
        });
        let owner = self.owner.clone();

        cx.spawn(async move |_, cx| {
            let outcome = job.await.unwrap_or_else(|e| ActionOutcome {
                result: WriteOutcome::Indeterminate(Error::QueryFailed(e.to_string())),
                server_warnings: None,
                verification_warning: None,
                possible_effects: false,
            });
            let _ = owner.update(cx, |v, cx| {
                let current = capture.route.is_current()
                    && v.navigation
                        .connected_routes
                        .get(&capture.project)
                        .is_some_and(|r| r.same_generation(&capture.route));
                let refresh = current && outcome.should_refresh();
                if refresh {
                    v.refresh_catalog(&capture.project, cx);
                }
                if v.admin_panels
                    .remove(&PanelKey::Finishing(execution))
                    .is_some()
                    || !current
                {
                    return;
                }
                let Some(panel) = v
                    .admin_panels
                    .values()
                    .find(|panel| {
                        panel
                            .read(cx)
                            .extensions
                            .running
                            .as_ref()
                            .is_some_and(|(id, _)| *id == execution)
                    })
                    .cloned()
                else {
                    return;
                };
                let fresh = catalog::Capture::new(v, &capture.project);
                panel.update(cx, |p, cx| {
                    p.extensions.running = None;
                    p.extensions.receipt = Some(receipt(&label, outcome));
                    if refresh && let Some(fresh) = fresh {
                        p.capture = fresh;
                        for &lane in p.lanes() {
                            if p.visible {
                                p.request(lane, cx);
                            } else {
                                *p.section(lane) = Section::default();
                            }
                        }
                    }
                    cx.notify();
                });
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

fn receipt(label: &str, outcome: ActionOutcome) -> (String, bool) {
    let (mut text, mut attention) = match outcome.result {
        WriteOutcome::Committed { cleanup_error, .. } => {
            let mut text = format!("{label} committed.");
            if let Some(error) = &cleanup_error {
                text.push_str(&format!(
                    " Session cleanup: {}",
                    diagnostic::bounded_message(error)
                ));
            }
            (text, cleanup_error.is_some())
        }
        WriteOutcome::Indeterminate(error) => (
            format!(
                "{label} completion is uncertain. Check the database before retrying. {}",
                diagnostic::bounded_message(error)
            ),
            true,
        ),
        WriteOutcome::Failed(error) => (
            format!("{label} failed. {}", diagnostic::bounded_message(error)),
            true,
        ),
    };
    if outcome.possible_effects {
        text.push_str(" Some changes may have taken effect.");
        attention = true;
    }
    for warning in [outcome.server_warnings, outcome.verification_warning]
        .into_iter()
        .flatten()
    {
        text.push(' ');
        text.push_str(&diagnostic::bounded_message(warning));
        attention = true;
    }
    (text, attention)
}
