use super::*;
use crate::{
    desktop::notifications::localized_row_count,
    workspace::{clipboard_order::ClipboardWrite, toast::Toast},
};
use rsql_desktop::results::export::{Capture, ExportJob, Format, Prepared};
use std::path::PathBuf;
use tokio::sync::{oneshot, watch};

type Selection = Result<Option<PathBuf>, String>;
pub(super) enum ExportRequest {
    Local { format: Format, copy: bool },
    RetainedCsv,
}

enum Finished {
    Saved,
    Copied(String),
    Cancelled,
}

#[derive(Clone)]
struct Guard {
    stop: watch::Receiver<bool>,
    cancel: watch::Receiver<bool>,
    route: rsql_core::services::ProjectTicket,
    clipboard: Option<ClipboardWrite>,
}
impl Guard {
    fn current(&self) -> bool {
        !*self.stop.borrow()
            && !*self.cancel.borrow()
            && self.cancel.has_changed().is_ok()
            && self.route.is_current()
            && self.clipboard.as_ref().is_none_or(ClipboardWrite::current)
    }
    async fn selection(&mut self, path: oneshot::Receiver<Selection>) -> Selection {
        tokio::pin!(path);
        loop {
            if !self.current() {
                return Ok(None);
            }
            tokio::select! {
                value = &mut path => return value.unwrap_or(Ok(None)),
                _ = self.stop.changed() => return Ok(None),
                _ = self.cancel.changed() => return Ok(None),
                _ = tokio::time::sleep(Duration::from_millis(100)) => {}
            }
        }
    }
}

