use super::*;
use crate::workspace::theme;
use gpui_kit::base::TestTextExt;
use gpui_kit::component::table::Column;

pub(super) struct Delegate {
    pub owner: GridOwner,
    pub data: GridData,
    pub grid: WeakEntity<Grid>,
    pub selection: Option<Selection>,
    pub row_press: Option<(usize, bool)>,
    widths: Vec<Pixels>,
    manual: HashSet<usize>,
    sampled: bool,
}

impl Delegate {
    pub fn new(owner: GridOwner, data: GridData, grid: WeakEntity<Grid>) -> Self {
        let mut delegate = Self {
            owner,
            data,
            grid,
            selection: None,
            row_press: None,
            widths: vec![],
            manual: Default::default(),
            sampled: false,
        };
        delegate.sync_widths();
        delegate
    }

    pub fn resized(&mut self, widths: &[Pixels]) {
        for (i, (old, &new)) in self.widths.iter_mut().zip(widths).enumerate() {
            if *old != new {
                self.manual.insert(i);
                *old = new;
            }
        }
    }

    pub fn needs_widths(&self) -> bool {
        self.widths.len() != self.data.result.columns().len()
            || (!self.sampled && self.data.result.cache.page(0).is_some())
    }

    pub fn sync_widths(&mut self) {
        let cols = self.data.result.columns();
        let page = self.data.result.cache.page(0);
        if cols.len() == self.widths.len() && (self.sampled || page.is_none()) {
            return;
        }
        self.widths.resize(cols.len(), px(80.));
        for (i, label) in cols.iter().enumerate() {
            if self.manual.contains(&i) {
                continue;
            }
            let mut units = label.encode_utf16().take(49).count() + 2;
            if let Some(page) = &page {
                for row in page.iter().take(100) {
                    if units >= 51 {
                        break;
                    }
                    if let Some(Some(value)) = row.get(i) {
                        units = units.max(value.encode_utf16().take(51).count());
                    }
                }
            }
            self.widths[i] = px((units as f32 * 7.5 + 24.).clamp(80., 400.));
        }
        self.sampled = page.is_some();
    }

    fn content(&self, pos: Position, label: bool, cx: &App) -> (String, Hsla, Option<Hsla>) {
        let c = theme::grid::colors(cx.theme().mode.is_dark());
        let access = &self.data.access;
        let raw = self.data.result.cache.cell(pos.row, pos.col);
        let value = match &raw {
            CellState::Loading => {
                return (if label { "Loading" } else { "…" }.into(), c.loading, None);
            }
            CellState::Error(e) => return (format!("Error: {e}"), cx.theme().danger, None),
            CellState::Value(v) => access
                .edited
                .get(&pos)
                .map(AsRef::as_ref)
                .unwrap_or(v.raw()),
        };
        let fk = access.eligibility.is_none()
            && access.foreign_keys.contains(&pos.col)
            && value.is_some();
        let text = match value {
            None => if label { "SQL NULL" } else { "NULL" }.into(),
            Some(s) if label && s.is_empty() => "Empty text".into(),
            Some(s) if label => format!("Text: {}", clipped(s)),
            Some(s) if fk => format!("{} →", clipped(s)),
            Some(s) => clipped(s),
        };
        if access.deleted.contains(&pos.row) {
            return (text, c.deleted_text, Some(c.deleted));
        }
        if access.edited.contains_key(&pos) {
            return (text, c.text, Some(c.modified));
        }
        (
            text,
            if value.is_none() {
                c.null
            } else if fk {
                c.fk
            } else {
                c.text
            },
            None,
        )
    }
}

