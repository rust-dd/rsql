mod prepare;
mod refresh;
mod review;
use super::*;
use gpui_kit::component::input::{InputEvent, InputState};
use rsql_desktop::{metadata::properties::PropertyText, structure::*};
use std::sync::Arc;
use structure_apply::{ApplyResult, ApplyReview, StructureOwner};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Page {
    #[default]
    Columns,
    Primary,
    Foreign,
    Unique,
    Index,
}
impl Page {
    pub(super) const ALL: [(Self, &'static str); 5] = [
        (Self::Columns, "Columns"),
        (Self::Primary, "Primary Key"),
        (Self::Foreign, "Foreign Keys"),
        (Self::Unique, "Unique"),
        (Self::Index, "Indexes"),
    ];
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Field {
    Name,
    Type,
    Default,
    KeyName,
}
pub(super) struct TextEdit {
    pub id: EditId,
    pub field: Field,
    pub input: Entity<InputState>,
    _events: Subscription,
}
pub(super) struct Editor {
    pub draft: Arc<StructureDraft>,
    owner: StructureOwner,
    pub page: Page,
    pub input: Option<TextEdit>,
    pub review: Option<(ApplyReview, PropertyText)>,
    revision: Uuid,
    pub reviewing: bool,
    review_cancel: Option<tokio::sync::watch::Sender<()>>,
    pub show_sql: bool,
    pub recover: bool,
    pub pending_fk_pairs: BTreeMap<EditId, usize>,
    pub targets: BTreeMap<EditId, LoadState<Arc<ForeignChoices>>>,
}

impl Editor {
    pub fn new(draft: Arc<StructureDraft>, owner: StructureOwner) -> Self {
        Self {
            draft,
            owner,
            page: Page::Columns,
            input: None,
            review: None,
            targets: Default::default(),
            pending_fk_pairs: Default::default(),
            revision: Uuid::new_v4(),
            reviewing: false,
            review_cancel: None,
            show_sql: false,
            recover: false,
        }
    }
    pub fn has_incomplete_fk_pairs(&self) -> bool {
        !self.pending_fk_pairs.is_empty()
            || self.draft.added_objects().iter().any(|(_, object)| {
                matches!(
                    object,
                    NewObject::ForeignKey(fk)
                        if fk.key.columns.len() != fk_target_column_count(&fk.target)
                )
            })
    }

    fn invalidate(&mut self) {
        self.review_cancel = None;
        self.review = None;
        self.show_sql = false;
        self.revision = Uuid::new_v4();
        self.reviewing = false;
    }
}

pub(super) fn fk_target_column_count(target: &ForeignTarget) -> usize {
    match target {
        ForeignTarget::ThisTable(ids) => ids.len(),
        ForeignTarget::Relation { columns, .. } => columns.len(),
    }
}

pub(super) fn change_key(
    draft: &mut StructureDraft,
    id: EditId,
    change: impl FnOnce(&mut KeyColumns),
) {
    if let Some(mut k) = draft
        .primary_key()
        .filter(|p| p.id() == id)
        .and_then(|p| p.current().cloned())
    {
        change(&mut k);
        draft.set_primary_key(k);
    } else if let Some(obj) = draft.new_object_mut(id) {
        match obj {
            NewObject::Unique(k)
            | NewObject::Index { key: k, .. }
            | NewObject::ForeignKey(ForeignKey { key: k, .. }) => change(k),
        }
    }
}

impl Workspace {
    pub(super) fn edit_draft(
        &mut self,
        cx: &mut Context<Self>,
        change: impl FnOnce(&mut StructureDraft),
    ) {
        self.edit_structure_state(cx, |editor| change(Arc::make_mut(&mut editor.draft)));
    }

    pub(super) fn edit_structure_state(
        &mut self,
        cx: &mut Context<Self>,
        change: impl FnOnce(&mut Editor),
    ) {
        let Some(p) = self.properties.as_ref() else {
            return;
        };
        if p.busy || !p.capture.current(self) {
            return;
        }
        let p = self.properties.as_mut().unwrap();
        let Some(e) = p.editor.as_mut().filter(|e| !e.recover) else {
            return;
        };
        e.invalidate();
        p.transfer = None;
        p.status.clear();
        change(e);
        e.targets
            .retain(|id, _| e.draft.added_objects().iter().any(|(item, _)| item == id));
        e.pending_fk_pairs
            .retain(|id, _| e.draft.added_objects().iter().any(|(item, _)| item == id));
        cx.notify();
    }
    pub(super) fn edit_field(
        &mut self,
        id: EditId,
        field: Field,
        value: SharedString,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let p = self.properties.as_mut().unwrap();
        let Some(e) = p.editor.as_mut().filter(|e| !e.recover && !p.busy) else {
            return;
        };
        let input = cx.new(|cx| InputState::new(w, cx).default_value(value));
        let events = cx.subscribe(&input, move |v, state, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change)
                && v.properties
                    .as_ref()
                    .and_then(|p| p.editor.as_ref())
                    .and_then(|e| e.input.as_ref())
                    .is_some_and(|i| i.input == state)
            {
                let value = state.read(cx).value();
                v.edit_draft(cx, |d| {
                    if field == Field::KeyName {
                        change_key(d, id, |k| k.name = value.into());
                    } else if let Some(c) = d.column_mut(id) {
                        match field {
                            Field::Name => c.name = value.into(),
                            Field::Type => c.data_type = value.into(),
                            Field::Default => c.default = (!value.is_empty()).then(|| value.into()),
                            Field::KeyName => unreachable!(),
                        }
                    }
                });
            }
        });
        input.focus_handle(cx).focus(w, cx);
        e.input = Some(TextEdit {
            id,
            field,
            input,
            _events: events,
        });
        cx.notify();
    }

    pub(super) fn reload_structure(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        let Some(p) = self.properties.as_ref().filter(|p| !p.busy) else {
            return;
        };
        let Some(c) = Capture::new(self, &p.object.project)
            .filter(|c| c.route.same_generation(&p.capture.route))
        else {
            return;
        };
        let p = self.properties.as_mut().unwrap();
        p.status = p.recovery_summary.clone().unwrap_or_default();
        p.capture = c;
        p.sections.clear();
        p.editor = None;
        p.transfer = None;
        p.tab = Tab::Structure;
        for &section in Tab::Structure.sections(true) {
            self.load_property(section, w, cx);
        }
    }
}
