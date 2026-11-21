use super::*;
use gpui_kit::{
    base::input::{AddCursorAbove, AddCursorBelow},
    component::input::{Paste, RopeExt, Textarea, TextareaState},
};
use rsql_desktop::results::{
    cell_input::{format_cell_input, parse_cell_input},
    edits::EditSession,
};

const INLINE_BYTES: usize = 65_536;
const EXISTING_LIMIT: &str =
    "Inline editing supports up to 65,536 UTF-8 bytes. Use the SQL editor to change this value.";
const DRAFT_LIMIT: &str = "Inline editing supports up to 65,536 UTF-8 bytes. Shorten the draft or press Escape to cancel.";

const PASTE_LIMIT: &str = "Paste less text. Inline editing supports up to 65,536 UTF-8 bytes.";

pub(super) struct CellEditor {
    pub display: Position,
    retained: Position,
    pane_generation: u64,
    pub input: Entity<TextareaState>,
    access: Arc<CellAccess>,
    _blur: Subscription,
    _changes: Subscription,
}

pub(super) fn element(input: Entity<TextareaState>, grid: WeakEntity<Grid>) -> impl IntoElement {
    let paste_grid = grid.clone();
    let input_id = input.entity_id();
    div()
        .id("result-cell-editor")
        .test_support()
        .size_full()
        .key_context("RsqlCellEditor")
        // Cell text has one selection. Alt-press also starts Kit's column-drag gesture.
        .capture_any_mouse_down(|e, _, cx| {
            if e.button == MouseButton::Left && e.modifiers.alt {
                cx.stop_propagation();
            }
        })
        .capture_action(move |_: &Paste, w, cx| {
            cx.stop_propagation();
            let _ = paste_grid.update(cx, |g, cx| g.paste_inline(input_id, w, cx));
        })
        .capture_action(|_: &AddCursorAbove, _, cx| cx.stop_propagation())
        .capture_action(|_: &AddCursorBelow, _, cx| cx.stop_propagation())
        .on_mouse_down_out(move |e, w, cx| {
            let _ = grid.update(cx, |g, cx| {
                let body = g
                    .table
                    .read(cx)
                    .vertical_scroll_handle
                    .0
                    .borrow()
                    .base_handle
                    .bounds();
                let in_grid = body.contains(&e.position);
                if !g.finish_editor(true, in_grid, w, cx) && in_grid {
                    cx.stop_propagation();
                }
            });
        })
        .on_click(|_, _, cx| cx.stop_propagation())
        .child(Textarea::new(&input).h_full())
}

fn clipboard_fits(item: &ClipboardItem, mut room: usize) -> bool {
    let mut text_bytes = 0usize;
    for entry in item.entries() {
        if let ClipboardEntry::String(s) = entry {
            text_bytes = text_bytes.saturating_add(s.text().len());
            if text_bytes > room {
                return false;
            }
        }
    }
    if text_bytes != 0 {
        return true;
    }
    // ClipboardItem::text concatenates displayed paths when the string entries are empty.
    for entry in item.entries() {
        if let ClipboardEntry::ExternalPaths(paths) = entry {
            for path in &paths.0 {
                if path.as_os_str().len() > room {
                    return false;
                }
                let Some(remaining) = room.checked_sub(path.to_string_lossy().len()) else {
                    return false;
                };
                room = remaining;
            }
        }
    }
    true
}

impl Grid {
    pub(super) fn open_editor(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        if self.editor.is_some() || !self.current(cx) || !self.owner.can_edit() {
            return;
        }
        let d = self.table.read(cx).delegate();
        let Some(display) = d.selection.map(|s| s.focus) else {
            return;
        };
        let Some(retained) = d.data.retained(display) else {
            return;
        };
        if !d.data.access.allows(retained) {
            return;
        }
        let CellState::Value(value) = d.data.result.cache.cell(retained.row, retained.col) else {
            return;
        };
        let access = d.data.access.clone();
        let Some(pane_generation) = self.owner.with_result(|_, generation| generation) else {
            return;
        };
        let raw = access
            .edited
            .get(&retained)
            .map(AsRef::as_ref)
            .unwrap_or(value.raw());
        if raw.as_ref().is_some_and(|v| v.len() > INLINE_BYTES) {
            self.message = Some(EXISTING_LIMIT.into());
            cx.notify();
            return;
        }
        let initial = format_cell_input(raw);
        if initial.len() > INLINE_BYTES {
            self.message = Some(EXISTING_LIMIT.into());
            cx.notify();
            return;
        }
        self.message = None;
        let input = cx.new(|cx| TextareaState::new(w, cx).default_value(initial));
        input.update(cx, |s, cx| {
            s.focus(w, cx);
            s.select_all(w, cx);
        });
        let entity = input.entity_id();
        let blur = cx.on_blur(&input.focus_handle(cx), w, move |g, w, cx| {
            if g.editor
                .as_ref()
                .is_some_and(|e| e.input.entity_id() == entity)
            {
                g.finish_editor(true, false, w, cx);
            }
        });
        let changes = cx.observe_in(&input, w, |_, _, w, _| w.refresh());
        self.editor = Some(CellEditor {
            _changes: changes,
            _blur: blur,
            display,
            retained,
            pane_generation,
            input,
            access,
        });
        self.table.update(cx, |_, cx| cx.notify());
        cx.notify();
    }

