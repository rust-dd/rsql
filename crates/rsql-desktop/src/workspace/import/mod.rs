mod dialog;
mod mapping;
mod model;
mod preview;
mod write;

use super::diagnostic::bounded_message;
use super::*;
use model::Preview;
use rsql_core::{
    postgres::roles_schema_objects::{
        CsvRows, ImportTable, import_bound_csv, parse_csv_preview_checked,
    },
    services::{ProjectTicket, WriteOutcome, run_guarded_write},
};
use std::{
    path::{Path, PathBuf},
    pin::Pin,
    sync::Arc,
};
use tokio::sync::watch;
use uuid::Uuid;

pub(super) type Selection = Pin<Box<dyn Future<Output = Option<PathBuf>>>>;
pub(super) type Picker = Rc<dyn Fn(&Window, &[&str]) -> Selection>;
pub(super) struct Dialog {
    session: Rc<Session>,
    route: ProjectTicket,
    project: String,
    schema: String,
    name: String,
    target: Option<Arc<ImportTable>>,
    picker: Picker,
    pending_picker: Option<Task<()>>,
    path: Option<PathBuf>,
    preview: Option<Preview>,
    body_scroll: ScrollHandle,
    table_scroll: ScrollHandle,
    table_width: Pixels,
    mapping_scroll: UniformListScrollHandle,
    destinations: mapping::Destinations,
    initial_focus: FocusHandle,
    picker_focus: FocusHandle,
    picking: bool,
    request: Uuid,
    cancel: watch::Sender<bool>,
    phase: Phase,
    message: String,
}

#[derive(PartialEq)]
enum Phase {
    Loading,
    Invalidated,
    Reading,
    Ready,
    Importing,
    Finished { committed: bool },
}

impl Workspace {
    pub(super) fn retire_import_read(&self, project: &str, cx: &mut App) {
        if let Some(dialog) = self.import_dialog.as_ref().and_then(WeakEntity::upgrade) {
            dialog.update(cx, |d, cx| {
                if d.project == project && d.phase == Phase::Loading {
                    d.phase = Phase::Invalidated;
                    d.message = "Catalog refreshed. Reopen Import CSV.".into();
                    cx.notify();
                }
            });
        }
    }

    pub(super) fn importing(&self) -> bool {
        self.import_dialog
            .as_ref()
            .and_then(WeakEntity::upgrade)
            .is_some()
    }
    pub(super) fn open_import(
        &mut self,
        capture: &catalog::Capture,
        object: &rsql_desktop::metadata::ObjectRef,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (project, schema, name) = (
            object.project.as_str(),
            object.schema.as_str(),
            object.name.as_str(),
        );
        let oid = object.oid;
        if self.importing() || !capture.current(self) || capture.project != object.project {
            return;
        }
        let route = capture.route.clone();
        let session = self.session.clone();
        let picker = self.import_picker.clone();
        let dialog = cx.new(|cx| Dialog {
            session: session.clone(),
            route: route.clone(),
            project: project.into(),
            schema: schema.into(),
            name: name.into(),
            target: None,
            picker,
            pending_picker: None,
            path: None,
            preview: None,
            body_scroll: ScrollHandle::new(),
            table_scroll: ScrollHandle::new(),
            table_width: px(0.),
            mapping_scroll: UniformListScrollHandle::new(),
            destinations: Default::default(),
            initial_focus: cx.focus_handle(),
            picker_focus: cx.focus_handle(),
            picking: false,
            request: Uuid::new_v4(),
            cancel: watch::channel(false).0,
            phase: Phase::Loading,
            message: "Loading table columns…".into(),
        });
        self.import_dialog = Some(dialog.downgrade());
        let mut closed = dialog.read(cx).cancel.subscribe();
        let mut stop = session.preflight_stop.subscribe();
        let services = session.services.clone();
        let (project, schema, name) = (project.to_owned(), schema.to_owned(), name.to_owned());
        let read_route = route.clone();
        let mut generation = capture.generation.clone();
        let captured = capture.clone();
        let owner = cx.weak_entity();
        let job = session.runtime.spawn(async move {
            services
                .run_metadata(
                    &project,
                    async {
                        tokio::select! {
                            _ = closed.changed() => (),
                            _ = stop.wait_for(|s| *s) => (),
                            _ = generation.changed() => (),
                        }
                    },
                    async |client| {
                        if !read_route.is_current() {
                            return Err(rsql_core::Error::QueryFailed("Connection changed".into()));
                        }
                        let target = ImportTable::load(client, &schema, &name).await?;
                        if target.relation_oid() != oid {
                            return Err(rsql_core::Error::QueryFailed(
                                "Object identity changed; refresh the catalog".into(),
                            ));
                        }
                        Ok(target)
                    },
                )
                .await
                .map_err(bounded_message)
        });
        dialog.update(cx, |_, cx| {
            cx.spawn_in(w, async move |view, cx| {
                let result = job
                    .await
                    .unwrap_or_else(|_| Err("Table metadata worker stopped.".into()));
                let _ = view.update_in(cx, |d, w, cx| {
                    if !d.current()
                        || !owner
                            .read_with(cx, |v, _| captured.current(v))
                            .unwrap_or(false)
                    {
                        return;
                    }
                    match result {
                        Ok(t) => {
                            d.destinations = mapping::Destinations::new(&t);
                            d.target = Some(Arc::new(t));
                            if d.initial_focus.is_focused(w) {
                                d.picker_focus.focus(w, cx);
                            }
                            d.phase = Phase::Ready;
                            d.message.clear();
                        }
                        Err(e) => {
                            d.phase = Phase::Invalidated;
                            d.message = e;
                        }
                    }
                    cx.notify()
                });
            })
            .detach()
        });
        w.defer(cx, move |w, cx| {
            if dialog.read(cx).current() {
                Dialog::present(dialog, w, cx);
            }
        });
    }
}