impl TableDelegate for Delegate {
    fn columns_count(&self, _: &App) -> usize {
        self.widths.len()
    }
    fn rows_count(&self, _: &App) -> usize {
        self.data.rows()
    }
    fn column(&self, i: usize, _: &App) -> Column {
        Column::new(i.to_string(), clipped(&self.data.result.columns()[i]))
            .width(self.widths[i])
            .p_0()
            .movable(false)
    }
    fn render_th(
        &mut self,
        i: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        div()
            .px(px(12.))
            .truncate()
            .font_family(cx.theme().mono_font_family.clone())
            .text_size(px(12.))
            .font_weight(FontWeight::BOLD)
            .text_color(cx.theme().table_head_foreground)
            .child(self.column(i, cx).name)
    }
    fn render_tr(
        &mut self,
        row: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> Stateful<Div> {
        div().id(("row", row)).on_mouse_down(
            MouseButton::Left,
            cx.listener(move |table, e: &MouseDownEvent, _, _| {
                table.delegate_mut().row_press = Some((row, e.modifiers.shift));
            }),
        )
    }

    fn cell_label(&self, row: usize, col: usize, cx: &App) -> Option<SharedString> {
        let pos = Position { row, col };
        let retained = self.data.retained(pos)?;
        Some(self.content(retained, true, cx).0.into())
    }
    fn row_label(&self, row: usize, _: &App) -> Option<SharedString> {
        Some(format!("Row {}", row + 1).into())
    }

    fn render_td(
        &mut self,
        row: usize,
        col: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let pos = Position { row, col };
        let grid = self.grid.clone();
        let mut cell = div()
            .id(format!("grid-cell-{row}-{col}"))
            .test_support()
            .size_full()
            .text_size(px(12.))
            .font_family(cx.theme().mono_font_family.clone())
            .flex()
            .items_center()
            .overflow_hidden();
        if let Some(editor) = grid.upgrade().and_then(|g| {
            g.read(cx)
                .editor
                .as_ref()
                .filter(|e| e.display == pos)
                .map(|e| e.input.clone())
        }) {
            return cell.child(super::cell_editor::element(editor, grid));
        }
        let (text, color, bg) = self
            .data
            .retained(pos)
            .map(|p| self.content(p, false, cx))
            .unwrap_or_default();
        if let Some(bg) = bg {
            cell = cell.bg(bg);
        }
        if self.selection.is_some_and(|s| s.contains(pos)) {
            cell = cell.bg(cx.theme().table_active);
        }
        if let Some(p) = self.data.retained(pos) {
            let access = &self.data.access;
            if col == 0
                && access
                    .eligibility
                    .as_ref()
                    .is_some_and(|e| e.rows.contains(&p.row))
            {
                let toggle = grid.clone();
                let marked = access.deleted.contains(&p.row);
                let session = access.session;
                cell = cell.child(
                    gpui_kit::component::checkbox::Checkbox::new(("delete-row", row))
                        .checked(marked)
                        .accessibility_label(format!("Mark row {} for deletion", row + 1))
                        .on_click(move |checked, _, cx| {
                            cx.stop_propagation();
                            let _ = toggle.update(cx, |g, cx| {
                                if g.current(cx) {
                                    cx.emit(super::edit_actions::GridAction::Mark(
                                        p.row, *checked, session,
                                    ));
                                }
                            });
                        }),
                );
            }
            if access.eligibility.is_none()
                && access.foreign_keys.contains(&col)
                && self.data.result.cache.foreign_target(p.row, col).is_some()
            {
                let navigate = grid.clone();
                cell = cell.on_click(move |_, _, cx| {
                    let _ = navigate.update(cx, |g, cx| {
                        if g.current(cx)
                            && let Some(generation) = g.owner.with_view(|v| v.loader.generation())
                        {
                            cx.emit(super::edit_actions::GridAction::Navigate(p, generation));
                        }
                    });
                });
            }
        }
        let drag = grid.clone();
        let text = SharedString::from(text);
        cell.px(px(12.))
            .text_color(color)
            .child(div().truncate().child(text.clone()).test_text(text))
            .on_mouse_down(MouseButton::Left, move |e, w, cx| {
                let _ = grid.update(cx, |g, cx| g.pointer(pos, e.modifiers.shift, false, w, cx));
            })
            .on_mouse_move(move |e, w, cx| {
                if e.pressed_button == Some(MouseButton::Left) {
                    let _ = drag.update(cx, |g, cx| g.pointer(pos, true, true, w, cx));
                }
            })
    }

    fn visible_rows_changed(
        &mut self,
        rows: Range<usize>,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) {
        self.owner.with_view(|view| {
            // Filters apply only to a fully resident local result.
            let range = match self
                .data
                .filtered
                .as_ref()
                .and_then(|f| f.get(rows.clone()))
            {
                Some(slice) => slice
                    .iter()
                    .min()
                    .zip(slice.iter().max())
                    .map_or(0..0, |(a, b)| *a..b + 1),
                None => rows,
            };
            view.loader.viewport(range);
        });
    }
}
