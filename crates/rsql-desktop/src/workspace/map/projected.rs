use rsql_desktop::geometry::{Geometry, MAX_COORDINATES, MapTransform, continuous_world};
use std::ops::RangeInclusive;

pub(super) struct ConnectedPath<'a> {
    coords: &'a [[f64; 2]],
    transform: MapTransform,
    pub copies: RangeInclusive<i32>,
    pub polygon: bool,
}

impl<'a> ConnectedPath<'a> {
    pub fn new(geometry: &'a Geometry, transform: MapTransform, width: f64) -> Option<Self> {
        let coords = geometry.coordinates();
        let polygon = matches!(geometry, Geometry::Polygon(_));
        let mut vertices = continuous_world(coords);
        let first = vertices.next()?[0];
        let (mut min, mut max, mut last) = (first, first, first);
        for [x, _] in vertices {
            min = min.min(x);
            max = max.max(x);
            last = x;
        }
        // The producer closed the ring in source coordinates. A different final world
        // copy cannot close in the native plane without inventing a boundary.
        if polygon && last != first {
            return None;
        }
        // Include the edge tolerance used for picking the stroke.
        let left = transform.screen_to_world([-3., 0.])[0];
        let right = transform.screen_to_world([width + 3., 0.])[0];
        let first = (left - max).ceil() as i32;
        let last = (right - min).floor() as i32;
        let copies = first..=last;
        // A path winding many times around the earth must not expand into quadratic tessellation.
        let count = (i64::from(last) - i64::from(first) + 1).max(0) as usize;
        let viewport_copies = (right - left).ceil() as usize + 1;
        if coords.len().saturating_mul(count) > MAX_COORDINATES.saturating_mul(viewport_copies) {
            return None;
        }
        Some(Self {
            coords,
            transform,
            copies,
            polygon,
        })
    }

    pub fn vertices(&self, copy: i32) -> impl Iterator<Item = [f64; 2]> + Clone {
        continuous_world(self.coords)
            .map(move |[x, y]| self.transform.world_to_screen([x + f64::from(copy), y]))
    }

    pub fn hit(&self, point: [f64; 2]) -> bool {
        self.copies.clone().any(|copy| {
            let mut vertices = self.vertices(copy);
            let Some(first) = vertices.next() else {
                return false;
            };
            let mut a = first;
            let mut inside = false;
            for b in vertices {
                let [dx, dy] = [b[0] - a[0], b[1] - a[1]];
                let length = dx * dx + dy * dy;
                let [x, y] = [point[0] - a[0], point[1] - a[1]];
                let t = if length > 0. {
                    ((x * dx + y * dy) / length).clamp(0., 1.)
                } else {
                    0.
                };
                if (x - t * dx).hypot(y - t * dy) <= 3. {
                    return true;
                }
                if self.polygon && (a[1] > point[1]) != (b[1] > point[1]) && x < y * dx / dy {
                    inside = !inside;
                }
                a = b;
            }
            inside
        })
    }
}
