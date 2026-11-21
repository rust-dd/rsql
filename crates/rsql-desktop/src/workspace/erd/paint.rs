use super::*;
use gpui_kit::component::{ActiveTheme, label::Label};
use rsql_desktop::erd::{Edge, EdgeGeometry, Point as World};

impl Panel {
    pub(super) fn drawing(&self, cx: &Context<Self>) -> impl IntoElement + use<> {
        let view = cx.weak_entity();
        let bounds_view = view.clone();
        canvas(
            move |bounds, w, cx| {
                let _ = bounds_view.update(cx, |p, cx| {
                    if p.viewport != bounds {
                        p.viewport = bounds;
                        p.clear_pointer();
                        cx.notify();
                    }
                });
                w.insert_hitbox(bounds, HitboxBehavior::Normal)
            },
            move |bounds, surface, w, cx| {
                let _ = view.read_with(cx, |p, cx| {
                    if let Some(d) = p
                        .model
                        .owner
                        .borrow()
                        .as_ref()
                        .and_then(DiagramOwner::diagram)
                    {
                        p.paint(bounds, d, w, cx);
                    }
                });
                w.on_mouse_event(move |e: &MouseMoveEvent, phase, w, cx| {
                    if phase == DispatchPhase::Capture {
                        let over_surface = surface.is_hovered(w);
                        let _ = view.update(cx, |p, cx| p.moved(e, over_surface, cx));
                    }
                });
            },
        )
        .absolute()
        .inset_0()
        .size_full()
    }

    fn paint(&self, bounds: Bounds<Pixels>, d: &DiagramState, w: &mut Window, cx: &App) {
        let cam = self.camera;
        let screen = |v| bounds.origin + cam.local(v);
        for edge in d.graph().edges() {
            let Some(g) = d.geometry(edge.id) else {
                continue;
            };
            let highlighted = self
                .hover
                .is_some_and(|id| id == edge.source.table || id == edge.target.table);
            let width = if highlighted { 2. } else { 1.5 };
            let start = screen(g.start);
            let end = screen(g.end);
            let c1 = screen(g.control1);
            let c2 = screen(g.control2);
            let outer = Bounds::from_corners(
                point(start.x.min(end.x).min(c2.x).min(c1.x), start.y.min(end.y)),
                point(start.x.max(end.x).max(c2.x).max(c1.x), start.y.max(end.y)),
            )
            .dilate(cam.length(6. * width));
            if !outer.intersects(&bounds) {
                continue;
            }
            let opacity = if highlighted {
                1.
            } else if self.hover.is_some() {
                0.15
            } else {
                0.7
            };
            let color = if highlighted {
                cx.theme().primary
            } else {
                cx.theme().muted_foreground
            }
            .opacity(opacity);
            let mut curve = PathBuilder::stroke(cam.length(width));
            curve.move_to(start);
            curve.cubic_bezier_to(end, c1, c2);
            if let Ok(path) = curve.build() {
                w.paint_path(path, color);
            }
            let [a, b, c] = g.arrow(width).map(screen);
            let mut arrow = PathBuilder::fill();
            arrow.move_to(a);
            arrow.line_to(b);
            arrow.line_to(c);
            arrow.close();
            if let Ok(path) = arrow.build() {
                w.paint_path(path, cx.theme().primary.opacity(opacity));
            }
            dot(start, cam.length(g.source_radius), color, w);
        }
    }

    pub(super) fn edge_label(
        &self,
        d: &DiagramState,
        edge: &Edge,
        g: EdgeGeometry,
        w: &Window,
        cx: &App,
    ) -> Option<impl IntoElement + use<>> {
        let source = &d.graph().column(edge.source)?.name;
        let target = &d.graph().column(edge.target)?.name;
        let label: SharedString = format!("{source} → {target}").into();
        let mut style = w.text_style();
        style.font_family = cx.theme().mono_font_family.clone();
        let font_size = self.camera.length(9.);
        let line = w.text_system().shape_line(
            label.clone(),
            font_size,
            &[style.to_run(label.len())],
            None,
        );
        let anchor = self.camera.local(World::new(
            (g.start.x + g.end.x) * 0.5,
            g.start.y.min(g.end.y) - 6.,
        ));
        let width = line.width().ceil() + px(1.);
        let bounds = Bounds::new(
            point(anchor.x - width * 0.5, anchor.y - font_size),
            size(width, self.camera.length(12.)),
        );
        bounds
            .intersects(&Bounds::new(Point::default(), self.viewport.size))
            .then(|| {
                Label::new(label)
                    .id(format!("fk-{}-{}", edge.id.constraint, edge.id.pair))
                    .absolute()
                    .left(bounds.left())
                    .top(bounds.top())
                    .w(width)
                    .text_center()
                    .whitespace_nowrap()
                    .text_size(font_size)
                    .line_height(self.camera.length(12.))
                    .text_color(cx.theme().primary)
            })
    }
}

fn dot(center: Point<Pixels>, radius: Pixels, color: Hsla, w: &mut Window) {
    // GPUI snaps quad bounds to device pixels. Preserve subpixel dots' area at low zoom.
    let diameter = f32::from(radius * 2.) * w.scale_factor();
    let (radius, color) = if diameter < 1. {
        (
            px(0.5 / w.scale_factor()),
            color.opacity(diameter * diameter),
        )
    } else {
        (radius, color)
    };
    let mut quad = fill(
        Bounds::new(
            center - point(radius, radius),
            size(radius * 2., radius * 2.),
        ),
        color,
    );
    quad.corner_radii = radius.into();
    w.paint_quad(quad);
}
