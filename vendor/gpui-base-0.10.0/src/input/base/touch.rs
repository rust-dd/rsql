//! Touch selection for the input engine: long press, grab handles, edit menu.
//!
//! A long press selects the word under the finger and keeps following the
//! finger while it stays down, like the double-click-and-drag it stands in
//! for. Releasing opens the edit menu; from then on the selection carries a
//! handle at each end which the styled layer draws and which drags through
//! [`InputBaseState::begin_edge_drag`].
//!
//! The touch selection remembers the range it made. As soon as the selection
//! is something else — the caret moved, text was typed, another cursor was
//! added — the handles and the menu are gone, without anyone having to hide
//! them. Only the touch gesture and the menu's own actions move the range and
//! carry the touch selection along.

use gpui::{Context, LongPressEvent, Pixels, Point, TouchPhase, Window, point};

use super::{InputBaseState, InputModeKind};
use crate::touch_selection::{
    EdgeDrag, SelectionEdge, TouchSelectionSnapshot, caret_in_view, caret_line_box,
};

/// The selection a touch gesture made, and what it is doing now.
#[derive(Debug, Default)]
pub(super) struct TouchSelection {
    /// The range the gesture left; `None` when no touch selection is live.
    range: Option<(usize, usize)>,
    menu_open: bool,
    drag: Option<EdgeDrag>,
}

impl<M: InputModeKind> InputBaseState<M> {
    /// The live touch selection, laid out for the handles and the edit menu.
    ///
    /// `None` when the last touch selection has since changed into something
    /// else, or when its ends are not laid out (scrolled out of view).
    pub fn touch_selection(&self) -> Option<TouchSelectionSnapshot> {
        let range = self.touch_selection.range?;
        let selection = self.active_selection();
        if (selection.start, selection.end) != range {
            return None;
        }

        let layout = self.last_layout.as_ref()?;
        let line_height = layout.line_height;
        let laid_out = layout.visible_range_offset.clone();
        let origin = self.last_bounds?.origin;
        let viewport = self.input_bounds;
        // An end scrolled out of the input gets no handle. Its line may not
        // even be laid out; it then stands just outside the viewport on its
        // side, which is all the other end's drag needs to know about it.
        let caret_box = |offset: usize, stand_in_y: Pixels| {
            let (_, _, position) = self.line_and_position_for_offset(offset);
            match position.filter(|_| laid_out.contains(&offset) || laid_out.end == offset) {
                Some(position) => caret_line_box(origin + position, line_height),
                None => caret_line_box(point(viewport.left(), stand_in_y), line_height),
            }
        };
        let start = caret_box(range.0, viewport.top() - line_height);
        let end = caret_box(range.1, viewport.bottom());
        Some(
            TouchSelectionSnapshot::new(start, end)
                .with_edge_visible(SelectionEdge::Start, caret_in_view(start, viewport))
                .with_edge_visible(SelectionEdge::End, caret_in_view(end, viewport))
                .with_menu_open(self.touch_selection.menu_open)
                .with_dragging(self.touch_selection.drag.map(|drag| drag.edge())),
        )
    }

    /// Records the current selection as the one the touch gesture made.
    fn retain_touch_selection(&mut self) {
        let selection = self.active_selection();
        self.touch_selection.range = Some((selection.start, selection.end));
    }

    /// Keeps the current selection as a touch selection, with the menu open
    /// over it: what a double tap's word selection asks for.
    pub(super) fn keep_touch_selection(&mut self, cx: &mut Context<Self>) {
        self.retain_touch_selection();
        self.touch_selection.menu_open = true;
        self.touch_selection.drag = None;
        cx.notify();
    }

    /// Drops the handles and the edit menu. Called where the selection is
    /// about to be moved by something other than the gesture.
    pub(super) fn dismiss_touch_selection(&mut self, cx: &mut Context<Self>) {
        if self.touch_selection.range.is_none() {
            return;
        }
        self.touch_selection = TouchSelection::default();
        cx.notify();
    }

    /// Closes the edit menu and keeps the selection with its handles.
    ///
    /// The menu's own actions end here: Copy has done its work, Select All
    /// reopens the menu over the new range through [`Self::select_all`].
    pub fn close_edit_menu(&mut self, cx: &mut Context<Self>) {
        if !self.touch_selection.menu_open {
            return;
        }
        self.touch_selection.menu_open = false;
        cx.notify();
    }

    /// Reopens the edit menu when `position` is on the touch selection, the
    /// way a tap on selected text asks for it. Returns whether it did.
    pub(super) fn reopen_edit_menu_at(
        &mut self,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(snapshot) = self.touch_selection() else {
            return false;
        };
        if snapshot.is_empty() {
            return false;
        }
        let (offset, _, _) = self.resolve_mouse_position(position);
        let selection = self.active_selection();
        if offset <= selection.start || offset >= selection.end {
            return false;
        }
        self.touch_selection.menu_open = true;
        cx.notify();
        true
    }

    /// Steps the menu aside while the content scrolls under a finger, and
    /// brings it back over the handles once the finger lifts.
    pub(super) fn edit_menu_on_scroll(&mut self, phase: TouchPhase, cx: &mut Context<Self>) {
        if self.touch_selection.range.is_none() {
            return;
        }
        match phase {
            TouchPhase::Ended | TouchPhase::Cancelled => {
                if !self.touch_selection.menu_open {
                    self.touch_selection.menu_open = true;
                    cx.notify();
                }
            }
            _ => self.close_edit_menu(cx),
        }
    }

