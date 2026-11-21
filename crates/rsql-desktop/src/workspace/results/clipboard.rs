use super::*;

use tokio::sync::watch;

pub(super) struct Work {
    id: Uuid,
    _cancel: watch::Sender<()>,
    _task: Task<()>,
}

enum Prepared {
    Copied(String),
    Pasted(EditIntent),
}

fn copy(
    data: &GridData,
    selection: Selection,
    current: &impl Fn() -> bool,
) -> Result<Prepared, String> {
    let (rows, cols) = selection.ranges(data.result.columns().len());
    let mut text = super::tsv::Field::default();
    let single = rows.len() == 1 && cols.len() == 1;
    for (r, row) in rows.enumerate() {
        if r > 0 {
            text.push("\n");
        }
        for (c, col) in cols.clone().enumerate() {
            if c > 0 {
                text.push("\t");
            }
            if !current() {
                return Err("Cancelled".into());
            }
            let pos = data
                .retained(Position { row, col })
                .ok_or("Selection is outside the result")?;
            let state = data.result.cache.cell(pos.row, pos.col);
            let value = match &state {
                CellState::Value(v) => data
                    .access
                    .edited
                    .get(&pos)
                    .map(AsRef::as_ref)
                    .unwrap_or(v.raw())
                    .as_deref()
                    .unwrap_or(""),
                CellState::Loading => {
                    return Err(format!(
                        "Row {} is outside the loaded data. Copy a smaller loaded range.",
                        row + 1
                    ));
                }
                CellState::Error(e) => return Err(format!("Could not copy row {}: {e}", row + 1)),
            };
            if single && value.is_empty() {
                return Ok(Prepared::Copied(String::new()));
            }
            let chunks = || {
                let mut rest = value;
                std::iter::from_fn(move || {
                    if rest.is_empty() {
                        return None;
                    }
                    let (part, tail) = rest.split_at(rest.floor_char_boundary(8192));
                    rest = tail;
                    Some(part)
                })
            };
            let mut quote = value.is_empty() && cols.len() == 1;
            for part in chunks() {
                if !current() {
                    return Err("Cancelled".into());
                }
                if part.contains(['\t', '\r', '\n', '"']) {
                    quote = true;
                    break;
                }
            }
            if quote {
                text.push("\"");
            }
            for part in chunks() {
                if !current() {
                    return Err("Cancelled".into());
                }
                if quote {
                    text.push(&part.replace('"', "\"\""));
                } else {
                    text.push(part);
                }
            }
            if quote {
                text.push("\"");
            }
        }
    }
    if !current() {
        return Err("Cancelled".into());
    }
    text.finish(current).map(Prepared::Copied)
}

impl Grid {
    pub(super) fn clipboard_action(
        &mut self,
        pasting: bool,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.table.focus_handle(cx).is_focused(w) || !self.current(cx) || self.editor.is_some()
        {
            cx.propagate();
            return;
        }
        let d = self.table.read(cx).delegate();
        let Some(selection) = d.selection else {
            return;
        };
        let data = d.data.clone();
        let input = if pasting {
            if data.access.eligibility.is_none() || !self.owner.can_edit() {
                return;
            }
            let Some(item) = crate::desktop::DesktopServices::read_clipboard(cx) else {
                return;
            };
            Some(item)
        } else {
            None
        };
        let generation = self.owner.with_view(|v| v.loader.generation());
        let (rows, cols) = selection.ranges(data.result.columns().len());
        let start = Position {
            row: rows.start,
            col: cols.start,
        };
        let order = (!pasting).then(|| clipboard_order::ClipboardWrite::begin(cx));
        let writing = order.clone();
        let id = Uuid::new_v4();
        let access = data.access.clone();
        let (cancel, cancelled) = watch::channel(());
        let stopped = self.owner.session.preflight_stop.subscribe();
        let edit = if pasting {
            self.owner.with_view(|v| v.edit.clone()).flatten()
        } else {
            None
        };
        let worker = self.owner.session.runtime.spawn_blocking(move || {
            let current = || {
                !*stopped.borrow()
                    && cancelled.has_changed().is_ok()
                    && writing.as_ref().is_none_or(|w| w.current())
            };
            match input {
                Some(item) => {
                    let changes = super::tsv::paste(item, &data, start, &current)?;
                    let mut intent = EditIntent {
                        result: data.result.id,
                        session: data.access.session,
                        changes,
                        prepared: None,
                    };
                    if let Some(edit) = edit
                        && !edit.prepare_paste(&mut intent, current)
                    {
                        return Err("Cancelled".into());
                    }
                    Ok(Prepared::Pasted(intent))
                }
                None => copy(&data, selection, &current),
            }
        });
        self.message = None;
        self.clipboard = Some(Work {
            id,
            _cancel: cancel,
            _task: cx.spawn(async move |g, cx| {
                let result = worker.await.map_err(|e| e.to_string()).and_then(|r| r);
                let _ = g.update(cx, |g, cx| {
                    if g.clipboard.as_ref().is_none_or(|work| work.id != id) {
                        return;
                    }
                    g.clipboard = None;
                    if order.as_ref().is_some_and(|w| !w.current()) {
                        return;
                    }
                    if !g.current(cx)
                        || g.owner.with_view(|v| v.loader.generation()) != generation
                        || !Arc::ptr_eq(&g.table.read(cx).delegate().data.access, &access)
                    {
                        return;
                    }
                    match result {
                        Ok(Prepared::Copied(text)) => {
                            crate::desktop::DesktopServices::write_clipboard(
                                cx,
                                ClipboardItem::new_string(text),
                            )
                        }
                        Ok(Prepared::Pasted(intent))
                            if !intent.changes.is_empty() && g.owner.can_edit() =>
                        {
                            cx.emit(intent)
                        }
                        Ok(_) => {}
                        Err(e) => g.message = Some(e),
                    }
                    cx.notify();
                });
            }),
        });
    }
}
