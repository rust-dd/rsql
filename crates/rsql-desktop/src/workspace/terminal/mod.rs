//! Retained native terminal pane. Its tab owner controls visibility and joins the session at close.

mod composition;
mod element;
mod geometry;
mod input;
mod pointer;
mod reports;
mod scroll;
mod style;

pub use geometry::TerminalGeometry;

use crate::terminal::{
    CursorShape, Scroll, TermMode, TerminalTheme,
    session::{CommandError, SessionPhase, SessionUpdate, TerminalSession, ViewCommand, Viewport},
};
use element::TerminalPaint;
use gpui_kit::{base::TestSupportExt, component::ActiveTheme, *};
use std::{sync::Arc, time::Duration};

#[derive(Default)]
struct Pending {
    theme: Option<TerminalTheme>,
    pixels: Option<(u16, u16)>,
    scroll: scroll::PendingScroll,
    clear_selection: bool,
    reports: reports::PendingReports,
}

pub struct TerminalView {
    session: TerminalSession,
    input: input::Input,
    frame: Arc<SessionUpdate>,
    theme: TerminalTheme,
    visible: bool,
    window_visible: bool,
    window_active: bool,
    keyboard_focus: bool,
    effective_focus: bool,
    painted_rows: Option<Arc<crate::terminal::DisplayedRows>>,
    geometry: Option<TerminalGeometry>,
    pending: Pending,
    retry: Option<Task<()>>,
    fractional_scroll: f32,
    blink: Option<Task<()>>,
    cursor_on: bool,
    _updates: Task<()>,
    _visibility: Subscription,
    _activation: Subscription,
}

impl TerminalView {
    pub fn new(
        session: TerminalSession,
        theme: TerminalTheme,
        visible: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut frames = session.updates();
        let frame = frames.current();
        let updates = cx.spawn(async move |view, cx| {
            while let Ok(frame) = frames.changed().await {
                if view.update(cx, |v, cx| v.receive(frame, cx)).is_err() {
                    break;
                }
            }
        });
        let visibility = cx.observe_window_visibility(window, |v, visibility, _, cx| {
            v.window_visible = visibility.is_visible();
            v.visibility_changed(cx);
        });
        let activation = cx.observe_window_activation(window, |v, window, cx| {
            v.sync_focus(window, cx);
            if !window.is_window_active() {
                v.release_mouse(None, cx);
                v.input.reset();
            }
            if v.shown() {
                cx.notify();
            }
        });
        let mut view = Self {
            session,
            input: input::Input::new(window, cx),
            frame,
            theme,
            visible,
            window_visible: window.is_visible(),
            window_active: window.is_window_active(),
            keyboard_focus: false,
            effective_focus: false,
            painted_rows: None,
            geometry: None,
            pending: Pending {
                theme: Some(theme),
                ..Default::default()
            },
            retry: None,
            fractional_scroll: 0.,
            blink: None,
            cursor_on: true,
            _updates: updates,
            _visibility: visibility,
            _activation: activation,
        };
        view.flush(cx);
        view
    }

    pub fn session(&self) -> &TerminalSession {
        &self.session
    }
    pub fn frame(&self) -> &Arc<SessionUpdate> {
        &self.frame
    }
    /// Geometry of the most recently laid out, visible pane for pointer and IME positioning.
    pub fn geometry(&self) -> Option<TerminalGeometry> {
        self.geometry.filter(|_| self.shown())
    }

