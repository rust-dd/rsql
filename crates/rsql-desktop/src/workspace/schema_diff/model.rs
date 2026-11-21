use icu_collator::Collator;
use icu_locale_core::Locale;
use rsql_core::postgres::roles_schema_objects::SchemaObject;
use std::{collections::BTreeMap, ops::Range, sync::Arc};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Status {
    Modified,
    OnlyLeft,
    OnlyRight,
    Identical,
}

#[derive(Clone, Copy, Default, Eq, PartialEq)]
pub(super) enum Filter {
    #[default]
    All,
    Changes,
    Status(Status),
}

#[derive(Debug)]
pub(super) struct DiffEntry {
    pub kind: String,
    pub name: String,
    pub status: Status,
    pub left: Option<Arc<String>>,
    pub right: Option<Arc<String>>,
}

#[derive(Debug)]
pub(super) struct Comparison {
    pub entries: Vec<DiffEntry>,
    ends: [usize; 4],
}

fn sort_key(collator: &icu_collator::CollatorBorrowed<'_>, text: &str) -> Vec<u8> {
    let mut key = Vec::new();
    let Ok(()) = collator.write_sort_key_to(text, &mut key);
    key
}

impl Comparison {
    pub fn new(
        left: Vec<SchemaObject>,
        right: Vec<SchemaObject>,
        check: impl FnMut() -> Result<(), String>,
    ) -> Result<Self, String> {
        Self::with_locale(left, right, sys_locale::get_locale().as_deref(), check)
    }

    fn with_locale(
        left: Vec<SchemaObject>,
        right: Vec<SchemaObject>,
        locale: Option<&str>,
        mut check: impl FnMut() -> Result<(), String>,
    ) -> Result<Self, String> {
        let locale = locale
            .and_then(|tag| tag.parse::<Locale>().ok())
            .unwrap_or(Locale::UNKNOWN);
        let collator = Collator::try_new(locale.into(), Default::default())
            .or_else(|_| Collator::try_new(Default::default(), Default::default()))
            .map_err(|e| format!("Cannot load schema comparison collation: {e}"))?;
        let mut pairs = BTreeMap::<_, (usize, [Option<Arc<String>>; 2])>::new();
        for (side, objects) in [left, right].into_iter().enumerate() {
            for object in objects {
                check()?;
                let position = pairs.len();
                let (_, definitions) = pairs
                    .entry((object.object_type, object.name))
                    .or_insert_with(|| (position, Default::default()));
                definitions[side] = Some(Arc::new(object.definition));
            }
        }
        let mut entries = Vec::with_capacity(pairs.len());
        let mut groups = std::array::from_fn::<_, 4, _>(|_| BTreeMap::new());
        for ((kind, name), (position, [left, right])) in pairs {
            check()?;
            let status = match (left.as_deref(), right.as_deref()) {
                (Some(l), Some(r)) if super::definition::equal(l, r, &mut check)? => {
                    Status::Identical
                }
                (Some(_), Some(_)) => Status::Modified,
                (Some(_), None) => Status::OnlyLeft,
                (None, Some(_)) => Status::OnlyRight,
                (None, None) => unreachable!(),
            };
            // Incremental insertion gives every row a cancellation boundary during ordering.
            let order = (
                sort_key(&collator, &kind),
                sort_key(&collator, &name),
                position,
            );
            groups[status as usize].insert(
                order,
                DiffEntry {
                    kind,
                    name,
                    status,
                    left,
                    right,
                },
            );
        }
        let mut total = 0;
        let ends = groups.each_ref().map(|g| {
            total += g.len();
            total
        });
        for group in groups {
            for entry in group.into_values() {
                check()?;
                entries.push(entry);
            }
        }
        Ok(Self { entries, ends })
    }
    pub fn range(&self, filter: Filter) -> Range<usize> {
        match filter {
            Filter::All => 0..self.entries.len(),
            Filter::Changes => 0..self.ends[Status::OnlyRight as usize],
            Filter::Status(s) => {
                let i = s as usize;
                let start = i.checked_sub(1).map_or(0, |p| self.ends[p]);
                start..self.ends[i]
            }
        }
    }
}
