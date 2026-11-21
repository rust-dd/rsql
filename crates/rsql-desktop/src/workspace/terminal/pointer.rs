use super::{TerminalGeometry, TerminalView};
mod protocol;
use crate::terminal::{
    SelectionType, Side, TermMode,
    session::{CommandError, SessionPhase, ViewCommand},
};
use gpui_kit::*;
use protocol::{ApplicationPress, MouseCell};
use std::time::Duration;

#[derive(Default)]
pub(super) struct Pointer {
    position: Option<Point<Pixels>>,
    application_press: Option<ApplicationPress>,
    last_motion: Option<MouseCell>,
    edge: Option<Task<()>>,
    pub interrupted: bool,
}

impl Pointer {
    fn application_button(&self) -> Option<MouseButton> {
        self.application_press.map(|press| press.button)
    }

    pub fn holds(&self, button: MouseButton) -> bool {
        self.application_button() == Some(button)
            || (button == MouseButton::Left && self.position.is_some())
    }

    pub fn stop_selection(&mut self) {
        self.position = None;
        self.edge = None;
    }

    pub fn reset_local(&mut self) {
        self.stop_selection();
        self.last_motion = None;
    }
}

impl TerminalView {
    fn tracking(&self) -> bool {
        self.frame.phase == SessionPhase::Running && self.mode().intersects(TermMode::MOUSE_MODE)
    }

    fn hit(&self, position: Point<Pixels>) -> Option<(usize, usize, Side)> {
        let g = self.geometry()?;
        let (row, col) = g.cell_at(position)?;
        let side = if position.x < g.cell_bounds(row, col).center().x {
            Side::Left
        } else {
            Side::Right
        };
        Some((row, col, side))
    }

    fn clamped_hit(&self, position: Point<Pixels>) -> Option<(usize, usize, Side)> {
        self.hit(self.geometry()?.clamp_position(position)?)
    }

    pub(super) fn pointer_down(
        &mut self,
        e: &MouseDownEvent,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((row, col, side)) = self.hit(e.position) else {
            return;
        };
        self.input.pointer.reset_local();
        self.input.focus.focus(w, cx);
        self.sync_focus(w, cx);
        if self.tracking()
            && !(e.button == MouseButton::Left && (e.modifiers.alt || e.modifiers.shift))
        {
            if let Some(geometry) = self.geometry()
                && let Some(button) = button_code(e.button)
                && let Some((generation, last_cell)) =
                    self.report_mouse(button, e.position, e.modifiers, cx)
            {
                self.input.pointer.application_press = Some(ApplicationPress {
                    button: e.button,
                    generation,
                    geometry,
                    last_cell,
                });
            }
            cx.stop_propagation();
            return;
        }
        if e.button != MouseButton::Left {
            return;
        }
        let kind = match e.click_count {
            2 => SelectionType::Semantic,
            3.. => SelectionType::Lines,
            _ if e.modifiers.alt => SelectionType::Block,
            _ => SelectionType::Simple,
        };
        let Some(target) = self.painted_rows.as_ref().and_then(|s| s.at(row, col)) else {
            return;
        };
        self.flush(cx);
        if self.pending.clear_selection {
            self.selection_interrupted(cx);
            return;
        }
        let result = self
            .session
            .view(ViewCommand::SelectDisplayed(Some(kind), target, side));
        if self.selection_admitted(result, cx) {
            self.finish_interruption(cx);
            self.input.pointer.position = Some(e.position);
        }
        cx.stop_propagation();
    }

    pub(super) fn pointer_move(
        &mut self,
        e: &MouseMoveEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.input.pointer.position.is_some() && e.dragging() {
            self.extend(e.position, cx);
        } else if self.tracking()
            && (self.mode().contains(TermMode::MOUSE_MOTION)
                || (self.mode().contains(TermMode::MOUSE_DRAG)
                    && e.pressed_button.is_some()
                    && self.input.pointer.application_button() == e.pressed_button))
        {
            let button = e.pressed_button.and_then(button_code).unwrap_or(3);
            self.report_mouse(32 + button, e.position, e.modifiers, cx);
        }
    }

    pub(super) fn pointer_up(&mut self, e: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.input.pointer.application_button() == Some(e.button) {
            self.release_mouse(Some(e), cx);
            self.input.pointer.reset_local();
        } else if e.button == MouseButton::Left {
            self.extend(e.position, cx);
            self.input.pointer.stop_selection();
        }
    }

