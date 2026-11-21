use super::*;
use crate::workspace::theme::controls::compact_button;
use gpui_kit::component::{Disableable, button::Button, label::Label};
use rsql_desktop::migration::{
    Backup, backup_locations, inspect_backup,
    review::{Destination, ImportReview},
};
use std::path::Path;

#[derive(Clone, Copy)]
enum Folder {
    Bundle,
    Manifest,
    Destination,
    Backup,
}

enum Preview {
    Import(ImportReview),
    Backup(Destination, Backup),
    Recover(Destination, Vec<PathBuf>),
}

impl Preview {
    fn description(&self) -> String {
        match self {
            Self::Import(review) if review.conflict => format!(
                "{}\nThis profile contains data. Replacement saves a verified backup first.",
                review.summary
            ),
            Self::Import(review) => format!("{}\nReady to import.", review.summary),
            Self::Backup(_, backup) => format!(
                "Verified backup at {}. Rollback first preserves the current profile.",
                backup.location().display()
            ),
            Self::Recover(_, locations) => {
                let mut message =
                    "Check the import journal and resume any interrupted operation.".to_owned();
                for location in locations.iter().take(5) {
                    message.push_str(&format!("\nRetained backup: {}", location.display()));
                }
                if locations.len() > 5 {
                    message.push_str(&format!(
                        "\n{} additional backups in this profile.",
                        locations.len() - 5
                    ));
                }
                message
            }
        }
    }
}

pub(super) struct Dialog {
    owner: Rc<Migration>,
    destination: PathBuf,
    input: Option<(Folder, PathBuf)>,
    preview: Option<Preview>,
    current: bool,
    message: String,
    request: u64,
    accepted: bool,
    picking: bool,
    close_when_done: bool,
}

impl Dialog {
    pub(super) fn new(owner: Rc<Migration>) -> Self {
        Self {
            destination: owner.profile.clone(),
            owner,
            input: None,
            preview: None,
            current: false,
            message: "Choose an export bundle or a retained backup folder.".into(),
            request: 0,
            accepted: false,
            picking: false,
            close_when_done: false,
        }
    }