    fn paste_inline(&mut self, input: gpui_kit::EntityId, w: &mut Window, cx: &mut Context<Self>) {
        if !self.current(cx) {
            return;
        }
        let Some(editor) = &mut self.editor else {
            return;
        };
        if editor.input.entity_id() != input
            || !editor.input.read(cx).is_editable()
            || self.owner.with_result(|v, generation| {
                generation == editor.pane_generation && v.allows(ResultAction::Edit)
            }) != Some(true)
        {
            return;
        }
        let item = crate::desktop::DesktopServices::read_clipboard(cx);
        let Some(item) = item else { return };
        let retained = editor.input.update(cx, |state, cx| {
            let text = state.text();
            let replaced = match state.marked_text_range(w, cx) {
                Some(mark) => {
                    text.offset_utf16_to_offset(mark.start)..text.offset_utf16_to_offset(mark.end)
                }
                None => state.selected_range(),
            };
            text.len() - replaced.len()
        });
        if !clipboard_fits(&item, INLINE_BYTES.saturating_sub(retained)) {
            self.message = Some(PASTE_LIMIT.into());
            cx.notify();
            return;
        }
        let Some(text) = item.text().filter(|text| !text.is_empty()) else {
            return;
        };
        // Atomic replacement lets Kit reveal the new caret against the updated text layout.
        editor
            .input
            .update(cx, |state, cx| state.replace(text, w, cx));
        if self.message.as_deref() == Some(PASTE_LIMIT) {
            self.message = None;
            cx.notify();
        }
    }

    pub(super) fn has_review_changes(&self, edit: &EditSession, cx: &App) -> bool {
        if self.staging_inline {
            return true;
        }
        if let Some(editor) = &self.editor
            && self.current(cx)
            && self.can_commit_editor(editor, cx)
        {
            let input = editor.input.read(cx);
            if input.text().len() <= INLINE_BYTES {
                return edit.has_changes_after_replacing(
                    editor.retained,
                    &parse_cell_input(&input.value()),
                );
            }
        }
        edit.counts() != (0, 0)
    }

    fn can_commit_editor(&self, editor: &CellEditor, cx: &App) -> bool {
        let data = &self.table.read(cx).delegate().data;
        self.owner.can_edit()
            && self.owner.with_result(|_, generation| generation) == Some(editor.pane_generation)
            && Arc::ptr_eq(&data.access, &editor.access)
            && editor.access.allows(editor.retained)
            && matches!(
                data.result
                    .cache
                    .cell(editor.retained.row, editor.retained.col),
                CellState::Value(_)
            )
    }

    pub(super) fn oversized_draft(&self, cx: &App) -> bool {
        self.editor
            .as_ref()
            .is_some_and(|editor| editor.input.read(cx).text().len() > INLINE_BYTES)
    }

    pub(in crate::workspace) fn finish_editor(
        &mut self,
        commit: bool,
        restore: bool,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if commit
            && let Some(editor) = &self.editor
            && editor.input.read(cx).text().len() > INLINE_BYTES
        {
            self.message = Some(DRAFT_LIMIT.into());
            if restore && self.current(cx) {
                editor.input.focus_handle(cx).focus(w, cx);
            }
            cx.notify();
            return false;
        }
        let Some(editor) = self.editor.take() else {
            return true;
        };
        self.message = None;
        if self.current(cx) {
            if commit && self.can_commit_editor(&editor, cx) {
                let value = parse_cell_input(&editor.input.read(cx).value());
                self.staging_inline = true;
                cx.emit(EditIntent {
                    prepared: None,
                    session: editor.access.session,
                    result: self.id(),
                    changes: vec![(editor.retained, value)],
                });
            }
            if restore {
                self.table.focus_handle(cx).focus(w, cx);
            }
        }
        self.table.update(cx, |_, cx| cx.notify());
        cx.notify();
        true
    }
}
