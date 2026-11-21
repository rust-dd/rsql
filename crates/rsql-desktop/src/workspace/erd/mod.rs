mod camera;
mod cards;
mod export;
mod grid;
mod paint;
mod view;

use super::*;
use crate::runtime::diagrams::LiveDiagram;
use camera::{Camera, Gesture};
use rsql_desktop::erd::{DiagramOwner, DiagramState, TableId};

pub(super) struct Panel {
    schema: String,
    capture: catalog::Capture,
    session: Rc<Session>,
    model: Rc<LiveDiagram>,
    delivery: Option<Task<()>>,
    export_message: Option<String>,
    failure_index: usize,
    camera: Camera,
    grid: Option<grid::Tile>,
    gesture: Option<Gesture>,
    hover: Option<TableId>,
    viewport: Bounds<Pixels>,
    focus: FocusHandle,
    _blur: Subscription,
    _activation: Subscription,
}

impl Workspace {
    pub(crate) fn release_erd_images(&self, mut window: Option<&mut Window>, cx: &mut App) {
        for panel in self.erd_panels.values() {
            panel.update(cx, |p, cx| {
                if let Some(tile) = p.grid.take() {
                    cx.drop_image(tile.image, window.as_deref_mut());
                }
            });
        }
    }

    pub(super) fn open_erd(
        &mut self,
        capture: &catalog::Capture,
        schema: &str,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if capture.current(self)
            && self.navigation.catalogs[&capture.project]
                .schema_names()
                .ok()
                .flatten()
                .is_some_and(|names| names.iter().any(|name| name == schema))
        {
            self.update_documents(w, cx, |docs| {
                docs.open(Kind::Erd(schema.into()), Some(capture.project.clone()));
            });
        }
    }

    pub(super) fn sync_erd_panels(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        let docs = self.session.documents.borrow();
        let active = docs.active();
        let mut obsolete = Vec::new();
        for (id, entity) in &self.erd_panels {
            let panel = entity.read(cx);
            let matches = docs.tabs.iter().any(|t| {
                &t.id == id
                    && t.project.as_ref() == Some(&panel.capture.project)
                    && matches!(&t.kind, Kind::Erd(s) if s == &panel.schema)
            });
            let keep = matches
                && panel.capture.current_route(self)
                && self
                    .navigation
                    .catalogs
                    .contains_key(&panel.capture.project);
            if !keep {
                entity.update(cx, |p, _| {
                    p.clear_pointer();
                    p.delivery = None;
                    grid::Tile::release(&mut p.grid, w);
                    p.model.retire();
                });
                obsolete.push(id.clone());
                continue;
            }
            if active.is_none_or(|t| &t.id != id)
                && (panel.gesture.is_some() || panel.hover.is_some() || panel.grid.is_some())
            {
                entity.update(cx, |p, _| {
                    p.clear_pointer();
                    grid::Tile::release(&mut p.grid, w);
                });
            }
            let panel = entity.read(cx);
            if !panel.capture.current(self)
                && let Some(capture) = catalog::Capture::new(self, &panel.capture.project)
            {
                entity.update(cx, |p, cx| {
                    p.capture = capture;
                    p.refresh(cx);
                });
            }
        }
        for id in obsolete {
            self.erd_panels.remove(&id);
        }
        let active = active.and_then(|t| match (&t.kind, &t.project) {
            (Kind::Erd(schema), Some(project)) => {
                Some((t.id.clone(), project.clone(), schema.clone()))
            }
            _ => None,
        });
        drop(docs);
        if let Some((id, project, schema)) = active
            && !self.erd_panels.contains_key(&id)
            && !*self.session.preflight_stop.borrow()
            && let Some(capture) = catalog::Capture::new(self, &project)
        {
            let panel = cx.new(|cx| Panel::new(self.session.clone(), capture, schema, w, cx));
            self.erd_panels.insert(id, panel);
        }
    }
}

impl Panel {
    fn new(
        session: Rc<Session>,
        capture: catalog::Capture,
        schema: String,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        let model = session.register_diagram(DiagramOwner::new(
            capture.project.clone(),
            schema.clone(),
            capture.route.clone(),
        ));
        cx.on_release_in(w, |p, w, _| grid::Tile::release(&mut p.grid, w))
            .detach();
        let mut panel = Self {
            schema,
            capture,
            model,
            session,
            delivery: None,
            export_message: None,
            failure_index: 0,
            camera: Default::default(),
            grid: None,
            gesture: None,
            hover: None,
            viewport: Default::default(),
            _blur: cx.on_blur(&focus, w, |p, _, cx| {
                p.clear_pointer();
                cx.notify();
            }),
            _activation: cx.observe_window_activation(w, |p, w, cx| {
                if !w.is_window_active() {
                    p.clear_pointer();
                    cx.notify();
                }
            }),
            focus,
        };
        panel.refresh(cx);
        panel
    }

    fn clear_pointer(&mut self) {
        self.gesture = None;
        self.hover = None;
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        if *self.session.preflight_stop.borrow()
            || !self.capture.route.is_current()
            || !matches!(self.capture.generation.has_changed(), Ok(false))
        {
            return;
        }
        self.model.cancel_export();
        self.export_message = None;
        let mut owner = self.model.owner.borrow_mut();
        let Some(owner) = owner.as_mut() else { return };
        let load = owner.refresh(
            &self.session.runtime,
            self.session.services.clone(),
            self.capture.generation.clone(),
        );
        self.delivery = Some(cx.spawn(async move |panel, cx| {
            let delivery = load.wait().await;
            let _ = panel.update(cx, |p, cx| {
                let published = p
                    .model
                    .owner
                    .borrow_mut()
                    .as_mut()
                    .is_some_and(|o| o.publish(delivery));
                if published {
                    p.model.cancel_export();
                    p.export_message = None;
                    p.failure_index = 0;
                    p.clear_pointer();
                    cx.notify();
                }
            });
        }));
        cx.notify();
    }
}
