use super::{
    DecodedPage,
    cell_input::{CellAccess, EditEligibility, EditIntent, Position},
};
use rsql_core::postgres::provenance::ForeignReference;
use rsql_core::{
    mutation::{MutationKind, RowMutation},
    postgres::provenance::ResultOrigin,
    services::ProjectTicket,
    wire::Cell,
};
use std::sync::Weak;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
use sum_tree::TreeMap;
use uuid::Uuid;

pub(crate) struct OriginRecipe {
    pub generated: Option<Arc<super::ForeignQuery>>,
    pub result: Uuid,
    pub sql: Arc<str>,
    pub route: ProjectTicket,
    pub origin: ResultOrigin,
}

pub struct EditTarget {
    pub generated: Option<Arc<super::ForeignQuery>>,
    pub result: Uuid,
    pub sql: Arc<str>,
    pub route: ProjectTicket,
    pub origin: ResultOrigin,
    foreign_keys: HashMap<usize, ForeignReference>,
    eligibility: Arc<EditEligibility>,
    page: Weak<DecodedPage>,
    keys: Vec<usize>,
}

impl EditTarget {
    pub(crate) fn prepare(
        recipe: &OriginRecipe,
        labels: &[String],
        page: &Arc<DecodedPage>,
        mut current: impl FnMut() -> bool,
    ) -> Option<Self> {
        let mut origin = recipe.origin.clone();
        if origin.labels != labels || origin.output.len() != origin.labels.len() {
            return None;
        }
        let mut numbers = HashSet::new();
        let mut labels = HashSet::new();
        let mut duplicates = HashSet::new();
        let mut repeated_labels = HashSet::new();
        for (label, n) in origin.labels.iter().zip(&origin.output) {
            if !labels.insert(label) {
                repeated_labels.insert(label);
            }
            if let Some(n) = n
                && !numbers.insert(n)
            {
                duplicates.insert(*n);
            }
        }
        for (label, n) in origin.labels.iter().zip(&mut origin.output) {
            if n.is_some_and(|n| duplicates.contains(&n)) || repeated_labels.contains(label) {
                *n = None
            }
        }
        let relation = &origin.relation;
        let mut columns = HashSet::new();
        let mut foreign_keys = HashMap::new();
        for (i, id) in origin.output.iter().enumerate() {
            if !current() {
                return None;
            }
            let Some(a) = relation.attributes.iter().find(|a| Some(a.number) == *id) else {
                continue;
            };
            if a.writable {
                columns.insert(i);
            }
            if let Some(reference) = origin
                .foreign_keys
                .iter()
                .find(|fk| fk.source_column == a.name)
                && reference.lossless
            {
                foreign_keys.insert(i, reference.clone());
            }
        }
        let keys = relation
            .primary_key
            .iter()
            .map(|n| origin.output.iter().position(|v| *v == Some(*n)))
            .collect::<Option<Vec<_>>>()
            .unwrap_or_default();
        let mut rows = HashSet::new();
        if !keys.is_empty() && relation.descendant_safe && relation.lossless_key() {
            let mut seen = HashMap::new();
            for (i, row) in page.iter().enumerate() {
                if !current() {
                    return None;
                }
                if row.len() != origin.labels.len() {
                    continue;
                }
                if let Some(key) = keys
                    .iter()
                    .map(|k| row[*k].as_deref())
                    .collect::<Option<Vec<_>>>()
                {
                    match seen.entry(key) {
                        std::collections::hash_map::Entry::Vacant(v) => {
                            v.insert(i);
                            rows.insert(i);
                        }
                        std::collections::hash_map::Entry::Occupied(v) => {
                            rows.remove(v.get());
                        }
                    }
                }
            }
        }
        Some(Self {
            generated: recipe.generated.clone(),
            result: recipe.result,
            sql: recipe.sql.clone(),
            route: recipe.route.clone(),
            origin,
            foreign_keys,
            keys,
            page: Arc::downgrade(page),
            eligibility: Arc::new(EditEligibility { rows, columns }),
        })
    }

    pub fn can_edit(&self) -> bool {
        self.route.is_current() && !self.eligibility.rows.is_empty()
    }
    pub fn readonly_reason(&self) -> &'static str {
        if !self.origin.relation.descendant_safe {
            "Read only · inherited rows cannot be identified"
        } else if self.origin.relation.primary_key.is_empty() {
            "Read only · table has no primary key"
        } else if !self.origin.relation.lossless_key() {
            "Read only · primary key type has no proven text round trip"
        } else if self.keys.is_empty() {
            "Read only · select every primary key column once"
        } else {
            "Read only · row identity unavailable"
        }
    }
    pub fn browse(&self) -> CellAccess {
        CellAccess {
            foreign_keys: self.foreign_keys.keys().copied().collect(),
            ..Default::default()
        }
    }
    pub(super) fn has_foreign_key(&self, col: usize) -> bool {
        self.foreign_keys.contains_key(&col)
    }
    pub fn navigate(
        &self,
        row: usize,
        col: usize,
        current: impl Fn() -> bool,
    ) -> Option<super::ForeignQuery> {
        let reference = self.foreign_keys.get(&col)?;
        let page = self.page.upgrade()?;
        super::ForeignQuery::build(
            reference,
            page.get(row)?.get(col)?.as_deref()?,
            self.route.clone(),
            current,
        )
    }
    fn name(&self, i: usize) -> String {
        self.origin
            .relation
            .attributes
            .iter()
            .find(|a| Some(a.number) == self.origin.output[i])
            .expect("verified output mapping")
            .name
            .clone()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Draft,
    Review,
    Applying,
}

