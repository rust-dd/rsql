use super::*;
use rsql_desktop::geometry::Geometry;

pub(super) struct Drag {
    origin: Point<Pixels>,
    last: Point<Pixels>,
    moved: bool,
}

impl ResultMap {
    fn local(&self, position: Point<Pixels>) -> [f64; 2] {
        let p = position - self.bounds.origin;
        [p.x, p.y].map(|n| f64::from(f32::from(n)))
    }

    pub(super) fn zoom(&mut self, delta: f64, anchor: [f64; 2]) {
        let before = self.transform.screen_to_world(anchor);
        self.transform.set_zoom(self.transform.zoom() + delta);
        let after = self.transform.screen_to_world(anchor);
        let center = self.viewport().map(|n| n / 2.);
        let [x, y] = self.transform.screen_to_world(center);
        self.transform
            .set_center(rsql_desktop::geometry::longitude_latitude([
                x + before[0] - after[0],
                y + before[1] - after[1],
            ]));
    }

    pub(super) fn press(&mut self, event: &MouseDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.drag = Some(Drag {
            origin: event.position,
            last: event.position,
            moved: false,
        });
        cx.stop_propagation();
    }

    pub(super) fn motion(
        &mut self,
        event: &MouseMoveEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.pressed_button != Some(MouseButton::Left) {
            self.drag = None;
            return;
        }
        let Some(drag) = &mut self.drag else { return };
        let distance = event.position - drag.origin;
        if !drag.moved && f32::from(distance.x).hypot(f32::from(distance.y)) < 3. {
            return;
        }
        drag.moved = true;
        let delta = event.position - drag.last;
        drag.last = event.position;
        let [x, y] = self.viewport().map(|n| n / 2.);
        self.transform.set_center(self.transform.from_screen([
            x - f64::from(f32::from(delta.x)),
            y - f64::from(f32::from(delta.y)),
        ]));
        cx.notify();
    }

    pub(super) fn release(&mut self, event: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(drag) = self.drag.take() else { return };
        if !drag.moved && self.bounds.contains(&event.position) {
            self.field_start = 0;
            self.selected = self.hit(self.local(event.position));
            cx.notify();
        }
    }

    pub(super) fn scroll(&mut self, event: &ScrollEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.wheel += match event.delta {
            ScrollDelta::Lines(p) => p.y,
            ScrollDelta::Pixels(p) => f32::from(p.y) / 40.,
        };
        let steps = self.wheel.trunc();
        self.wheel -= steps;
        if steps != 0. {
            self.zoom(f64::from(steps), self.local(event.position));
            cx.notify();
        }
        cx.stop_propagation();
    }

    fn hit(&self, p: [f64; 2]) -> Option<usize> {
        let shapes = self.shapes.as_ref()?;
        shapes
            .geometries
            .iter()
            .rev()
            .filter(|g| !self.unpainted.contains(&g.row_index()))
            .find(|g| {
                let coords = g.coordinates();
                if matches!(g.as_ref(), Geometry::Point(_)) {
                    let [x, y] = self.transform.to_screen(coords[0]);
                    return (x - p[0]).hypot(y - p[1]) <= 7.;
                }
                projected::ConnectedPath::new(g, self.transform, self.viewport()[0])
                    .is_some_and(|path| path.hit(p))
            })
            .map(|g| g.row_index())
    }
}