    fn choose(&mut self, kind: Folder, w: &Window, cx: &mut Context<Self>) {
        if self.accepted || self.picking {
            return;
        }
        self.request += 1;
        self.preview.take();
        self.picking = true;
        let request = self.request;
        let response = (self.owner.picker)(w, matches!(kind, Folder::Manifest));
        cx.spawn(async move |view, cx| {
            let selected = response.await;
            let _ = view.update(cx, |d, cx| {
                if d.request != request {
                    return;
                }
                d.picking = false;
                if let Some(selected) = selected {
                    match kind {
                        Folder::Destination => d.destination = selected,
                        _ => d.input = Some((kind, selected)),
                    }
                    d.validate(cx);
                } else {
                    d.message = "Selection canceled.".into();
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn validate(&mut self, cx: &mut Context<Self>) {
        if self.accepted || self.picking {
            return;
        }
        self.request += 1;
        self.preview.take();
        let request = self.request;
        let dest = self.destination.clone();
        let active = self.owner.profile.clone();
        let input = self.input.clone();
        let lease = self.owner.live_lease.borrow().upgrade();
        self.message = "Checking profile and files…".into();
        let job = self
            .owner
            .runtime
            .spawn_blocking(move || inspect(&active, &dest, input, lease.as_deref()));
        cx.spawn(async move |view, cx| {
            let result = job
                .await
                .map_err(|_| "Review worker stopped".to_owned())
                .and_then(|response| response.map_err(|error| error.to_string()));
            let _ = view.update(cx, |d, cx| {
                if request != d.request {
                    return;
                }
                match result {
                    Ok((preview, current)) => {
                        d.message = preview.description();
                        if current {
                            d.message.push_str(" RSQL will restart after completion.");
                        }
                        d.current = current;
                        d.preview = Some(preview);
                    }
                    Err(error) => d.message = error,
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn accept(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        if self.accepted || self.owner.busy() {
            return;
        }
        let Some(preview) = self.preview.take() else {
            return;
        };
        let operation = match preview {
            Preview::Import(review) => ReviewedOperation::Import {
                replace: review.conflict,
                review,
            },
            Preview::Backup(destination, backup) => ReviewedOperation::Restore {
                destination,
                backup,
            },
            Preview::Recover(dest, _) => ReviewedOperation::Recover(dest),
        };
        self.accepted = true;
        let Some(id) = self.owner.reserve() else {
            return;
        };
        self.message = "Waiting for the profile operation to finish safely…".into();
        if self.current && self.owner.attached.get() {
            let transition = self
                .owner
                .transition
                .borrow()
                .clone()
                .expect("attached lifecycle");
            w.close_dialog(cx);
            transition(id, Some(operation), None, w, cx);
            return;
        }
        let rx = self.owner.start(operation);
        let owner = self.owner.clone();
        let current = self.current;
        let handle = w.window_handle();
        cx.spawn(async move |view, cx| {
            let mut result = receive(rx).await;
            let relaunch = result.is_ok() && current && !owner.closing();
            if relaunch {
                let command = owner.restart.clone();
                result = owner
                    .runtime
                    .spawn_blocking(move || {
                        command
                            .and_then(|c| c.start())
                            .map(|_| "Restarted RSQL.".to_owned())
                    })
                    .await
                    .map_err(|_| "Relaunch worker stopped".to_string())
                    .and_then(|r| r);
            }
            cx.update(|cx| {
                if relaunch
                    && result.is_ok()
                    && let Some(status) = &owner.startup_error
                {
                    status.take();
                }
                if !owner.finish(id, cx) {
                    return;
                }
                if current && result.is_ok() {
                    cx.quit();
                    return;
                }
                let _ = view.update(cx, |d, cx| {
                    d.accepted = false;
                    d.message = result.unwrap_or_else(|e| e);
                    if d.close_when_done {
                        cx.defer(move |cx| {
                            let _ = handle.update(cx, |_, w, cx| w.close_dialog(cx));
                        });
                    }
                    cx.notify();
                });
            });
        })
        .detach();
        cx.notify();
    }

    fn close(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        if self.accepted {
            self.close_when_done = true;
            self.message = "This dialog will close when the profile is safe.".into();
            cx.notify();
        } else {
            self.request += 1;
            w.close_dialog(cx);
        }
    }
}

fn inspect(
    active: &Path,
    dest: &Path,
    input: Option<(Folder, PathBuf)>,
    lease: Option<&rsql_core::migration::ProfileLease>,
) -> Result<(Preview, bool), rsql_desktop::migration::ImportError> {
    let destination = Destination::select(dest)?;
    let open = Destination::select(active).ok();
    let current = open.as_ref().is_some_and(|d| d.same(&destination));
    let input = match input {
        Some((Folder::Manifest, path)) => {
            if path
                .file_name()
                .is_none_or(|n| n != rsql_core::migration::MANIFEST_FILENAME)
            {
                return Err(rsql_desktop::migration::ImportError::Integrity(
                    "choose the bundle's manifest.json",
                ));
            }
            let folder = path
                .parent()
                .ok_or(rsql_desktop::migration::ImportError::Stale)?;
            Some((Folder::Bundle, folder.to_owned()))
        }
        other => other,
    };
    let preview = match input {
        Some((Folder::Bundle, path)) => {
            Preview::Import(ImportReview::inspect(&path, destination, lease)?)
        }
        Some((Folder::Backup, path)) => {
            let backup = inspect_backup(&path, &destination.path)?;
            Preview::Backup(destination, backup)
        }
        _ => {
            let locations = backup_locations(&destination.path)?;
            Preview::Recover(destination, locations)
        }
    };
    Ok((preview, current))
}

impl Render for Dialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut row = div().flex().gap_2();
        for (label, kind) in [
            ("Export bundle", Folder::Bundle),
            ("Manifest file", Folder::Manifest),
            ("Destination", Folder::Destination),
            ("Backup folder", Folder::Backup),
        ] {
            row = row.child(
                compact_button(Button::new(label).label(label))
                    .disabled(self.accepted || self.picking)
                    .on_click(cx.listener(move |d, _, w, cx| d.choose(kind, w, cx))),
            );
        }
        let accept = match &self.preview {
            Some(Preview::Import(r)) if r.conflict => "Back up and replace",
            Some(Preview::Import(_)) => "Import profile",
            Some(Preview::Backup(..)) => "Roll back profile",
            Some(Preview::Recover(..)) => "Recover profile",
            None => "Import profile",
        };
        div()
            .flex()
            .flex_col()
            .gap_2()
            .text_sm()
            .child(row)
            .child(Label::new(format!(
                "Destination: {}",
                self.destination.display()
            )))
            .children(
                self.input
                    .as_ref()
                    .map(|(_, p)| Label::new(format!("Source: {}", p.display()))),
            )
            .child(Label::new(self.message.clone()))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        compact_button(Button::new("Check recovery").label("Check recovery"))
                            .disabled(self.accepted || self.picking)
                            .on_click(cx.listener(|d, _, _, cx| {
                                d.input = None;
                                d.validate(cx);
                            })),
                    )
                    .child(
                        compact_button(Button::new("Review again").label("Review again"))
                            .disabled(self.accepted || self.picking)
                            .on_click(cx.listener(|d, _, _, cx| d.validate(cx))),
                    )
                    .child(
                        compact_button(Button::new(accept).label(accept))
                            .disabled(self.preview.is_none() || self.accepted)
                            .on_click(cx.listener(|d, _, w, cx| d.accept(w, cx))),
                    )
                    .child(
                        compact_button(Button::new("Close review").label(if self.accepted {
                            "Close when finished"
                        } else {
                            "Cancel"
                        }))
                        .on_click(cx.listener(|d, _, w, cx| d.close(w, cx))),
                    ),
            )
    }
}
