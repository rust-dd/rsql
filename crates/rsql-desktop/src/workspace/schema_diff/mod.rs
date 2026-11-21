use super::*;
use model::{Comparison, Filter, Status};
use std::sync::Arc;
use tokio::sync::{oneshot, watch};
use uuid::Uuid;

mod definition;
mod detail;
mod model;
mod read;
mod view;

struct Pending {
    id: Uuid,
    _cancel: watch::Sender<()>,
}

pub(super) struct Panel {
    tab: String,
    owner: WeakEntity<Workspace>,
    capture: catalog::Capture,
    schemas: [String; 2],
    picker: Option<usize>,
    pending: Option<Pending>,
    error: Option<String>,
    result: Option<Arc<Comparison>>,
    filter: Filter,
    selected: Option<detail::Detail>,
    _observe: Subscription,
}

impl Workspace {
    pub(super) fn open_schema_diff(
        &mut self,
        capture: &catalog::Capture,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if capture.current(self) {
            self.update_documents(w, cx, |docs| {
                docs.open(Kind::SchemaDiff, Some(capture.project.clone()));
            });
        }
    }

    pub(super) fn sync_schema_panels(&mut self, cx: &mut Context<Self>) {
        let docs = self.session.documents.borrow();
        let obsolete = self
            .schema_panels
            .iter()
            .filter(|(id, p)| {
                let panel = p.read(cx);
                !panel.capture.current(self)
                    || !docs.tabs.iter().any(|t| {
                        &t.id == *id
                            && t.kind == Kind::SchemaDiff
                            && t.project.as_deref() == Some(&panel.capture.project)
                    })
            })
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        let active = docs
            .active()
            .filter(|t| t.kind == Kind::SchemaDiff)
            .and_then(|t| Some((t.id.clone(), t.project.clone()?)));
        drop(docs);
        for id in obsolete {
            if let Some(p) = self.schema_panels.remove(&id) {
                p.update(cx, |p, _| p.reset());
            }
        }
        let Some((tab, project)) = active else { return };
        let Some(capture) = catalog::Capture::new(self, &project) else {
            return;
        };
        self.ensure_schema_names(&project, false, cx);
        self.schema_panels.entry(tab.clone()).or_insert_with(|| {
            let owner = cx.entity();
            cx.new(|cx| Panel {
                tab,
                owner: owner.downgrade(),
                capture,
                schemas: Default::default(),
                picker: None,
                pending: None,
                error: None,
                result: None,
                filter: Filter::All,
                selected: None,
                _observe: cx.observe(&owner, |_, _, cx| cx.notify()),
            })
        });
    }
}

impl Panel {
    fn current(&self, cx: &App) -> bool {
        self.owner
            .read_with(cx, |v, _| {
                self.capture.current(v)
                    && v.session.documents.borrow().tabs.iter().any(|t| {
                        t.id == self.tab
                            && t.kind == Kind::SchemaDiff
                            && t.project.as_deref() == Some(&self.capture.project)
                    })
            })
            .unwrap_or(false)
    }

    fn reset(&mut self) {
        self.pending = None;
        self.error = None;
        self.result = None;
        self.selected = None;
        self.picker = None;
    }

    fn choose(&mut self, side: usize, schema: String, cx: &mut Context<Self>) {
        if self.picker == Some(side) && self.current(cx) {
            self.picker = None;
            if self.schemas[side] != schema {
                self.reset();
                self.schemas[side] = schema;
            }
            cx.notify();
        }
    }

    fn compare(&mut self, cx: &mut Context<Self>) {
        if !self.current(cx) || self.schemas.iter().any(String::is_empty) {
            return;
        }
        let Ok(session) = self.owner.read_with(cx, |v, _| v.session.clone()) else {
            return;
        };
        self.reset();
        let id = Uuid::new_v4();
        let (cancel, cancelled) = watch::channel(());
        self.pending = Some(Pending {
            id,
            _cancel: cancel,
        });
        let job = session.runtime.spawn(read::compare(
            session.services.clone(),
            self.capture.clone(),
            self.schemas.clone(),
            cancelled,
            session.preflight_stop.subscribe(),
        ));
        cx.spawn(async move |panel, cx| {
            let result = job.await.map_err(|e| e.to_string()).and_then(|r| r);
            let _ = panel.update(cx, |p, cx| p.publish(id, result, cx));
        })
        .detach();
        cx.notify();
    }

    fn publish(&mut self, id: Uuid, result: Result<Comparison, String>, cx: &mut Context<Self>) {
        if self.pending.as_ref().is_some_and(|p| p.id == id) && self.current(cx) {
            self.pending = None;
            match result {
                Ok(diff) => self.result = Some(Arc::new(diff)),
                Err(error) => self.error = Some(error),
            }
            cx.notify();
        }
    }

    fn filter(&mut self, filter: Filter, cx: &mut Context<Self>) {
        self.filter = filter;
        if self.selected.as_ref().is_some_and(|s| {
            self.result
                .as_ref()
                .is_none_or(|r| !r.range(filter).contains(&s.index))
        }) {
            self.selected = None;
        }
        cx.notify();
    }
}
