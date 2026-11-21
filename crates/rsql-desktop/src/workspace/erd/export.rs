use super::*;
use gpui_kit::component::ActiveTheme;
use rsql_desktop::{
    erd::svg::{SvgTheme, suggested_filename},
    results::export::save_replacing,
};
use std::io;
use tokio::sync::watch;
use uuid::Uuid;

fn rgb(color: Hsla) -> u32 {
    let color = color.to_rgb();
    let byte = |component: f32| (component.clamp(0., 1.) * 255.).round() as u32;
    (byte(color.r) << 16) | (byte(color.g) << 8) | byte(color.b)
}

pub(super) fn colors(cx: &App) -> SvgTheme {
    let t = cx.theme();
    SvgTheme {
        background: rgb(t.background),
        card: rgb(t.group_box),
        border: rgb(t.border),
        primary: rgb(t.primary),
        primary_foreground: rgb(t.primary_foreground),
        muted: rgb(t.muted),
        muted_foreground: rgb(t.muted_foreground),
        foreground: rgb(t.foreground),
    }
}

enum Outcome {
    Saved,
    DialogCancelled,
    Retired,
}

impl Panel {
    pub(super) fn export_svg(&mut self, cx: &mut Context<Self>) {
        if self.model.export.borrow().is_some()
            || *self.session.preflight_stop.borrow()
            || !matches!(self.capture.generation.has_changed(), Ok(false))
        {
            return;
        }
        let Some(snapshot) = self
            .model
            .owner
            .borrow()
            .as_ref()
            .and_then(DiagramOwner::capture_svg)
        else {
            return;
        };
        let theme = colors(cx);
        let id = Uuid::new_v4();
        let (cancel, mut retired) = watch::channel(());
        self.model.export.replace(Some((id, cancel)));
        let mut stop = self.session.preflight_stop.subscribe();
        let mut capture = self.capture.clone();
        let prompt =
            crate::desktop::DesktopServices::save_path(cx, &suggested_filename(&self.schema));
        self.export_message = Some("Exporting SVG…".into());
        let worker = self.session.runtime.spawn(async move {
            let path = tokio::select! {
                biased;
                _ = stop.changed() => return Ok(Outcome::Retired),
                _ = retired.changed() => return Ok(Outcome::Retired),
                _ = capture.route.retired() => return Ok(Outcome::Retired),
                _ = capture.generation.changed() => return Ok(Outcome::Retired),
                response = prompt => response.map_err(io::Error::other)?,
            };
            let Some(path) = path else {
                return Ok(Outcome::DialogCancelled);
            };
            // Hold preflight until the blocking closure and its captured graph have settled.
            let joined = stop.clone();
            let result = tokio::task::spawn_blocking(move || {
                let current = || {
                    !*stop.borrow()
                        && matches!(retired.has_changed(), Ok(false))
                        && capture.route.is_current()
                        && matches!(capture.generation.has_changed(), Ok(false))
                };
                let svg = snapshot.serialize(&theme, &current)?;
                let bytes = svg.as_bytes();
                save_replacing(&path, bytes, &current)?;
                Ok::<_, io::Error>(Outcome::Saved)
            })
            .await;
            drop(joined);
            result.map_err(io::Error::other)?
        });
        cx.spawn(async move |panel, cx| {
            let result = worker.await.map_err(io::Error::other).and_then(|v| v);
            let _ = panel.update(cx, |p, cx| {
                if p.model
                    .export
                    .borrow()
                    .as_ref()
                    .is_none_or(|job| job.0 != id)
                {
                    return;
                }
                p.model.export.take();
                p.export_message = if *p.session.preflight_stop.borrow()
                    || !p.capture.route.is_current()
                    || !matches!(p.capture.generation.has_changed(), Ok(false))
                {
                    None
                } else {
                    match result {
                        Ok(Outcome::Saved) => Some("Schema diagram exported".into()),
                        Ok(Outcome::DialogCancelled | Outcome::Retired) => None,
                        Err(error) => Some(error.to_string()),
                    }
                };
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}
