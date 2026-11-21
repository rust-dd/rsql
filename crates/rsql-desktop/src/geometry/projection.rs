use std::f64::consts::{PI, TAU};

const MAX_LATITUDE: f64 = 85.051_128_779_806_6;
const TILE_SIZE: f64 = 256.;
const FIT_PADDING: f64 = 40.;

/// Normalized Web Mercator coordinates, with the northwestern corner at (0, 0).
pub fn world([longitude, latitude]: [f64; 2]) -> [f64; 2] {
    let x = (longitude.rem_euclid(360.) / 360. + 0.5).rem_euclid(1.);
    let latitude = latitude.clamp(-MAX_LATITUDE, MAX_LATITUDE).to_radians();
    [x, (0.5 - latitude.tan().asinh() / TAU).clamp(0., 1.)]
}

/// Project consecutive vertices into one continuous copy of the world.
pub fn continuous_world(coords: &[[f64; 2]]) -> impl Iterator<Item = [f64; 2]> + Clone {
    let mut previous = None;
    let mut copy = 0.;
    coords.iter().map(move |&coord| {
        let [x, y] = world(coord);
        if let Some(last) = previous {
            let distance: f64 = last - x;
            if distance.abs() > 0.5 {
                copy += distance.signum();
            }
        }
        previous = Some(x);
        [x + copy, y]
    })
}

pub fn longitude_latitude([x, y]: [f64; 2]) -> [f64; 2] {
    let longitude = x.rem_euclid(1.) * 360. - 180.;
    let latitude = (PI * (1. - 2. * y.clamp(0., 1.)))
        .sinh()
        .atan()
        .to_degrees();
    [longitude, latitude]
}

#[derive(Clone, Copy, Debug)]
struct Extent {
    min: f64,
    max: f64,
}

impl Extent {
    fn point(value: f64) -> Self {
        Self {
            min: value,
            max: value,
        }
    }
    fn extend(&mut self, value: f64) {
        self.min = self.min.min(value);
        self.max = self.max.max(value);
    }
    fn span(self) -> f64 {
        self.max - self.min
    }
    fn center(self) -> f64 {
        (self.max + self.min) / 2.
    }
}

/// Track longitude extents on both sides of the date line without retaining points.
#[derive(Clone, Copy, Debug, Default)]
pub struct Bounds {
    axes: Option<[Extent; 3]>,
}

impl Bounds {
    pub fn extend(&mut self, coordinate: [f64; 2]) {
        if !coordinate.iter().all(|c| c.is_finite()) {
            return;
        }
        let [x, y] = world(coordinate);
        let values = [x, (x + 0.5).rem_euclid(1.) - 0.5, y];
        if let Some(axes) = &mut self.axes {
            for (axis, value) in axes.iter_mut().zip(values) {
                axis.extend(value);
            }
        } else {
            self.axes = Some(values.map(Extent::point));
        }
    }

