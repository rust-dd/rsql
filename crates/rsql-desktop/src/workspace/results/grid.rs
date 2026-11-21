use super::*;
use super::{cell_editor::CellEditor, delegate::Delegate};
use gpui_kit::component::{
    Sizable, Size,
    input::{Copy, Paste},
    table::{DataTable, TableEvent},
};

#[derive(Clone, PartialEq, Action)]
#[action(namespace = rsql, no_json)]
struct GridKey {
    key: &'static str,
    shift: bool,
}

pub(crate) fn bind_keys(cx: &mut App) {
    super::export_menu::bind_keys(cx);
    let table = Some("RsqlResultGrid > DataTable");
    for (key, shifted) in [
        ("left", "shift-left"),
        ("right", "shift-right"),
        ("up", "shift-up"),
        ("down", "shift-down"),
        ("home", "shift-home"),
        ("end", "shift-end"),
        ("pageup", "shift-pageup"),
        ("pagedown", "shift-pagedown"),
        ("tab", "shift-tab"),
        ("enter", "shift-enter"),
        ("escape", "shift-escape"),
        ("space", "shift-space"),
    ] {
        for (stroke, shift) in [(key, false), (shifted, true)] {
            cx.bind_keys([KeyBinding::new(stroke, GridKey { key, shift }, table)]);
            if matches!(key, "tab" | "escape" | "enter") && !(key == "enter" && shift) {
                cx.bind_keys([KeyBinding::new(
                    stroke,
                    GridKey { key, shift },
                    Some("RsqlCellEditor > Input"),
                )]);
            }
        }
    }
    let [copy, paste] = if cfg!(target_os = "macos") {
        ["cmd-c", "cmd-v"]
    } else {
        ["ctrl-c", "ctrl-v"]
    };
    cx.bind_keys([
        KeyBinding::new(copy, Copy, table),
        KeyBinding::new(paste, Paste, table),
    ]);
}

pub(in crate::workspace) struct Grid {
    pub(super) table: Entity<TableState<Delegate>>,
    pub(super) owner: GridOwner,
    pub(super) editor: Option<CellEditor>,
    pub(super) staging_inline: bool,
    pub(super) message: Option<String>,
    pub(super) clipboard: Option<super::clipboard::Work>,
    active: bool,
    dragging: bool,
    keyboard_row_events: std::collections::VecDeque<usize>,
    _events: Subscription,
}

impl EventEmitter<super::edit_actions::GridAction> for Grid {}
impl EventEmitter<EditIntent> for Grid {}
impl EventEmitter<Option<Selection>> for Grid {}

impl Grid {
    pub(in crate::workspace) fn new(
        session: Rc<Session>,
        pane: Owner,
        view: &ResultView,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let owner = GridOwner {
            session,
            pane,
            result: view.data.id,
        };
        let data = GridData {
            result: view.data.clone(),
            filtered: view.filtered_rows.clone(),
            access: view.cell_access.clone(),
        };
        let delegate = Delegate::new(owner.clone(), data, cx.weak_entity());
        let table = cx.new(|cx| {
            TableState::new(delegate, w, cx)
                .cell_selectable(true)
                .row_selectable(true)
                .col_selectable(false)
                .col_movable(false)
                .sortable(false)
        });
        let events = cx.subscribe_in(&table, w, |g, table, event, w, cx| match event {
            TableEvent::ColumnWidthsChanged(widths) => {
                table.update(cx, |t, _| t.delegate_mut().resized(widths))
            }
            TableEvent::SelectRow(row) => {
                if g.keyboard_row_events.front() == Some(row) {
                    g.keyboard_row_events.pop_front();
                    return;
                }
                if !g.current(cx) {
                    return;
                }
                let old = g.selection(cx);
                table.update(cx, |t, cx| {
                    let pos = Position { row: *row, col: 0 };
                    let shift = t.delegate_mut().row_press.take() == Some((*row, true));
                    let s = t
                        .delegate_mut()
                        .selection
                        .get_or_insert_with(Default::default);
                    s.select(pos, shift && old.is_some(), true);
                    cx.notify();
                });
                g.publish_selection(old, cx);
                table.focus_handle(cx).focus(w, cx);
            }
            TableEvent::DoubleClickedCell(..) => g.open_editor(w, cx),
            TableEvent::ClearSelection => {
                let old = g.selection(cx);
                table.update(cx, |t, cx| {
                    t.delegate_mut().selection = None;
                    cx.notify();
                });
                g.publish_selection(old, cx);
            }
            _ => {}
        });
        Self {
            table,
            owner,
            editor: None,
            staging_inline: false,
            clipboard: None,
            message: None,
            dragging: false,
            keyboard_row_events: Default::default(),
            active: true,
            _events: events,
        }
    }

    pub(in crate::workspace) fn selection(&self, cx: &App) -> Option<Selection> {
        self.table.read(cx).delegate().selection
    }

    fn publish_selection(&self, old: Option<Selection>, cx: &mut Context<Self>) {
        let selection = self.selection(cx);
        if selection != old {
            cx.emit(selection);
            cx.notify();
        }
    }

