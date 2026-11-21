use super::{TerminalGeometry, TerminalView, button_code};
use crate::terminal::TermMode;
use gpui_kit::*;

#[derive(Clone, Copy)]
pub(super) struct ApplicationPress {
    pub button: MouseButton,
    pub generation: u64,
    pub geometry: TerminalGeometry,
    pub last_cell: MouseCell,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct MouseCell {
    row: usize,
    column: usize,
    code: u8,
}

impl MouseCell {
    fn encode(self, suffix: char) -> String {
        format!(
            "\x1b[<{};{};{}{suffix}",
            self.code,
            self.column + 1,
            self.row + 1
        )
    }
}

impl TerminalView {
    fn mouse_cell(&self, code: u8, position: Point<Pixels>, mods: Modifiers) -> Option<MouseCell> {
        let geometry = self
            .geometry
            .filter(|g| !g.content.is_empty())
            .or_else(|| {
                self.input
                    .pointer
                    .application_press
                    .map(|press| press.geometry)
            })?;
        let (row, column) = geometry.cell_at(geometry.clamp_position(position)?)?;
        let code = code
            | (u8::from(mods.shift) * 4)
            | (u8::from(mods.alt) * 8)
            | (u8::from(mods.control) * 16);
        Some(MouseCell { row, column, code })
    }

    pub(super) fn report_mouse(
        &mut self,
        code: u8,
        position: Point<Pixels>,
        mods: Modifiers,
        cx: &mut Context<Self>,
    ) -> Option<(u64, MouseCell)> {
        let cell = self.mouse_cell(code, position, mods)?;
        let motion = code & 32 != 0;
        if motion && self.input.pointer.last_motion == Some(cell) || !self.flush_before_input(cx) {
            return None;
        }
        let held = self.input.pointer.application_press;
        let accepted =
            match self
                .session
                .report_input(cell.encode('M').as_bytes(), |mode, generation| {
                    let button = held
                        .filter(|press| press.generation == generation)
                        .and_then(|press| button_code(press.button));
                    let tracking = if !motion {
                        TermMode::MOUSE_MODE
                    } else if button == Some(code & 3) {
                        TermMode::MOUSE_DRAG | TermMode::MOUSE_MOTION
                    } else if code == 35 {
                        TermMode::MOUSE_MOTION
                    } else {
                        return false;
                    };
                    mode.contains(TermMode::SGR_MOUSE)
                        && mode.intersects(tracking)
                        && !(code < 3 && button.is_some())
                }) {
                Ok(accepted) => accepted,
                Err(error) => {
                    self.admission(Err(error), cx);
                    return None;
                }
            };
        let generation = accepted?;
        self.input.pointer.last_motion = motion.then_some(cell);
        if motion
            && let Some(press) = &mut self.input.pointer.application_press
            && press.generation == generation
            && button_code(press.button) == Some(code & 3)
        {
            press.last_cell = MouseCell {
                code: cell.code & !32,
                ..cell
            };
        }
        Some((generation, cell))
    }

    pub(in super::super) fn release_mouse(
        &mut self,
        event: Option<&MouseUpEvent>,
        cx: &mut Context<Self>,
    ) {
        let Some(press) = self.input.pointer.application_press else {
            return;
        };
        let cell = event
            .and_then(|e| self.mouse_cell(button_code(press.button)?, e.position, e.modifiers))
            .unwrap_or(press.last_cell);
        self.input.pointer.application_press = None;
        self.pending
            .reports
            .release(cell.encode('m'), press.generation);
        self.flush(cx);
    }
}
