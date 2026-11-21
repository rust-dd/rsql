mod interaction;
mod paint;
mod popup;
mod projected;
mod render;
mod tiles;

use super::{Session, Workspace};
use crate::runtime::map::{Shapes, Ticket};
use gpui_kit::*;
use rsql_desktop::{
    geometry::MapTransform,
    results::{
        DecodedPage, FirstPageRead, ResultView, ViewChoice,
        diff::{RowSnapshot, SnapshotKey},
    },
};
use std::{
    rc::Rc,
    sync::{Arc, Weak},
};

pub(super) struct ResultMap {
    session: Rc<Session>,
    workspace: WeakEntity<Workspace>,
    tab: String,
    generation: u64,
    key: Option<SnapshotKey>,
    page: Weak<DecodedPage>,
    read: Option<FirstPageRead>,
    detected: Option<Option<usize>>,
    work: Option<(bool, Ticket, Task<()>)>,
    rows: Option<RowSnapshot>,
    shapes: Option<Shapes>,
    first_page: Option<String>,
    selected: Option<usize>,
    transform: MapTransform,
    bounds: gpui_kit::Bounds<Pixels>,
    unpainted: std::collections::HashSet<usize>,
    upload_error: Option<String>,
    fit_pending: bool,
    field_start: usize,
    drag: Option<interaction::Drag>,
    wheel: f32,
    tiles: tiles::Tiles,
    active: bool,
}

fn group_count(n: usize) -> String {
    if n < 1000 {
        n.to_string()
    } else {
        format!("{},{:03}", group_count(n / 1000), n % 1000)
    }
}

impl ResultMap {
    pub(super) fn new(
        session: Rc<Session>,
        workspace: WeakEntity<Workspace>,
        tab: String,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.on_release(|v, cx| v.tiles.retire(None, cx)).detach();
        cx.observe_window_activation(w, |v, w, cx| {
            if !w.is_window_active() {
                v.suspend(w, cx);
            }
            let _ = v.workspace.update(cx, |_, cx| cx.notify());
        })
        .detach();
        Self {
            session,
            workspace,
            tab,
            generation: 0,
            key: None,
            page: Weak::new(),
            read: None,
            detected: None,
            work: None,
            rows: None,
            shapes: None,
            first_page: None,
            selected: None,
            transform: MapTransform::new([0., 0.], [0., 0.], 2.),
            bounds: Default::default(),
            unpainted: Default::default(),
            upload_error: None,
            fit_pending: false,
            field_start: 0,
            drag: None,
            wheel: 0.,
            tiles: Default::default(),
            active: false,
        }
    }

    pub(super) fn available(&self, view: &ResultView) -> bool {
        self.key == Some(view.key())
            && self.detected.flatten().is_some()
            && (view.map_input().is_some() || view.can_read_map_page())
    }

    pub(super) fn suspend(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        self.active = false;
        self.read.take();
        self.rows.take();
        self.drag = None;
        self.upload_error = None;
        self.wheel = 0.;
        self.work.take();
        self.tiles.retire(Some(w), cx);
    }

