use super::*;
use rsql_desktop::geometry::Geometry;

impl ResultMap {
    pub(super) fn paint(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        let Some(shapes) = &self.shapes else { return };
        let bounds = self.bounds;
        let width = self.viewport()[0];
        let color = Hsla::from(rgb(0x3b82f6));
        let mut unpainted = std::collections::HashSet::new();
        let error = w.with_content_mask(Some(ContentMask { bounds }), |w| {
            let error = self.tiles.paint(w);
            for geometry in &shapes.geometries {
                let screen = |coord| {
                    let [x, y] = self.transform.to_screen(coord);
                    bounds.origin + point(px(x as f32), px(y as f32))
                };
                let coordinates = geometry.coordinates();
                match geometry.as_ref() {
                    Geometry::Point(_) => {
                        let origin = screen(coordinates[0]) - point(px(7.), px(7.));
                        w.paint_quad(quad(
                            gpui_kit::Bounds::new(origin, size(px(14.), px(14.))),
                            px(7.),
                            color.opacity(0.7),
                            px(2.),
                            color,
                            BorderStyle::Solid,
                        ));
                    }
                    Geometry::LineString(_) | Geometry::Polygon(_) => {
                        let Some(path) =
                            projected::ConnectedPath::new(geometry, self.transform, width)
                        else {
                            unpainted.insert(geometry.row_index());
                            continue;
                        };
                        let stroke_width = px(if path.polygon { 2. } else { 3. });
                        let paths = path
                            .copies
                            .clone()
                            .map(|copy| {
                                let build = |mut builder: PathBuilder| {
                                    let mut vertices = path.vertices(copy).map(|[x, y]| {
                                        bounds.origin + point(px(x as f32), px(y as f32))
                                    });
                                    builder.move_to(vertices.next().unwrap());
                                    for vertex in vertices {
                                        builder.line_to(vertex);
                                    }
                                    if path.polygon {
                                        builder.close();
                                    }
                                    builder.build()
                                };
                                let fill =
                                    path.polygon.then(|| build(PathBuilder::fill())).transpose();
                                let stroke = build(PathBuilder::stroke(stroke_width));
                                fill.and_then(|fill| stroke.map(|stroke| (fill, stroke)))
                            })
                            .collect::<Result<Vec<_>, _>>();
                        let Ok(paths) = paths else {
                            unpainted.insert(geometry.row_index());
                            continue;
                        };
                        for (fill, stroke) in paths {
                            if let Some(fill) = fill {
                                w.paint_path(fill, color.opacity(0.2));
                            }
                            w.paint_path(stroke, color);
                        }
                    }
                }
            }
            error
        });
        if self.unpainted != unpainted || self.upload_error != error {
            self.unpainted = unpainted;
            self.upload_error = error;
            self.selected = self.selected.filter(|row| !self.unpainted.contains(row));
            cx.notify();
        }
    }
}