    pub fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if self.visible != visible {
            self.visible = visible;
            self.visibility_changed(cx);
        }
    }

    pub fn set_theme(&mut self, theme: TerminalTheme, cx: &mut Context<Self>) {
        if self.theme != theme {
            self.theme = theme;
            self.pending.theme = Some(theme);
            self.flush(cx);
            if self.shown() {
                cx.notify();
            }
        }
    }

    fn shown(&self) -> bool {
        self.visible && self.window_visible
    }

    fn sync_focus(&mut self, window: &Window, cx: &mut Context<Self>) {
        self.window_active = window.is_window_active();
        self.keyboard_focus = self.input.focus.is_focused(window);
        self.update_focus(cx);
    }

    fn update_focus(&mut self, cx: &mut Context<Self>) {
        let focused = self.shown() && self.window_active && self.keyboard_focus;
        if focused != self.effective_focus {
            self.effective_focus = focused;
            if !focused {
                self.blink = None;
                self.cursor_on = true;
            }
            if self.mode().contains(TermMode::FOCUS_IN_OUT) {
                let bytes = if focused { b"\x1b[I" } else { b"\x1b[O" };
                self.pending.reports.focus(bytes);
                self.flush(cx);
            }
            cx.notify();
        }
    }

    fn visibility_changed(&mut self, cx: &mut Context<Self>) {
        self.update_focus(cx);
        if !self.shown() {
            self.release_mouse(None, cx);
            self.input.reset();
            self.input.copy = None;
            self.blink = None;
            self.cursor_on = true;
        }
        cx.notify();
    }

    fn receive(&mut self, frame: Arc<SessionUpdate>, cx: &mut Context<Self>) {
        if self.session.accepts(&frame) {
            if self
                .frame
                .screen
                .as_ref()
                .is_some_and(|s| s.mode.contains(TermMode::ALT_SCREEN))
                != frame
                    .screen
                    .as_ref()
                    .is_some_and(|s| s.mode.contains(TermMode::ALT_SCREEN))
            {
                self.fractional_scroll = 0.;
            }
            if !matches!(frame.phase, SessionPhase::Running | SessionPhase::Starting) {
                self.input.composition = Default::default();
            }
            self.frame = frame;
            if self.pending.scroll.is_pending() {
                self.flush(cx);
            }
        } else if self.session.owner() == &frame.owner {
            self.blink = None;
        } else {
            return;
        }
        if self.shown() {
            cx.notify();
        }
    }

    fn layout(
        &mut self,
        geometry: TerminalGeometry,
        clip: Bounds<Pixels>,
        cx: &mut Context<Self>,
    ) -> CursorShape {
        if self.geometry.map(|g| (g.columns, g.rows)) != Some((geometry.columns, geometry.rows)) {
            let size =
                Viewport::new(geometry.columns, geometry.rows).expect("clamped terminal viewport");
            let _ = self.session.resize(size);
        }
        if self.geometry.map(|g| g.cell) != Some(geometry.cell) {
            self.pending.pixels = Some((
                f32::from(geometry.cell.width).round() as u16,
                f32::from(geometry.cell.height).round() as u16,
            ));
            self.flush(cx);
        }
        self.geometry = Some(geometry);
        let clipped = geometry.content.intersect(&clip);
        let running =
            self.frame.phase == SessionPhase::Running && self.session.accepts(&self.frame);
        let blinking = !clipped.is_empty()
            && self.effective_focus
            && self.shown()
            && running
            && self.frame.screen.as_ref().is_some_and(|s| {
                s.cursor.blinking
                    && geometry
                        .cursor_bounds(s, s.cursor.shape)
                        .is_some_and(|bounds| bounds.intersects(&clipped))
            });
        if blinking && self.blink.is_none() {
            self.blink = Some(cx.spawn(async |view, cx| {
                loop {
                    cx.background_executor()
                        .timer(Duration::from_millis(600))
                        .await;
                    if view
                        .update(cx, |v, cx| {
                            v.cursor_on = !v.cursor_on;
                            cx.notify();
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            }));
        } else if !blinking {
            self.blink = None;
            self.cursor_on = true;
        }
        if !self.cursor_on || !running {
            CursorShape::Hidden
        } else if !self.effective_focus {
            CursorShape::HollowBlock
        } else {
            self.frame
                .screen
                .as_ref()
                .map_or(CursorShape::Hidden, |s| s.cursor.shape)
        }
    }

    fn flush(&mut self, cx: &mut Context<Self>) {
        let submit = |command| self.session.view(command) != Err(CommandError::Busy);
        if self.pending.clear_selection && submit(ViewCommand::ClearSelection) {
            self.pending.clear_selection = false;
        }
        self.pending.reports.flush(&self.session);
        if let Some(theme) = self.pending.theme
            && submit(ViewCommand::Theme(theme))
        {
            self.pending.theme = None;
        }
        if let Some((w, h)) = self.pending.pixels
            && submit(ViewCommand::CellSize(w, h))
        {
            self.pending.pixels = None;
        }
        let primary = self
            .frame
            .screen
            .as_ref()
            .is_some_and(|s| !s.mode.contains(TermMode::ALT_SCREEN));
        let primary = primary && !self.mode().contains(TermMode::ALT_SCREEN);
        if primary {
            self.pending
                .scroll
                .drain(|d| self.session.view(ViewCommand::Scroll(Scroll::Delta(d))));
        }
        if self.retry.is_none()
            && (self.pending.clear_selection
                || self.pending.reports.is_pending()
                || self.pending.theme.is_some()
                || self.pending.pixels.is_some()
                || (primary && self.pending.scroll.is_pending()))
        {
            self.retry = Some(cx.spawn(async |view, cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(16))
                    .await;
                let _ = view.update(cx, |v, cx| {
                    v.retry = None;
                    v.flush(cx);
                });
            }));
        }
    }
}

