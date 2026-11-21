use super::*;
use rsql_desktop::tiles::{
    ACTIVE_LIMIT, QUEUE_LIMIT, TileConsumer, TileKey, TilePaintLease, TileState,
};
use std::collections::HashMap;

#[derive(PartialEq)]
struct Viewport {
    center: [f64; 2],
    zoom: f64,
    bounds: gpui_kit::Bounds<Pixels>,
    clip: gpui_kit::Bounds<Pixels>,
}

#[derive(Default)]
pub(super) struct Tiles {
    pub(super) consumer: Option<TileConsumer>,
    viewport: Option<Viewport>,
    positions: Vec<(TileKey, Point<Pixels>)>,
    leases: HashMap<TileKey, TilePaintLease>,
    watching: Option<Task<()>>,
    expiry: Option<(u64, Task<()>)>,
    pub(super) message: Option<String>,
    pub(super) can_retry: bool,
    admission: Option<String>,
}

impl Tiles {
    pub(super) fn retire(&mut self, mut w: Option<&mut Window>, cx: &mut App) {
        for (_, lease) in self.leases.drain() {
            lease.retire(w.as_deref_mut(), cx);
        }
        *self = Self::default();
    }

    pub(super) fn retry(&mut self, w: &mut Window, cx: &mut Context<ResultMap>) {
        if self.admission.is_some() {
            self.retire(Some(w), cx);
        } else if let Some(consumer) = &self.consumer {
            self.admission = consumer.retry().err();
            self.refresh(w, cx);
        }
    }

    fn positions(
        map: MapTransform,
        bounds: gpui_kit::Bounds<Pixels>,
        clip: gpui_kit::Bounds<Pixels>,
    ) -> Result<Vec<(TileKey, Point<Pixels>)>, String> {
        let z = map.zoom() as u8;
        let width = (1u32 << z) as f64;
        let world_at = |p: Point<Pixels>| {
            let local = p - bounds.origin;
            map.screen_to_world([local.x, local.y].map(|n| f64::from(f32::from(n))))
        };
        let [left, top] = world_at(clip.origin).map(|n| (n * width).floor() as i64);
        let [right, bottom] = world_at(clip.bottom_right()).map(|n| (n * width).ceil() as i64);
        let y = top.max(0)..bottom.min(width as i64);
        if (right - left).saturating_mul((y.end - y.start).max(0))
            > (ACTIVE_LIMIT + QUEUE_LIMIT) as i64
        {
            return Err("Visible tile queue capacity reached".into());
        }
        let mut slots = Vec::new();
        for y in y {
            for x in left..right {
                let [sx, sy] = map.world_to_screen([x as f64 / width, y as f64 / width]);
                slots.push((
                    TileKey::new(z, x, y).unwrap(),
                    bounds.origin + point(px(sx as f32), px(sy as f32)),
                ));
            }
        }
        Ok(slots)
    }

    pub(super) fn layout(
        &mut self,
        session: &Session,
        map: MapTransform,
        bounds: gpui_kit::Bounds<Pixels>,
        clip: gpui_kit::Bounds<Pixels>,
        w: &mut Window,
        cx: &mut Context<ResultMap>,
    ) {
        if self.consumer.is_none() {
            match session.tiles.consumer() {
                Ok(consumer) => {
                    let mut revision = consumer.revisions();
                    self.watching = Some(cx.spawn_in(w, async move |view, cx| {
                        while revision.changed().await.is_ok() {
                            if view
                                .update_in(cx, |v, w, cx| v.tiles.refresh(w, cx))
                                .is_err()
                            {
                                break;
                            }
                        }
                    }));
                    self.consumer = Some(consumer);
                }
                Err(error) => {
                    self.admission = Some(error);
                    self.refresh(w, cx);
                    return;
                }
            }
        }
        let viewport = Viewport {
            center: map.center(),
            zoom: map.zoom(),
            bounds,
            clip,
        };
        if self.viewport.as_ref() == Some(&viewport) {
            if self.expiry.is_some() {
                self.refresh(w, cx);
            }
            return;
        }
        self.viewport = Some(viewport);
        let slots = Self::positions(map, bounds, clip);
        let (positions, mut error) = match slots {
            Ok(slots) => (slots, None),
            Err(e) => (Vec::new(), Some(e)),
        };
        let keys = positions.iter().map(|(k, _)| *k).collect::<Vec<_>>();
        if let Err(e) = self.consumer.as_mut().unwrap().set_visible(&keys) {
            error = Some(e);
        }
        self.positions = positions;
        self.admission = error;
        self.refresh(w, cx);
    }

    fn refresh(&mut self, w: &mut Window, cx: &mut Context<ResultMap>) {
        let mut images = HashMap::new();
        let mut error = self.admission.clone();
        let mut loading = false;
        if let Some(consumer) = &self.consumer {
            for (key, _) in &self.positions {
                let image = match consumer.state(*key) {
                    Some(TileState::Ready(image)) => Some(image),
                    Some(TileState::Error { message, image }) => {
                        error.get_or_insert(message);
                        image
                    }
                    _ => {
                        loading = true;
                        None
                    }
                };
                if let Some(image) = image {
                    images.insert(*key, image);
                }
            }
        }
        let mut changed = false;
        for (key, lease) in std::mem::take(&mut self.leases) {
            if images.get(&key).is_some_and(|image| lease.matches(image)) {
                images.remove(&key);
                self.leases.insert(key, lease);
            } else {
                lease.retire(Some(w), cx);
                changed = true;
            }
        }
        changed |= !images.is_empty();
        self.leases
            .extend(images.into_iter().map(|(k, i)| (k, TilePaintLease::new(i))));
        let retries = self
            .consumer
            .as_ref()
            .map(TileConsumer::retry_status)
            .unwrap_or_default();
        let can_retry = self.admission.is_some() || retries.manual > 0;
        changed |= self.can_retry != can_retry;
        self.can_retry = can_retry;
        let message = error
            .map(|e| {
                let mut detail = format!("Tiles unavailable · {e}");
                if retries.scheduled > 0 {
                    detail.push_str(&format!(" · Retry scheduled ({})", retries.scheduled));
                    if retries.manual > 0 {
                        detail.push_str(&format!(" · {} eligible now", retries.manual));
                    }
                }
                detail
            })
            .or_else(|| loading.then(|| "Loading map tiles…".into()));
        if self.message != message || changed {
            self.message = message;
            cx.notify();
        }
        let deadline = self
            .consumer
            .as_ref()
            .and_then(TileConsumer::fallback_deadline);
        if self.expiry.as_ref().map(|(until, _)| *until) != deadline.map(|(until, _)| until) {
            self.expiry = deadline.map(|(until, remaining)| {
                let timer = cx
                    .background_executor()
                    .timer(remaining.min(std::time::Duration::from_secs(86_400)));
                (
                    until,
                    cx.spawn_in(w, async move |view, cx| {
                        timer.await;
                        let _ = view.update_in(cx, |v, w, cx| {
                            v.tiles.expiry.take();
                            v.tiles.refresh(w, cx);
                        });
                    }),
                )
            });
        }
    }

    pub(super) fn paint(&self, w: &mut Window) -> Option<String> {
        let mut error = None;
        for (key, origin) in &self.positions {
            if let Some(image) = self.leases.get(key)
                && let Err(e) =
                    image.paint(gpui_kit::Bounds::new(*origin, size(px(256.), px(256.))), w)
            {
                error.get_or_insert_with(|| format!("Tile display unavailable · {e}"));
            }
        }
        error
    }
}
