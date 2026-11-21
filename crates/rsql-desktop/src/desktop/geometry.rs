use gpui_kit::{
    App, Bounds, Pixels, PlatformDisplay, TitlebarOptions, Window, WindowBounds, WindowOptions,
    point, px, size,
};
use serde::{Deserialize, Serialize};
use std::{
    io::{self, Read},
    path::Path,
    rc::Rc,
};
use uuid::Uuid;

const FILENAME: &str = "window-geometry.json";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Geometry {
    display: Option<String>,
    origin: Option<[f32; 2]>,
    size: [f32; 2],
}

impl Geometry {
    pub fn read(directory: &Path) -> Option<Self> {
        let path = directory.join(FILENAME);
        let metadata = path.symlink_metadata().ok()?;
        if !metadata.is_file() || metadata.len() > 4096 {
            return None;
        }
        let mut bytes = Vec::new();
        std::fs::File::open(path)
            .ok()?
            .take(4097)
            .read_to_end(&mut bytes)
            .ok()?;
        serde_json::from_slice::<Self>(&bytes)
            .ok()
            .filter(Self::valid)
    }

    pub fn save(self, directory: &Path) -> io::Result<()> {
        let mut file = tempfile::NamedTempFile::new_in(directory)?;
        serde_json::to_writer(&mut file, &self)?;
        file.as_file().sync_all()?;
        file.persist(directory.join(FILENAME))?;
        Ok(())
    }

    fn valid(&self) -> bool {
        self.size.iter().all(|s| s.is_finite() && *s > 0.)
            && self
                .origin
                .is_none_or(|xy| xy.iter().all(|v| v.is_finite()))
    }

    pub fn capture(window: &Window, cx: &App) -> Option<Self> {
        // AppKit also reports a zoomed frame as Windowed.
        if window.is_maximized() || window.is_fullscreen() {
            return None;
        }
        let WindowBounds::Windowed(b) = window.window_bounds() else {
            return None;
        };
        let d = window.display(cx)?;
        let id = d.uuid().ok().filter(|&id| {
            unique_display(id, &cx.displays()).is_some_and(|matched| matched.id() == d.id())
        });
        let value = Self {
            display: id.map(|id| id.to_string()),
            origin: (id.is_some() && cx.compositor_name() != "Wayland")
                .then_some([b.origin.x.into(), b.origin.y.into()]),
            size: [b.size.width.into(), b.size.height.into()],
        };
        value.valid().then_some(value)
    }
}

fn unique_display(
    id: Uuid,
    displays: &[Rc<dyn PlatformDisplay>],
) -> Option<&Rc<dyn PlatformDisplay>> {
    if id.is_nil() {
        return None;
    }
    let mut matches = displays.iter().filter(|d| d.uuid().ok() == Some(id));
    let first = matches.next()?;
    matches.next().is_none().then_some(first)
}

fn initial_and_minimum(production: bool) -> (Option<[f32; 2]>, [f32; 2]) {
    if production {
        (Some([1440., 900.]), [1280., 768.])
    } else {
        (None, [600., 360.])
    }
}

fn clamp_bounds(
    area: Bounds<Pixels>,
    dimensions: [f32; 2],
    minimum: [f32; 2],
    origin: Option<[f32; 2]>,
) -> Bounds<Pixels> {
    let available = [f32::from(area.size.width), f32::from(area.size.height)];
    let start = [f32::from(area.origin.x), f32::from(area.origin.y)];
    let lengths =
        std::array::from_fn::<_, 2, _>(|i| dimensions[i].max(minimum[i]).min(available[i]));
    let xy = std::array::from_fn::<_, 2, _>(|i| {
        let room = (available[i] - lengths[i]).max(0.);
        origin.map_or(start[i] + room / 2., |o| {
            o[i].clamp(start[i], start[i] + room)
        })
    });
    Bounds::new(
        point(px(xy[0]), px(xy[1])),
        size(px(lengths[0]), px(lengths[1])),
    )
}

#[cfg(target_os = "macos")]
fn titled_frame_extra() -> [f32; 2] {
    use objc2_app_kit::{NSWindow, NSWindowStyleMask as Style};
    use objc2_foundation::{MainThreadMarker, NSRect, NSSize};

    // Match GPUI's opaque, resizable and minimizable native title bar.
    let style = Style::Titled | Style::Closable | Style::Resizable | Style::Miniaturizable;
    let content = NSRect::new(Default::default(), NSSize::new(800., 600.));
    let frame = NSWindow::frameRectForContentRect_styleMask(
        content,
        style,
        MainThreadMarker::new().expect("window options run on the main thread"),
    );
    [
        (frame.size.width - content.size.width) as f32,
        (frame.size.height - content.size.height) as f32,
    ]
}

pub fn options(saved: Option<Geometry>, cx: &App) -> WindowOptions {
    #[cfg(target_os = "macos")]
    let extra = titled_frame_extra();
    #[cfg(not(target_os = "macos"))]
    let extra = [0.; 2];
    window_options(
        saved,
        &cx.displays(),
        cx.primary_display(),
        cx.compositor_name() == "Wayland",
        extra,
    )
}

fn window_options(
    saved: Option<Geometry>,
    displays: &[Rc<dyn PlatformDisplay>],
    primary: Option<Rc<dyn PlatformDisplay>>,
    wayland: bool,
    extra: [f32; 2],
) -> WindowOptions {
    let (initial, mut minimum) = initial_and_minimum(cfg!(feature = "production"));
    let saved = saved.filter(Geometry::valid);

    let original = saved
        .as_ref()
        .and_then(|s| s.display.as_ref())
        .and_then(|id| Uuid::parse_str(id).ok())
        .and_then(|id| unique_display(id, displays).cloned());
    let display = original
        .clone()
        .or(primary)
        .or_else(|| displays.first().cloned());
    let bounds = display.as_ref().and_then(|d| {
        let mut area = d.visible_bounds();
        // WindowOptions takes a frame origin but a content size on macOS.
        area.size.width = (area.size.width - px(extra[0])).max(px(1.));
        area.size.height = (area.size.height - px(extra[1])).max(px(1.));
        minimum = [
            minimum[0].min(area.size.width.into()),
            minimum[1].min(area.size.height.into()),
        ];
        let dimensions = saved
            .as_ref()
            .map(|s| [s.size[0] - extra[0], s.size[1] - extra[1]])
            .or(initial)
            .or_else(|| {
                // Retain GPUI's default placement unless the default frame exceeds usable space.
                let default = d.default_bounds().size;
                (default.width > area.size.width || default.height > area.size.height)
                    .then_some([default.width.into(), default.height.into()])
            })?;
        let origin = original.and_then(|_| saved?.origin).filter(|_| !wayland);
        Some(WindowBounds::Windowed(clamp_bounds(
            area, dimensions, minimum, origin,
        )))
    });
    WindowOptions {
        app_id: Some(crate::profile::ID.to_owned()),
        display_id: display.map(|d| d.id()),
        window_bounds: bounds,
        window_min_size: Some(size(px(minimum[0]), px(minimum[1]))),
        titlebar: Some(TitlebarOptions {
            title: Some(crate::profile::NAME.into()),
            ..Default::default()
        }),
        ..Default::default()
    }
}