impl Render for TerminalView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let pane = div()
            .id("terminal-viewport")
            .test_support()
            .flex()
            .flex_col()
            .size_full()
            .min_h_0()
            .min_w_0()
            .overflow_hidden();
        if !self.shown() {
            return pane;
        }
        let base = font(cx.theme().mono_font_family.clone());
        let font_size = cx.theme().mono_font_size;
        let width =
            cx.text_system()
                .layout_width(cx.text_system().resolve_font(&base), font_size, '0');
        let frame = &self.frame;
        let backdrop = frame.screen.as_deref().map_or_else(
            || style::hsla_from_rgb(self.theme.default_background()),
            style::background,
        );
        let selection = style::selection(self.theme);
        let owner = cx.entity();
        let screen = frame.screen.clone();
        let displayed = frame.displayed.clone();
        let editor = owner.clone();
        let focus = self.input.focus.clone();
        let composing = !self.input.composition.is_empty();
        let surface = canvas(
            move |bounds, window, cx| {
                let geometry = TerminalGeometry::fit(bounds, size(width, font_size * 1.3));
                let cursor_shape = owner.update(cx, |view, cx| {
                    view.painted_rows = displayed;
                    view.sync_focus(window, cx);
                    view.layout(geometry, window.content_mask().bounds, cx)
                });
                (geometry, cursor_shape)
            },
            move |bounds, (geometry, cursor_shape), window, cx| {
                let drag = editor.clone();
                window.on_mouse_event(move |e: &MouseMoveEvent, phase, w, cx| {
                    if phase == DispatchPhase::Capture && e.pressed_button.is_some() {
                        drag.update(cx, |v, cx| {
                            if e.pressed_button
                                .is_some_and(|button| v.input.pointer.holds(button))
                                && v.geometry()
                                    .is_some_and(|g| !g.content.contains(&e.position))
                            {
                                v.pointer_move(e, w, cx);
                            }
                        });
                    }
                });
                window.handle_input(&focus, ElementInputHandler::new(bounds, editor.clone()), cx);
                if let Some(screen) = screen {
                    TerminalPaint {
                        screen: &screen,
                        font: base,
                        font_size,
                        selection,
                        cursor_shape,
                    }
                    .paint(geometry, bounds, window, cx);
                    if composing {
                        editor.update(cx, |view, cx| {
                            if view.can_type(window) {
                                view.input
                                    .composition
                                    .paint(geometry, &screen, selection, window, cx);
                            }
                        });
                    }
                }
            },
        )
        .flex_1()
        .min_h_0()
        .w_full();
        let mut pane = self
            .input_events(pane, cx)
            .bg(backdrop)
            .on_scroll(cx.listener(Self::wheel))
            .child(surface);
        for (id, notice) in [
            ("terminal-notice", frame.phase.notice()),
            ("terminal-input-notice", self.input.notice),
        ] {
            let Some(notice) = notice else {
                continue;
            };
            pane = pane.child(
                div()
                    .id(id)
                    .test_support()
                    .px(px(8.))
                    .text_size(font_size)
                    .text_color(cx.theme().muted_foreground)
                    .child(notice.to_owned()),
            );
        }
        pane
    }
}