impl Dialog {
    fn current(&self) -> bool {
        self.route.is_current() && !*self.session.preflight_stop.borrow()
    }
    fn can_choose(&self) -> bool {
        self.target.is_some()
            && !self.picking
            && matches!(self.phase, Phase::Ready | Phase::Reading)
            && self.current()
    }
    fn choose(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        if !self.can_choose() {
            return;
        }
        let response = (self.picker)(w, &["csv", "tsv", "txt"]);
        self.picking = true;
        self.pending_picker = Some(cx.spawn(async move |view, cx| {
            let path = response.await;
            let _ = view.update(cx, |d, cx| {
                d.picking = false;
                if !d.current() {
                    d.message = "Connection closed.".into();
                    cx.notify();
                    return;
                }
                if let Some(path) = path {
                    d.select(path, cx)
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
    fn select(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.cancel = watch::channel(false).0;
        self.path = Some(path.clone());
        self.table_scroll = ScrollHandle::new();
        self.mapping_scroll = UniformListScrollHandle::new();
        self.preview = None;
        self.message = "Reading preview…".into();
        self.phase = Phase::Reading;
        self.request = Uuid::new_v4();
        let id = self.request;
        let cancel = self.cancel.subscribe();
        let stop = self.session.preflight_stop.subscribe();
        let route = self.route.clone();
        let target = self.target.clone().unwrap();
        let job = self.session.runtime.spawn_blocking(move || {
            let current = || cancel.has_changed().is_ok() && !*stop.borrow() && route.is_current();
            let (headers, rows) =
                parse_csv_preview_checked(&path, 5, current).map_err(bounded_message)?;
            Preview::new(
                headers,
                rows,
                target.columns.iter().map(|c| c.name.as_str()),
                current,
            )
        });
        cx.spawn(async move |d, cx| {
            let result = job
                .await
                .unwrap_or_else(|_| Err("CSV preview worker stopped.".into()));
            let _ = d.update(cx, |d, cx| {
                if d.request != id || !d.current() {
                    return;
                }
                d.phase = Phase::Ready;
                match result {
                    Ok(preview) => {
                        d.preview = Some(preview);
                        d.message.clear();
                    }
                    Err(e) => {
                        d.message = e;
                    }
                }
                cx.notify()
            });
        })
        .detach();
    }

    fn can_import(&self) -> bool {
        !self.picking
            && self.phase == Phase::Ready
            && self.current()
            && self.preview.as_ref().is_some_and(|p| p.problem().is_none())
    }
    fn accept(&mut self, cx: &mut Context<Self>) {
        if !self.can_import() {
            return;
        }
        self.phase = Phase::Importing;
        self.message = "Importing…".into();
        let preview = self.preview.as_ref().unwrap();
        let batch = write::Batch {
            services: self.session.services.clone(),
            project: self.project.clone(),
            route: self.route.clone(),
            table: self.target.clone().unwrap(),
            path: self.path.clone().unwrap(),
            headers: preview.headers.clone(),
            selection: preview.selection.clone(),
        };
        let job = self.session.runtime.spawn(batch.run(
            self.session.preflight_stop.subscribe(),
            self.cancel.subscribe(),
        ));
        cx.spawn(async move |d, cx| {
            let result = job.await;
            let _ = d.update(cx, |d, cx| {
                (d.phase, d.message) = result.unwrap_or_else(|_| {
                    (
                        Phase::Finished { committed: false },
                        "Import worker stopped. Check the table before importing again.".into(),
                    )
                });
                cx.notify()
            });
        })
        .detach();
        cx.notify();
    }
}