    /// Selects everything and, when a touch selection is live, keeps the
    /// handles and the edit menu over the new range.
    pub fn select_all_from_edit_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let touch = self.touch_selection.range.is_some();
        self.select_all(window, cx);
        if touch {
            self.retain_touch_selection();
            self.touch_selection.menu_open = true;
            cx.notify();
        }
    }

    /// Handles one phase of a long press inside the input.
    ///
    /// Returns whether the started phase was claimed; the caller then captures
    /// the gesture so the moves keep coming even after the finger leaves the
    /// input.
    pub(super) fn on_long_press(
        &mut self,
        event: &LongPressEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        match event.phase {
            TouchPhase::Started => {
                if self.disabled {
                    return false;
                }
                if !self.focus_handle.is_focused(window) {
                    window.focus(&self.focus_handle, cx);
                }
                // The input selects on its own; keep the window text selection
                // from starting a drag under it.
                crate::GlobalState::suppress_text_selection(cx);
                self.undo_manager.break_transaction_coalescing();
                M::clear_inline_completion(self, cx);
                self.touch_selection = TouchSelection::default();

                let (offset, line_end_affinity, _) =
                    self.resolve_mouse_position(event.start_position);
                self.selections.remove_all_but_active();
                self.set_cursor_to(offset);
                self.select_word(offset, window, cx);
                let pressed_word = !self.active_selection().is_empty()
                    && !self.selected_text().chars().all(char::is_whitespace);
                if !pressed_word {
                    // Whitespace or an empty field: the press places the caret,
                    // and the menu offers Paste and Select All.
                    self.move_to_with_affinity(offset, None, line_end_affinity, cx);
                    self.selected_word_range = None;
                }
                self.selecting = true;
                self.retain_touch_selection();
                cx.notify();
                true
            }
            TouchPhase::Moved => {
                let (offset, line_end_affinity, _) = self.resolve_mouse_position(event.position);
                if self.selected_word_range.is_some() {
                    // The press took a word; the sweep grows it a word at a time.
                    self.select_to_with_affinity(offset, line_end_affinity, cx);
                } else {
                    // The press placed the caret; the sweep carries it.
                    self.move_to_with_affinity(offset, None, line_end_affinity, cx);
                }
                self.retain_touch_selection();
                true
            }
            TouchPhase::Ended | TouchPhase::Cancelled => {
                self.selecting = false;
                self.selected_word_range = None;
                if self.touch_selection.range.is_some() {
                    self.touch_selection.menu_open = true;
                }
                cx.notify();
                true
            }
        }
    }

    /// Starts dragging one end of the touch selection from `finger`.
    ///
    /// The other end stays where it is. The menu closes for the duration of the
    /// drag and reopens on [`Self::end_edge_drag`].
    pub fn begin_edge_drag(
        &mut self,
        edge: SelectionEdge,
        finger: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let Some(snapshot) = self.touch_selection() else {
            return;
        };
        self.undo_manager.break_transaction_coalescing();
        self.selected_word_range = None;
        self.active_selection_mut().reversed = edge == SelectionEdge::Start;
        self.touch_selection.drag = Some(EdgeDrag::begin(edge, snapshot.edge(edge), finger));
        self.touch_selection.menu_open = false;
        cx.notify();
    }

    /// Moves the dragged end to the text under `finger`.
    ///
    /// Past the top or bottom of a multi-line input the content scrolls under
    /// the finger, so the end can reach text that was out of view.
    pub fn update_edge_drag(&mut self, finger: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(drag) = self.touch_selection.drag else {
            return;
        };
        let position = drag.text_position(finger);
        self.extend_edge_drag_to(position, cx);

        if self.is_single_line() {
            return;
        }
        self.auto_scroll.last_drag_position = Some(position);
        let delta = crate::AutoScroll::compute_delta(position.y, self.input_bounds);
        // Input's ScrollHandle uses negative-y-is-down; negate the positive-towards-bottom delta.
        let scroll_delta = delta.map(|delta| -delta);
        self.auto_scroll.set(scroll_delta, cx, |delta, state, cx| {
            let current = state.scroll_handle.offset();
            state.update_scroll_offset(Some(point(current.x, current.y + delta)), cx);
            if let Some(position) = state.auto_scroll.last_drag_position {
                state.extend_edge_drag_to(position, cx);
            }
        });
    }

    fn extend_edge_drag_to(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        if self.touch_selection.drag.is_none() {
            return;
        }
        let (offset, line_end_affinity, _) = self.resolve_mouse_position(position);
        let before = *self.active_selection();
        self.select_to_with_affinity(offset, line_end_affinity, cx);
        // A handle never collapses the selection: at the other end it stops,
        // and the finger has to pass that end to swap the two.
        if self.active_selection().is_empty() {
            *self.active_selection_mut() = before;
            return;
        }
        // Dragging one end past the other swaps them: the selection now runs
        // the other way and the finger holds what became the other handle.
        let edge = if self.active_selection().reversed {
            SelectionEdge::Start
        } else {
            SelectionEdge::End
        };
        if let Some(drag) = self.touch_selection.drag.as_mut() {
            drag.set_edge(edge);
        }
        self.retain_touch_selection();
        cx.notify();
    }

    /// Ends the handle drag and reopens the edit menu.
    pub fn end_edge_drag(&mut self, cx: &mut Context<Self>) {
        if self.touch_selection.drag.take().is_none() {
            return;
        }
        self.auto_scroll.stop();
        if self.active_selection().is_empty() {
            self.active_selection_mut().reversed = false;
        }
        self.touch_selection.menu_open = true;
        cx.notify();
    }
}