async fn run(
    captured: Capture,
    format: Format,
    mut guard: Guard,
    ready: oneshot::Sender<()>,
    path: oneshot::Receiver<Selection>,
    progress: watch::Sender<usize>,
) -> Result<Finished, String> {
    let check = guard.clone();
    let prepared = tokio::task::spawn_blocking(move || {
        captured.prepare(format, check.clipboard.is_some(), &|| check.current())
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;
    if !guard.current() {
        return Ok(Finished::Cancelled);
    }
    if let Prepared::Clipboard(text) = prepared {
        return Ok(Finished::Copied(text));
    }
    if ready.send(()).is_err() {
        return Ok(Finished::Cancelled);
    }
    let Some(path) = guard.selection(path).await? else {
        return Ok(Finished::Cancelled);
    };
    let check = guard.clone();
    tokio::task::spawn_blocking(move || {
        prepared.save(&path, &|| check.current(), |n| {
            progress.send_replace(n);
        })
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;
    Ok(Finished::Saved)
}

impl Workspace {
    pub(super) fn start_export(
        &mut self,
        owner: &Owner,
        id: Uuid,
        request: ExportRequest,
        cx: &mut Context<Self>,
    ) {
        if owner.1 != Side::Main || *self.session.preflight_stop.borrow() {
            return;
        }
        let mut docs = self.session.documents.borrow_mut();
        let Some(view) = docs
            .selected
            .map(|i| &mut docs.tabs[i])
            .filter(|t| t.id == owner.0)
            .and_then(|t| t.pane_mut(owner.1).view_mut())
            .filter(|v| v.data.id == id && v.export_job.is_none())
        else {
            return;
        };
        let (capture, format, copy) = match request {
            ExportRequest::RetainedCsv => (Capture::retained(view), Format::Csv, false),
            ExportRequest::Local { format, copy } if !copy || format != Format::Xml => (
                view.snapshot(ResultAction::Export).map(Capture::Local),
                format,
                copy,
            ),
            _ => return,
        };
        let Some(capture) = capture else {
            view.export_message =
                Some("The selected export is no longer available for these rows.".into());
            cx.notify();
            return;
        };
        let packed = matches!(request, ExportRequest::RetainedCsv);
        let clipboard = copy.then(|| ClipboardWrite::begin(cx));
        let (cancel, cancelled) = watch::channel(false);
        let (progress, mut changed) = watch::channel(0);
        let job = Uuid::new_v4();
        let guard = Guard {
            stop: self.session.preflight_stop.subscribe(),
            cancel: cancelled,
            route: view.route.clone(),
            clipboard: clipboard.clone(),
        };
        let capped = view.data.capped;
        let total = view.data.total_rows;
        view.export_message = None;
        view.export_job = Some(ExportJob {
            id: job,
            cancel,
            progress: changed.clone(),
            retained: packed,
        });
        drop(docs);
        let (ready, requested) = oneshot::channel();
        let (mut selected, path) = oneshot::channel();
        let worker = self
            .session
            .runtime
            .spawn(run(capture, format, guard, ready, path, progress));
        let owner = owner.clone();
        let target = owner.clone();
        let filename = if packed {
            "results.csv"
        } else {
            match format {
                Format::Csv => "export.csv",
                Format::Json => "export.json",
                Format::Sql => "export.sql",
                Format::Markdown => "export.md",
                Format::Xml => "export.xml",
            }
        };
        cx.spawn(async move |entity, cx| {
            if requested.await.is_err() {
                return;
            }
            let prompt = entity.update(cx, |v, cx| {
                let docs = v.session.documents.borrow();
                let work = docs
                    .tabs
                    .iter()
                    .find(|t| t.id == target.0)
                    .and_then(|t| t.pane(target.1).retained_view())
                    .filter(|v| v.data.id == id)
                    .and_then(|v| v.export_job.as_ref())
                    .filter(|j| j.id == job)?;
                (!*work.cancel.borrow() && !*v.session.preflight_stop.borrow())
                    .then(|| crate::desktop::DesktopServices::save_path(cx, filename))
            });
            if let Ok(Some(prompt)) = prompt {
                tokio::select! {
                    response = prompt => {
                        let path = response;
                        let _ = selected.send(path);
                    }
                    _ = selected.closed() => {}
                }
            }
        })
        .detach();
        cx.spawn(async move |entity, cx| {
            while changed.changed().await.is_ok() {
                if entity.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        })
        .detach();
        cx.spawn(async move |v, cx| {
            let result = worker.await.map_err(|e| e.to_string()).and_then(|r| r);
            let _ = v.update(cx, |v, cx| {
                let mut docs = v.session.documents.borrow_mut();
                let Some(view) = docs
                    .tabs
                    .iter_mut()
                    .find(|t| t.id == owner.0)
                    .and_then(|t| t.pane_mut(owner.1).retained_view_mut())
                    .filter(|v| {
                        v.data.id == id && v.export_job.as_ref().is_some_and(|j| j.id == job)
                    })
                else {
                    return;
                };
                let work = view.export_job.take().unwrap();
                if *v.session.preflight_stop.borrow()
                    || clipboard.as_ref().is_some_and(|order| !order.current())
                {
                    cx.notify();
                    return;
                }
                let stopped = *work.cancel.borrow();
                let (message, toast) = match result {
                    Err(e) if packed => {
                        let confirmed = *work.progress.borrow();
                        let message = format!("{e} ({confirmed} of {total} retained rows confirmed)");
                        let title = if stopped {
                            "CSV export stopped"
                        } else {
                            "Could not export CSV"
                        };
                        let toast = Toast::error(title).description(&message);
                        (Some(message), Some(toast))
                    }
                    Err(e) => {
                        let title = if copy {
                            "Could not copy results"
                        } else {
                            "Could not export results"
                        };
                        let toast = Toast::error(title).description(&e);
                        (Some(e), Some(toast))
                    }
                    Ok(Finished::Copied(_) | Finished::Cancelled) if stopped => (
                        Some("Export stopped".into()),
                        packed.then(|| Toast::error("CSV export stopped")),
                    ),
                    Ok(Finished::Cancelled) => (None, None),
                    Ok(Finished::Copied(s)) => {
                        crate::desktop::DesktopServices::write_clipboard(
                            cx,
                            ClipboardItem::new_string(s),
                        );
                        let toast = Toast::success("Results copied");
                        (Some("Results sent to clipboard".into()), Some(toast))
                    }
                    Ok(Finished::Saved) if packed => {
                        let suffix = if capped { " (truncated result)" } else { "" };
                        let toast = Toast::success(format!(
                            "Exported {} rows",
                            localized_row_count(total)
                        ));
                        let toast = if capped {
                            toast.description(
                                "The query was truncated; this file contains the retained rows only.",
                            )
                        } else {
                            toast
                        };
                        (
                            Some(format!("Exported {total} retained rows{suffix}")),
                            Some(toast),
                        )
                    }
                    Ok(Finished::Saved) => (
                        Some("Results exported".into()),
                        Some(Toast::success("Results exported")),
                    ),
                };
                // A finished export is reported by its toast alone, as in the legacy app.
                view.export_message = if toast.is_some() { None } else { message };
                if let Some(toast) = toast {
                    v.toasts.show(toast, cx);
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}