    pub(in crate::workspace) fn id(&self) -> Uuid {
        self.owner.result
    }
    pub(in crate::workspace) fn sync(
        &mut self,
        view: &ResultView,
        active: bool,
        cx: &mut Context<Self>,
    ) {
        let current = &self.table.read(cx).delegate().data.filtered;
        let changed = match (current, &view.filtered_rows) {
            (Some(a), Some(b)) => !Arc::ptr_eq(a, b),
            (None, None) => false,
            _ => true,
        };
        let access_changed = !Arc::ptr_eq(
            &view.cell_access,
            &self.table.read(cx).delegate().data.access,
        );
        if (self.active && !active) || changed || access_changed {
            self.clipboard = None;
            if changed || access_changed || !self.oversized_draft(cx) {
                self.editor = None;
            }
            self.dragging = false;
        }
        self.active = active;
        if active {
            let old = self.selection(cx);
            self.table.update(cx, |table, cx| {
                if access_changed {
                    table.delegate_mut().data.access = view.cell_access.clone();
                    cx.notify();
                }
                if changed {
                    table.delegate_mut().data.filtered = view.filtered_rows.clone();
                    table.delegate_mut().selection = None;
                    table.clear_selection(cx);
                    table.scroll_to_row(0, cx);
                }
                if changed || table.delegate().needs_widths() {
                    // The resize handle updates Kit's column groups before it delivers
                    // ColumnWidthsChanged. Capture those widths before rebuilding the groups.
                    let widths = (0..table.delegate().columns_count(cx))
                        .map_while(|i| table.col_width(i))
                        .collect::<Vec<_>>();
                    table.delegate_mut().resized(&widths);
                    table.delegate_mut().sync_widths();
                    table.refresh(cx);
                }
            });
            self.publish_selection(old, cx);
        }
    }

    pub(super) fn current(&self, cx: &App) -> bool {
        self.active
            && self
                .owner
                .with_view(|v| {
                    Arc::ptr_eq(&v.cell_access, &self.table.read(cx).delegate().data.access)
                        && match (
                            &v.filtered_rows,
                            &self.table.read(cx).delegate().data.filtered,
                        ) {
                            (None, None) => true,
                            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                            _ => false,
                        }
                })
                .unwrap_or(false)
    }

    pub(super) fn pointer(
        &mut self,
        pos: Position,
        extend: bool,
        drag: bool,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.current(cx) || (drag && !self.dragging) {
            return;
        }
        if self.editor.is_some() && !self.finish_editor(true, true, w, cx) {
            return;
        }
        self.dragging = true;
        let old = self.selection(cx);
        self.table.update(cx, |t, cx| {
            let s = t
                .delegate_mut()
                .selection
                .get_or_insert_with(Default::default);
            if drag && s.focus == pos {
                return;
            }
            s.select(pos, extend && old.is_some(), false);
            if t.selected_cell() != Some((pos.row, pos.col)) {
                t.set_selected_cell(pos.row, pos.col, cx);
            } else {
                cx.notify();
            }
        });
        self.publish_selection(old, cx);
        self.table.focus_handle(cx).focus(w, cx);
    }

    pub(super) fn navigate(&mut self, key: &str, shift: bool, cx: &mut Context<Self>) {
        let old = self.selection(cx);
        self.table.update(cx, |t, cx| {
            let rows = t.delegate().rows_count(cx);
            let cols = t.delegate().columns_count(cx);
            let page = t.visible_range().rows().len().saturating_sub(1);
            let mut selection = old.unwrap_or_default();
            if selection.navigate(key, shift, rows, cols, page) {
                t.delegate_mut().selection = Some(selection);
                let pos = selection.focus;
                if selection.rows {
                    self.keyboard_row_events.push_back(pos.row);
                    t.set_selected_row(pos.row, cx);
                } else {
                    t.set_selected_cell(pos.row, pos.col, cx);
                }
            }
        });
        self.publish_selection(old, cx);
    }

    fn key(&mut self, key: &GridKey, w: &mut Window, cx: &mut Context<Self>) {
        if !self.current(cx) {
            return;
        }
        if let Some(editor) = &self.editor {
            if !matches!(key.key, "tab" | "escape" | "enter")
                || !editor.input.focus_handle(cx).is_focused(w)
                || editor
                    .input
                    .update(cx, |s, cx| s.marked_text_range(w, cx).is_some())
            {
                cx.propagate();
                return;
            }
            if !self.finish_editor(key.key != "escape", true, w, cx) {
                return;
            }
            match key.key {
                "enter" => self.navigate("down", false, cx),
                "tab" => self.navigate(if key.shift { "left" } else { "right" }, false, cx),
                _ => {}
            }
        } else if self.table.focus_handle(cx).is_focused(w) {
            match key.key {
                "enter" => self.open_editor(w, cx),
                "escape" => self.table.update(cx, |t, cx| t.clear_selection(cx)),
                _ => self.navigate(key.key, key.shift, cx),
            }
        }
    }
}

impl Render for Grid {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .key_context("RsqlResultGrid")
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .on_action(cx.listener(Self::key))
            .on_action(cx.listener(|g, _: &Copy, w, cx| g.clipboard_action(false, w, cx)))
            .on_action(cx.listener(|g, _: &Paste, w, cx| g.clipboard_action(true, w, cx)))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|g, _, _, _| g.dragging = false),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|g, _, _, _| g.dragging = false),
            )
            .children(
                self.message
                    .clone()
                    .map(|m| div().text_color(cx.theme().danger).child(m)),
            )
            .child(
                div().flex_1().min_h_0().child(
                    DataTable::new(&self.table)
                        .bordered(false)
                        .with_size(Size::Medium)
                        .header_row_height(px(34.))
                        .accessibility_label(self.selection(cx).map_or_else(
                            || "Query results".into(),
                            |s| s.table_label(self.table.read(cx).delegate().columns_count(cx)),
                        )),
                ),
            )
    }
}