    pub(super) fn extend(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        if self
            .input
            .pointer
            .position
            .is_none_or(|previous| previous == position)
        {
            return;
        }
        let Some((row, col, side)) = self.clamped_hit(position) else {
            return;
        };
        let Some(target) = self.painted_rows.as_ref().and_then(|s| s.at(row, col)) else {
            self.selection_interrupted(cx);
            return;
        };
        let result = self
            .session
            .view(ViewCommand::SelectDisplayed(None, target, side));
        if !self.selection_admitted(result, cx) {
            return;
        }
        self.input.pointer.position = Some(position);
        if self.edge_delta(position) == 0 {
            self.input.pointer.edge = None;
        } else if self.input.pointer.edge.is_none() {
            self.input.pointer.edge = Some(cx.spawn(async |view, cx| {
                loop {
                    cx.background_executor()
                        .timer(Duration::from_millis(50))
                        .await;
                    if !view
                        .update(cx, |v, cx| {
                            let Some(p) = v.input.pointer.position else {
                                return false;
                            };
                            let delta = v.edge_delta(p);
                            if delta == 0 {
                                return false;
                            }
                            if v.frame.screen.as_ref().is_some_and(|s| {
                                if delta > 0 {
                                    s.scroll_offset < s.history_size
                                } else {
                                    s.scroll_offset > 0
                                }
                            }) {
                                v.scroll_gesture(p, delta, cx);
                            }
                            true
                        })
                        .unwrap_or(false)
                    {
                        break;
                    }
                }
            }));
        }
    }

    fn edge_delta(&self, p: Point<Pixels>) -> i32 {
        let Some(g) = self.geometry() else {
            return 0;
        };
        if self
            .frame
            .screen
            .as_ref()
            .is_none_or(|s| s.mode.contains(TermMode::ALT_SCREEN) || s.history_size == 0)
        {
            return 0;
        }
        let above = (g.content.top() - p.y).max(px(0.));
        let below = (p.y - g.content.bottom()).max(px(0.));
        let distance = f32::from(above - below);
        (distance.signum() * ((distance.abs() / 50.).min(1.) * 15.).ceil()) as i32
    }

    fn scroll_gesture(
        &mut self,
        position: Point<Pixels>,
        delta: i32,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.input.pointer.position.is_none() || delta == 0 {
            return false;
        }
        let Some((row, col, side)) = self.clamped_hit(position) else {
            return false;
        };
        self.flush(cx);
        let result = if self.pending.scroll.is_pending() {
            Err(CommandError::Busy)
        } else {
            let Some(point) = self.painted_rows.as_ref().and_then(|s| s.at(row, col)) else {
                self.selection_interrupted(cx);
                return true;
            };
            self.session
                .view(ViewCommand::ScrollDisplayed(delta, row, point, side))
        };
        self.selection_admitted(result, cx);
        true
    }

    pub(super) fn select_all(&mut self, cx: &mut Context<Self>) {
        self.input.pointer.stop_selection();
        self.flush(cx);
        if self.pending.clear_selection {
            self.selection_interrupted(cx);
            return;
        }
        let result = self.session.view(ViewCommand::SelectAll);
        if self.selection_admitted(result, cx) {
            self.finish_interruption(cx);
        }
    }

    fn finish_interruption(&mut self, cx: &mut Context<Self>) {
        if std::mem::take(&mut self.input.pointer.interrupted) {
            self.input.notice = None;
            cx.notify();
        }
    }

    fn selection_admitted(
        &mut self,
        result: Result<(), CommandError>,
        cx: &mut Context<Self>,
    ) -> bool {
        if result == Err(CommandError::Busy) {
            self.selection_interrupted(cx);
            false
        } else {
            self.admission(result, cx)
        }
    }

    pub(super) fn selection_interrupted(&mut self, cx: &mut Context<Self>) {
        self.input.pointer.stop_selection();
        if !self.input.pointer.interrupted {
            self.input.pointer.interrupted = true;
            self.pending.clear_selection = true;
        }
        self.input.notice = Some("Terminal was busy. Select the text again.");
        self.flush(cx);
        cx.notify();
    }

    pub(super) fn wheel(&mut self, e: &ScrollEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(g) = self.geometry() else {
            return;
        };
        let Some(screen) = &self.frame.screen else {
            return;
        };
        let tracking = self.tracking()
            && self.input.pointer.position.is_none()
            && !(e.modifiers.alt || e.modifiers.shift);
        let alternate = screen.mode.contains(TermMode::ALT_SCREEN);
        if !tracking && !alternate && screen.history_size == 0 {
            return;
        }
        let delta = match e.delta {
            ScrollDelta::Lines(p) => p.y,
            ScrollDelta::Pixels(p) => p.y / g.cell.height,
        };
        if !delta.is_finite() || delta == 0. {
            return;
        }
        self.fractional_scroll += delta;
        let whole = self.fractional_scroll.trunc();
        self.fractional_scroll -= whole;
        cx.stop_propagation();
        if whole == 0. {
            return;
        }
        if tracking {
            self.report_mouse(
                if whole > 0. { 64 } else { 65 },
                e.position,
                e.modifiers,
                cx,
            );
        } else if alternate {
            if !self.tracking()
                && self.frame.phase == SessionPhase::Running
                && self.input.pointer.position.is_none()
            {
                self.apply_edit(
                    crate::terminal::session::Edit::AlternateScroll { up: whole > 0. },
                    cx,
                );
            }
        } else if !self.scroll_gesture(e.position, whole as i32, cx) {
            self.pending.scroll.push(whole as i32);
            self.flush(cx);
        }
    }
}

fn button_code(button: MouseButton) -> Option<u8> {
    match button {
        MouseButton::Left => Some(0),
        MouseButton::Middle => Some(1),
        MouseButton::Right => Some(2),
        _ => None,
    }
}
