use super::super::theme::controls::compact_button;
use super::*;
use gpui_kit::component::{Disableable, label::Label};
use rsql_desktop::results::{
    FieldLocation, ViewChoice,
    diff::{DiffKey, RowSnapshot, SnapshotKey},
};

pub(in crate::workspace) const FIELD_COUNT: usize = 12;
const PREVIEW_BYTES: usize = 512;
const DETAIL_BYTES: usize = 4096;

#[derive(Clone)]
pub(in crate::workspace) struct Preview {
    pub text: SharedString,
    pub partial: bool,
}
impl Preview {
    pub fn new(value: &str) -> Self {
        let end = value.floor_char_boundary(PREVIEW_BYTES.min(value.len()));
        Self {
            text: value[..end].to_owned().into(),
            partial: end < value.len(),
        }
    }
}

pub(super) struct RecordField {
    pub label: Preview,
    pub value: Option<Preview>,
}

struct RecordPreview {
    key: (Uuid, usize, usize),
    fields: Rc<[RecordField]>,
}

#[derive(Default)]
pub(in crate::workspace) struct FieldCache {
    record: Option<RecordPreview>,
    chunk: Option<((Uuid, FieldLocation, usize), SharedString)>,
}
impl FieldCache {
    pub(in crate::workspace) fn retain_for(&mut self, view: Option<&ResultView>) {
        if view.is_none_or(|v| v.choice() != ViewChoice::Record) {
            self.record = None;
        }
        if view.is_none_or(|v| v.field_detail.is_none()) {
            self.chunk = None;
        }
    }

    pub(super) fn record(
        &mut self,
        rows: &RowSnapshot,
        row: usize,
        start: usize,
    ) -> Rc<[RecordField]> {
        let key = (rows.key.result, row, start);
        if let Some(previous) = &self.record
            && previous.key == key
        {
            return previous.fields.clone();
        }
        let fields: Rc<[_]> = rows
            .columns
            .iter()
            .zip(&rows.page[row])
            .skip(start)
            .take(FIELD_COUNT)
            .map(|(label, value)| RecordField {
                label: Preview::new(label),
                value: value.as_deref().map(Preview::new),
            })
            .collect();
        self.record = Some(RecordPreview {
            key,
            fields: fields.clone(),
        });
        fields
    }
    fn chunk(
        &mut self,
        id: Uuid,
        location: FieldLocation,
        index: usize,
        value: &str,
    ) -> SharedString {
        let key = (id, location, index);
        if let Some((previous, text)) = &self.chunk
            && *previous == key
        {
            return text.clone();
        }
        // Adjacent chunks share the very same rounded byte boundary.
        let start = value.floor_char_boundary(index * DETAIL_BYTES);
        let end = value.floor_char_boundary(((index + 1) * DETAIL_BYTES).min(value.len()));
        let text: SharedString = value[start..end].to_owned().into();
        self.chunk = Some((key, text.clone()));
        text
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct FieldOwner(SnapshotKey, ViewChoice, Option<DiffKey>);
impl FieldOwner {
    pub fn new(view: &ResultView) -> Self {
        Self(
            view.key(),
            view.choice(),
            view.comparison.as_ref().map(|d| d.key),
        )
    }
}

impl Workspace {
    pub(super) fn change_field(
        &self,
        owner: FieldOwner,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut ResultView),
    ) {
        let mut docs = self.session.documents.borrow_mut();
        if let Some(view) = docs
            .selected
            .map(|i| &mut docs.tabs[i])
            .and_then(|t| t.main.view_mut())
            .filter(|v| FieldOwner::new(v) == owner)
        {
            f(view);
            cx.notify();
        }
    }

    pub(in crate::workspace) fn field_detail(
        &self,
        view: &ResultView,
        cx: &Context<Self>,
    ) -> Option<Div> {
        let (location, index) = view.field_detail?;
        let owner = FieldOwner::new(view);
        let close = compact_button(self.button("Close field", cx, move |v, _, cx| {
            v.change_field(owner, cx, |r| {
                if r.field_detail == Some((location, index)) {
                    r.field_detail = None;
                }
            })
        }));
        let rows = match location.pinned {
            Some(id) => self
                .session
                .pin
                .borrow()
                .as_ref()
                .filter(|p| p.id == id)?
                .rows
                .clone(),
            None if view.choice() == ViewChoice::Map => {
                let Some(input) = view.map_input() else {
                    let pending = view.data.cache.error(0).is_none();
                    return Some(
                        div()
                            .children(
                                pending.then(|| Label::new("Waiting for the first result page…")),
                            )
                            .child(close),
                    );
                };
                input.rows
            }
            None => view.snapshot(if view.choice() == ViewChoice::Diff {
                ResultAction::Diff
            } else {
                ResultAction::Record
            })?,
        };
        let value = if location.label {
            rows.columns.get(location.column)?.as_str()
        } else {
            rows.page
                .get(location.row)?
                .get(location.column)?
                .as_deref()?
        };
        let chunks = value.len().div_ceil(DETAIL_BYTES).max(1);
        let index = index.min(chunks - 1);
        let text = self
            .field_cache
            .borrow_mut()
            .chunk(rows.key.result, location, index, value);
        let button = |label: &'static str, to: usize| {
            compact_button(self.button(label, cx, move |v, _, cx| {
                v.change_field(owner, cx, |r| {
                    if r.field_detail == Some((location, index)) {
                        r.field_detail = Some((location, to));
                    }
                })
            }))
            .disabled(to == index)
        };
        let controls = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_1()
            .child(Label::new(format!(
                "{} {} · chunk {} of {}",
                if location.label {
                    "Column name"
                } else {
                    "Field"
                },
                location.column + 1,
                index + 1,
                chunks
            )))
            .child(button("First chunk", 0))
            .child(button("Previous chunk", index.saturating_sub(1)))
            .child(button("Next chunk", (index + 1).min(chunks - 1)))
            .child(button("Last chunk", chunks - 1))
            .child(close);
        Some(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .child(controls)
                .child(
                    div()
                        .id("field-text")
                        .overflow_y_scroll()
                        .flex_1()
                        .min_h_0()
                        .font_family(cx.theme().mono_font_family.clone())
                        .child(Label::new(text).id("field-chunk-value").whitespace_normal()),
                ),
        )
    }
}
