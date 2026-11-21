use super::*;
use rsql_desktop::erd::{Point as World, TableBox};

#[derive(Clone, Copy)]
pub(super) struct Camera {
    pub pan: Point<Pixels>,
    pub zoom: f64,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            pan: Default::default(),
            zoom: 1.,
        }
    }
}

impl Camera {
    pub fn length(self, distance: f64) -> Pixels {
        px((self.zoom * distance) as f32)
    }
    pub fn local(self, p: World) -> Point<Pixels> {
        self.pan + point(self.length(p.x), self.length(p.y))
    }
    pub fn world(self, local: Point<Pixels>) -> World {
        let d = local - self.pan;
        World::new(
            f64::from(f32::from(d.x)) / self.zoom,
            f64::from(f32::from(d.y)) / self.zoom,
        )
    }
    pub fn bounds(self, b: TableBox) -> Bounds<Pixels> {
        Bounds::new(
            self.local(b.origin),
            size(self.length(b.width), self.length(b.height)),
        )
    }
}

pub(super) fn extent(diagram: &DiagramState) -> World {
    diagram.boxes().fold(World::new(800., 600.), |acc, b| {
        World::new(
            acc.x.max(b.origin.x + b.width + 60.),
            acc.y.max(b.origin.y + b.height + 60.),
        )
    })
}

#[derive(Clone, Copy)]
pub(super) enum Gesture {
    Pan(Point<Pixels>),
    Table(TableId, World),
}

impl Panel {
    fn hit(&self, at: Point<Pixels>, diagram: &DiagramState) -> Option<TableBox> {
        if !self.viewport.contains(&at) {
            return None;
        }
        let local = at - self.viewport.origin;
        diagram
            .boxes()
            .filter(|b| self.camera.bounds(*b).contains(&local))
            .last()
    }
    pub(super) fn press(&mut self, at: Point<Pixels>, w: &mut Window, cx: &mut Context<Self>) {
        let owner = self.model.owner.borrow();
        let Some(diagram) = owner.as_ref().and_then(DiagramOwner::diagram) else {
            return;
        };
        let local = at - self.viewport.origin;
        let hit = self.hit(at, diagram);
        self.hover = hit.map(|b| b.table);
        self.gesture = Some(match hit {
            Some(b) => {
                let world = self.camera.world(local);
                Gesture::Table(
                    b.table,
                    World::new(world.x - b.origin.x, world.y - b.origin.y),
                )
            }
            None => Gesture::Pan(local - self.camera.pan),
        });
        w.prevent_default();
        self.focus.focus(w, cx);
        cx.notify();
    }
    pub(super) fn moved(&mut self, e: &MouseMoveEvent, over_surface: bool, cx: &mut Context<Self>) {
        let owner = self.model.owner.borrow();
        let Some(diagram) = owner.as_ref().and_then(DiagramOwner::diagram) else {
            return;
        };
        let had_gesture = self.gesture.is_some();
        if e.pressed_button != Some(MouseButton::Left) {
            self.gesture = None;
        }
        let local = e.position - self.viewport.origin;
        match self.gesture {
            Some(Gesture::Pan(offset)) => self.camera.pan = local - offset,
            Some(Gesture::Table(id, offset)) => {
                let p = self.camera.world(local);
                diagram.move_table(
                    id,
                    World::new((p.x - offset.x).max(0.), (p.y - offset.y).max(0.)),
                );
            }
            None => {}
        }
        let hover = over_surface
            .then(|| self.hit(e.position, diagram))
            .flatten()
            .map(|b| b.table);
        if self.hover != hover || had_gesture {
            self.hover = hover;
            cx.notify();
        }
    }
    pub(super) fn zoom(&mut self, factor: f64, at: Option<Point<Pixels>>, cx: &mut Context<Self>) {
        self.gesture = None;
        self.camera.zoom = (self.camera.zoom * factor).clamp(0.1, 3.);
        self.hover = at.and_then(|at| {
            let owner = self.model.owner.borrow();
            self.hit(at, owner.as_ref()?.diagram()?).map(|b| b.table)
        });
        cx.notify();
    }
    pub(super) fn fit(&mut self, cx: &mut Context<Self>) {
        self.clear_pointer();
        let owner = self.model.owner.borrow();
        if let Some(d) = owner.as_ref().and_then(DiagramOwner::diagram) {
            let bounds = extent(d);
            let viewport = self.viewport.size;
            self.camera.zoom = (0.9
                * (f64::from(f32::from(viewport.width)) / bounds.x)
                    .min(f64::from(f32::from(viewport.height)) / bounds.y))
            .clamp(0.1, 2.);
            self.camera.pan = point(px(10.), px(10.));
            cx.notify();
        }
    }
}