pub struct PreparedEdit {
    base: Arc<CellAccess>,
    session: EditSession,
}
#[derive(Clone)]
pub struct EditSession {
    changed_rows: TreeMap<usize, usize>,
    page: Arc<DecodedPage>,
    deletions: usize,
    updates: usize,
    pub target: Arc<EditTarget>,
    pub phase: Phase,
    pub access: Arc<CellAccess>,
}

impl EditSession {
    pub fn begin(target: Arc<EditTarget>) -> Option<Self> {
        let page = target.page.upgrade()?;
        target.can_edit().then(|| Self {
            page,
            deletions: 0,
            changed_rows: Default::default(),
            updates: 0,
            access: Arc::new(CellAccess {
                session: Some(Uuid::new_v4()),
                eligibility: Some(target.eligibility.clone()),
                ..Default::default()
            }),
            target,
            phase: Phase::Draft,
        })
    }
    pub fn stage(&mut self, intent: &EditIntent) {
        if intent.session != self.access.session
            || intent.result != self.target.result
            || self.phase != Phase::Draft
            || !self.target.route.is_current()
        {
            return;
        }
        if let Some(prepared) = &intent.prepared {
            if Arc::ptr_eq(&prepared.base, &self.access) {
                *self = prepared.session.clone();
            }
        } else {
            self.stage_values(intent, || true);
        }
    }

    /// Prepare a rectangular paste alongside parsing on the owned background worker.
    pub fn prepare_paste(mut self, intent: &mut EditIntent, current: impl Fn() -> bool) -> bool {
        let base = self.access.clone();
        if !self.stage_values(intent, current) {
            return false;
        }
        intent.prepared = Some(PreparedEdit {
            base,
            session: self,
        });
        true
    }

    fn stage_values(&mut self, intent: &EditIntent, current: impl Fn() -> bool) -> bool {
        let access = Arc::make_mut(&mut self.access);
        for (pos, value) in &intent.changes {
            if !current() {
                return false;
            }
            if !access.allows(*pos) {
                continue;
            }
            let cells = self.changed_rows.get(&pos.row).copied().unwrap_or_default();
            if self.page[pos.row][pos.col] == *value {
                if access.edited.remove(pos).is_some() {
                    if cells == 1 {
                        self.changed_rows.remove(&pos.row);
                        self.updates -= 1;
                    } else {
                        self.changed_rows.insert(pos.row, cells - 1);
                    }
                }
            } else if access
                .edited
                .insert_or_replace(*pos, Arc::new(value.clone()))
                .is_none()
            {
                self.changed_rows.insert(pos.row, cells + 1);
                if cells == 0 {
                    self.updates += 1;
                }
            }
        }
        true
    }
    pub fn mark(&mut self, row: usize, marked: bool) {
        if self.access.deleted.contains(&row) == marked
            || self.phase != Phase::Draft
            || !self.target.eligibility.rows.contains(&row)
            || !self.target.route.is_current()
        {
            return;
        }
        let access = Arc::make_mut(&mut self.access);
        let deletes = &mut access.deleted;
        if deletes.remove(&row) {
            self.deletions -= 1;
            if self.changed_rows.contains_key(&row) {
                self.updates += 1;
            }
        } else {
            deletes.insert(row);
            self.deletions += 1;
            if self.changed_rows.contains_key(&row) {
                self.updates -= 1;
            }
        }
    }
    pub fn counts(&self) -> (usize, usize) {
        (self.updates, self.deletions)
    }
    pub fn has_changes_after_replacing(&self, position: Position, value: &Cell) -> bool {
        if self.deletions != 0 || self.updates > 1 {
            return true;
        }
        if !self.access.allows(position) {
            return self.updates != 0;
        }
        if self.page[position.row][position.col] != *value {
            return true;
        }
        let removes_only_update = self.changed_rows.get(&position.row) == Some(&1)
            && self.access.edited.contains_key(&position);
        self.updates != 0 && !removes_only_update
    }
    pub fn payload(
        target: &EditTarget,
        access: &CellAccess,
        check: impl Fn() -> Result<(), String>,
    ) -> Result<Vec<RowMutation>, String> {
        let page = target
            .page
            .upgrade()
            .ok_or("Original result page is unavailable")?;
        let pk = |row: usize| {
            target
                .keys
                .iter()
                .map(|k| (target.name(*k), page[row][*k].clone()))
                .collect()
        };
        let mut updates = std::collections::BTreeMap::<usize, Vec<_>>::new();
        for (p, v) in access.edited.iter() {
            check()?;
            if !access.deleted.contains(&p.row) {
                updates
                    .entry(p.row)
                    .or_default()
                    .push((target.name(p.col), v.as_ref().clone()));
            }
        }
        let mut batch: Vec<_> = updates
            .into_iter()
            .map(|(row, mut set)| {
                set.sort_by(|a, b| a.0.cmp(&b.0));
                RowMutation {
                    kind: MutationKind::Update,
                    pk: pk(row),
                    set,
                }
            })
            .collect();
        let mut deletes: Vec<_> = access.deleted.iter().copied().collect();
        deletes.sort_unstable();
        for row in deletes {
            check()?;
            batch.push(RowMutation {
                kind: MutationKind::Delete,
                pk: pk(row),
                set: vec![],
            });
        }
        Ok(batch)
    }
}