    /// Find the smallest longitude envelope containing one complete copy of each drawable shape.
    pub fn for_geometries<'a>(
        geometries: impl IntoIterator<Item = &'a super::Geometry>,
        mut cancelled: impl FnMut() -> bool,
    ) -> Option<Self> {
        let mut intervals = Vec::new();
        let mut vertical = None::<Extent>;
        let mut right = f64::NEG_INFINITY;
        for geometry in geometries {
            let mut vertices = continuous_world(geometry.coordinates()).enumerate();
            let (_, [first, top]) = vertices.next()?;
            let mut x = Extent::point(first);
            let mut y = Extent::point(top);
            let mut last = first;
            if cancelled() {
                return None;
            }
            for (i, [next, latitude]) in vertices {
                if i % 1024 == 0 && cancelled() {
                    return None;
                }
                x.extend(next);
                y.extend(latitude);
                last = next;
            }
            if matches!(geometry, super::Geometry::Polygon(_)) && last != first {
                continue;
            }
            let min = x.min.rem_euclid(1.);
            let max = min + x.span();
            intervals.push(Extent { min, max });
            right = right.max(max);
            let total = vertical.get_or_insert(y);
            total.extend(y.min);
            total.extend(y.max);
        }
        let Some(y) = vertical else {
            return Some(Self::default());
        };
        intervals.sort_unstable_by(|a, b| a.min.total_cmp(&b.min));
        let mut best = Extent {
            min: intervals[0].min,
            max: right,
        };
        // Each possible left edge is a shape start. Earlier starts move one full world
        // to the right, so a running maximum covers every translated right edge.
        for (i, shape) in intervals.into_iter().enumerate() {
            if i % 1024 == 0 && cancelled() {
                return None;
            }
            let candidate = Extent {
                min: shape.min,
                max: right,
            };
            if candidate.span() < best.span() {
                best = candidate;
            }
            right = right.max(shape.max + 1.);
        }
        Some(Self {
            axes: Some([best, best, y]),
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub struct MapTransform {
    center: [f64; 2],
    viewport: [f64; 2],
    zoom: f64,
}

impl MapTransform {
    pub fn new(center: [f64; 2], viewport: [f64; 2], zoom: f64) -> Self {
        let mut map = Self {
            center: [0.5, 0.5],
            viewport: [0.; 2],
            zoom: 2.,
        };
        map.set_center(center);
        map.resize(viewport);
        map.set_zoom(zoom);
        map
    }

    pub fn fit(bounds: &Bounds, viewport: [f64; 2]) -> Self {
        let mut map = Self::new([0., 0.], viewport, 2.);
        let Some([normal, shifted, y]) = bounds.axes else {
            return map;
        };
        let x = if shifted.span() < normal.span() {
            shifted
        } else {
            normal
        };
        map.center = [x.center().rem_euclid(1.), y.center()];
        let mut zoom = 16.0_f64;
        for (pixels, span) in map.viewport.into_iter().zip([x.span(), y.span()]) {
            if span > 0. {
                let available = (pixels - 2. * FIT_PADDING).max(1.);
                zoom = zoom.min((available / (TILE_SIZE * span)).log2().floor());
            }
        }
        map.set_zoom(zoom);
        map
    }

    pub fn set_center(&mut self, coordinate: [f64; 2]) {
        if coordinate.iter().all(|n| n.is_finite()) {
            self.center = world(coordinate);
        }
    }

    pub fn center(&self) -> [f64; 2] {
        longitude_latitude(self.center)
    }

    pub fn zoom(&self) -> f64 {
        self.zoom
    }

    pub fn set_zoom(&mut self, zoom: f64) {
        if zoom.is_finite() {
            self.zoom = zoom.clamp(0., 19.);
        }
    }

    pub fn resize(&mut self, viewport: [f64; 2]) {
        self.viewport = viewport.map(|size| if size.is_finite() { size.max(0.) } else { 0. });
    }

    fn scale(&self) -> f64 {
        TILE_SIZE * self.zoom.exp2()
    }

    pub fn to_screen(&self, coordinate: [f64; 2]) -> [f64; 2] {
        let [x, y] = world(coordinate);
        let dx = (x - self.center[0] + 0.5).rem_euclid(1.) - 0.5;
        self.world_to_screen([self.center[0] + dx, y])
    }

    /// An unwrapped world x selects a particular copy of the map.
    pub fn world_to_screen(&self, [x, y]: [f64; 2]) -> [f64; 2] {
        [
            self.viewport[0] / 2. + (x - self.center[0]) * self.scale(),
            self.viewport[1] / 2. + (y - self.center[1]) * self.scale(),
        ]
    }

    pub fn screen_to_world(&self, [x, y]: [f64; 2]) -> [f64; 2] {
        let scale = self.scale();
        [
            self.center[0] + (x / scale - self.viewport[0] / (2. * scale)),
            self.center[1] + (y / scale - self.viewport[1] / (2. * scale)),
        ]
    }

    pub fn from_screen(&self, screen: [f64; 2]) -> [f64; 2] {
        longitude_latitude(self.screen_to_world(screen))
    }
}