    pub(super) fn sync(
        &mut self,
        view: &ResultView,
        generation: u64,
        foreground: bool,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = view.map_input();
        let key = view.key();
        let changed = self.key != Some(key) || self.generation != generation;
        if changed {
            self.suspend(w, cx);
            self.field_start = 0;
            if self.key.is_none_or(|old| old.result != key.result) || self.generation != generation
            {
                self.selected = None;
            }
            self.key = Some(key);
            self.generation = generation;
            self.detected = None;
            self.unpainted.clear();
            self.shapes = None;
            self.rows = None;
            self.first_page = None;
            self.page = Weak::new();
        }
        let page = input
            .as_ref()
            .map_or_else(Weak::new, |r| Arc::downgrade(&r.rows.page));
        if !self.page.ptr_eq(&page) {
            self.work.take();
            self.page = page;
            if let Some(input) = &input {
                self.first_page = input.first_page_of.map(|total| {
                    let total = group_count(total);
                    let count = group_count(input.rows.page.len());
                    format!("Map shows the first {count} of {total} rows. Narrow your query to map the full result.")
                });
            }
        }
        if input.is_none() && !view.can_read_map_page() {
            self.suspend(w, cx);
            self.shapes = None;
            self.detected = None;
            return;
        }
        let needs_page = foreground && w.is_window_active() && view.choice() == ViewChoice::Map;
        let visible = needs_page && view.field_detail.is_none();
        if self.active && !visible {
            self.suspend(w, cx);
        }
        self.active = visible;
        if !needs_page {
            self.rows.take();
            self.read.take();
        }
        let Some(rows) = input
            .map(|input| input.rows)
            .filter(|_| foreground && w.is_window_active())
        else {
            self.rows.take();
            self.work.take();
            if needs_page && self.read.as_ref().is_none_or(|read| !read.is_current()) {
                self.read = Some(view.loader.read_first_page());
            }
            return;
        };
        self.read.take();
        if needs_page && (self.shapes.is_some() || view.field_detail.is_some()) {
            self.rows = Some(rows);
            return;
        }
        if self
            .work
            .as_ref()
            .is_some_and(|(parse, _, _)| *parse == visible)
            || (self.detected.is_some() && !visible)
        {
            return;
        }
        self.work.take();
        let page = Arc::downgrade(&rows.page);
        let (ticket, receive) = self
            .session
            .map_worker
            .submit(rows, view.route.clone(), visible);
        let task = cx.spawn(async move |entity, cx| {
            let Ok(output) = receive.await else { return };
            let _ = entity.update(cx, |v, cx| {
                let mut docs = v.session.documents.borrow_mut();
                let Some(pane) = docs
                    .selected
                    .map(|index| &mut docs.tabs[index])
                    .filter(|tab| tab.id == v.tab)
                    .map(|tab| &mut tab.main)
                else {
                    return;
                };
                if pane.generation() != generation || pane.is_busy() {
                    return;
                }
                let Some(current) = pane.view_mut().filter(|view| view.key() == key) else {
                    return;
                };
                let Some(input) = current
                    .map_input()
                    .filter(|i| page.ptr_eq(&Arc::downgrade(&i.rows.page)))
                else {
                    return;
                };
                if visible
                    != (current.choice() == ViewChoice::Map && current.field_detail.is_none())
                {
                    return;
                }
                let fallback = visible && output.column.is_none() && input.rows.filtered.is_none();
                if fallback {
                    current.choose(ViewChoice::Grid);
                    docs.sync_views();
                }
                drop(docs);
                let rows = (visible && !fallback).then_some(input.rows);
                v.detected = Some(output.column);
                v.shapes = output.shapes.filter(|_| !fallback);
                if let Some(shapes) = &v.shapes {
                    v.selected = v
                        .selected
                        .filter(|row| shapes.geometries.iter().any(|g| g.row_index() == *row));
                    v.rows = rows;
                    v.fit_pending = true;
                }
                v.work.take();
                let _ = v.workspace.update(cx, |_, cx| cx.notify());
                cx.notify();
            });
        });
        self.work = Some((visible, ticket, task));
    }

    fn layout(&mut self, bounds: gpui_kit::Bounds<Pixels>, w: &mut Window, cx: &mut Context<Self>) {
        self.bounds = bounds;
        let viewport = self.viewport();
        self.transform.resize(viewport);
        if self.fit_pending && viewport.iter().all(|n| *n > 0.) {
            self.fit();
        }
        let clip = bounds.intersect(&w.content_mask().bounds);
        if self.active
            && self
                .shapes
                .as_ref()
                .is_some_and(|s| !s.geometries.is_empty())
            && clip.size.width > px(0.)
            && clip.size.height > px(0.)
        {
            self.tiles
                .layout(&self.session, self.transform, bounds, clip, w, cx);
        } else if self.tiles.consumer.is_some() {
            self.tiles.retire(Some(w), cx);
        }
    }

    fn viewport(&self) -> [f64; 2] {
        [self.bounds.size.width, self.bounds.size.height].map(|n| f64::from(f32::from(n)))
    }

    fn fit(&mut self) {
        if let Some(shapes) = &self.shapes {
            self.transform = MapTransform::fit(&shapes.bounds, self.viewport());
            self.fit_pending = false;
        }
    }
}

impl Workspace {
    pub(crate) fn release_maps(&mut self, mut w: Option<&mut Window>, cx: &mut Context<Self>) {
        for live in self.editors.values_mut() {
            if let Some(map) = live.map.take() {
                map.update(cx, |v, cx| {
                    v.active = false;
                    v.work.take();
                    v.read.take();
                    v.rows.take();
                    v.shapes.take();
                    v.tiles.retire(w.as_deref_mut(), cx);
                });
            }
        }
    }
}
