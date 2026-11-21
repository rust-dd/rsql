use super::*;
use gpui_kit::component::ActiveTheme;
use image::{Frame, Rgba, RgbaImage};
use rsql_desktop::erd::Point as World;
use std::sync::Arc;

const PERIOD: f64 = 20.;
const TILE_SIDE: f64 = 256.;
const MAX_TEXELS: u32 = 512;

pub(super) struct Tile {
    zoom: f64,
    scale: f32,
    color: Hsla,
    cells: u32,
    pub(super) image: Arc<RenderImage>,
}

impl Tile {
    pub fn prepare<'a>(
        current: &'a mut Option<Self>,
        camera: Camera,
        w: &mut Window,
        cx: &App,
    ) -> &'a Self {
        let zoom = camera.zoom;
        let scale = w.scale_factor();
        let color = cx.theme().border.opacity(0.5);
        if current
            .as_ref()
            .is_some_and(|tile| tile.zoom != zoom || tile.scale != scale || tile.color != color)
        {
            Self::release(current, w);
        }
        current.get_or_insert_with(|| {
            let cells = (TILE_SIDE / (PERIOD * zoom)).ceil() as u32;
            let side = (f64::from(cells) * PERIOD * zoom * f64::from(scale))
                .round()
                .clamp(1., f64::from(MAX_TEXELS)) as u32;
            let image = Arc::new(RenderImage::new([Frame::new(pattern(cells, side, color))]));
            Self {
                zoom,
                scale,
                color,
                cells,
                image,
            }
        })
    }

    pub fn release(current: &mut Option<Self>, w: &mut Window) {
        if let Some(tile) = current.take() {
            let _ = w.drop_image(tile.image);
        }
    }

    pub fn layer(&self, camera: Camera, viewport: Size<Pixels>, extent: World) -> Div {
        let bounds = Bounds::from_corners(camera.pan, camera.local(extent));
        let clip = bounds.intersect(&Bounds::new(Point::default(), viewport));
        let mut layer = div()
            .absolute()
            .left(clip.left())
            .top(clip.top())
            .w(clip.size.width)
            .h(clip.size.height)
            .overflow_hidden();
        if clip.is_empty() {
            return layer;
        }
        let side = f32::from(camera.length(PERIOD * f64::from(self.cells)));
        let offset = |start: Pixels, pan: Pixels| -f32::from(start - pan).rem_euclid(side);
        let x = offset(clip.left(), camera.pan.x);
        let y = offset(clip.top(), camera.pan.y);
        let columns = ((f32::from(clip.size.width) - x) / side).ceil() as usize;
        let rows = ((f32::from(clip.size.height) - y) / side).ceil() as usize;
        for row in 0..rows {
            for column in 0..columns {
                let tile = img(self.image.clone())
                    .id(("erd-grid-tile", row * columns + column))
                    .object_fit(ObjectFit::Fill)
                    .absolute()
                    .left(px(x + column as f32 * side))
                    .top(px(y + row as f32 * side))
                    .size(px(side))
                    .test_support();
                layer = layer.child(tile);
            }
        }
        layer
    }
}

fn pattern(cells: u32, side: u32, color: Hsla) -> RgbaImage {
    let color = color.to_rgb();
    let byte = |v: f32| (v * 255.).round() as u8;
    // GPUI's RenderImage reads BGRA. Keep RGB in transparent texels for linear filtering.
    let mut pixels = RgbaImage::from_pixel(
        side,
        side,
        Rgba([byte(color.b), byte(color.g), byte(color.r), 0]),
    );
    let period = side as f32 / cells as f32;
    let diameter = period / PERIOD as f32;
    let radius = diameter * 0.5;
    for row in 0..cells {
        let cy = (row as f32 + 0.5) * period;
        for col in 0..cells {
            let cx = (col as f32 + 0.5) * period;
            if diameter <= 1. {
                // GPUI snaps small quads to device pixels. Keep the dot's projected area.
                let pixel = |center: f32| (center.ceil() as u32).saturating_sub(1);
                pixels.get_pixel_mut(pixel(cx), pixel(cy)).0[3] = byte(color.a * diameter.powi(2));
            } else {
                let span =
                    |center: f32| (center - radius).floor() as u32..(center + radius).ceil() as u32;
                for y in span(cy) {
                    for x in span(cx) {
                        let distance = (x as f32 + 0.5 - cx).hypot(y as f32 + 0.5 - cy);
                        let coverage = (radius + 0.5 - distance).clamp(0., 1.);
                        pixels.get_pixel_mut(x, y).0[3] = byte(color.a * coverage);
                    }
                }
            }
        }
    }
    pixels
}
