use super::{Columns, DecodedPage};
use rsql_core::wire::Cell;
use std::{
    collections::{HashMap, hash_map::RandomState},
    hash::{BuildHasher, Hash, Hasher},
    sync::Arc,
};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnapshotKey {
    pub result: Uuid,
    pub filter_revision: u64,
}

#[derive(Clone)]
pub struct RowSnapshot {
    pub key: SnapshotKey,
    pub columns: Arc<Columns>,
    pub page: Arc<DecodedPage>,
    pub filtered: Option<Arc<[usize]>>,
}

impl RowSnapshot {
    pub fn len(&self) -> usize {
        self.filtered.as_ref().map_or(self.page.len(), |r| r.len())
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn source_index(&self, position: usize) -> Option<usize> {
        match &self.filtered {
            Some(indices) => indices.get(position).copied(),
            None => (position < self.page.len()).then_some(position),
        }
    }
    fn indices(&self) -> impl Iterator<Item = usize> + '_ {
        (0..self.len()).map(|i| self.source_index(i).unwrap())
    }
}

#[derive(Clone)]
pub struct Pin {
    pub id: Uuid,
    pub rows: RowSnapshot,
    pub label: String,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct DiffKey {
    pub pin: Uuid,
    pub original: SnapshotKey,
    pub current: SnapshotKey,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Comparison {
    ColumnsDiffer,
    Rows {
        added: Vec<usize>,
        removed: Vec<usize>,
        unchanged: usize,
    },
}

const CHUNK: usize = 8192;

fn equal_bytes(a: &str, b: &str, current: &impl Fn() -> bool) -> Option<bool> {
    if a.len() != b.len() {
        return Some(false);
    }
    for (a, b) in a.as_bytes().chunks(CHUNK).zip(b.as_bytes().chunks(CHUNK)) {
        current().then_some(())?;
        if a != b {
            return Some(false);
        }
    }
    Some(true)
}

fn equal_row(a: &[Cell], b: &[Cell], current: &impl Fn() -> bool) -> Option<bool> {
    if a.len() != b.len() {
        return Some(false);
    }
    for (a, b) in a.iter().zip(b) {
        current().then_some(())?;
        match (a, b) {
            (None, None) => (),
            (Some(a), Some(b)) if equal_bytes(a, b, current)? => (),
            _ => return Some(false),
        }
    }
    Some(true)
}

fn fingerprint(row: &[Cell], seed: &RandomState, current: &impl Fn() -> bool) -> Option<u64> {
    let mut hash = seed.build_hasher();
    for cell in row {
        current().then_some(())?;
        cell.as_ref().map(String::len).hash(&mut hash);
        if let Some(s) = cell {
            for chunk in s.as_bytes().chunks(CHUNK) {
                current().then_some(())?;
                hash.write(chunk);
            }
        }
    }
    Some(hash.finish())
}

struct Membership<'a> {
    page: &'a DecodedPage,
    buckets: HashMap<u64, Vec<usize>>,
}

impl Membership<'_> {
    fn contains(&self, hash: u64, row: &[Cell], current: &impl Fn() -> bool) -> Option<bool> {
        if let Some(bucket) = self.buckets.get(&hash) {
            for &i in bucket {
                if equal_row(&self.page[i], row, current)? {
                    return Some(true);
                }
            }
        }
        Some(false)
    }
}

/// Source set membership counts repeated current members and emits absent
/// duplicates in their original order. Only indices and fingerprints are stored.
pub fn compare(
    pin: &RowSnapshot,
    now: &RowSnapshot,
    current: impl Fn() -> bool,
) -> Option<Comparison> {
    if pin.columns.len() != now.columns.len() {
        return Some(Comparison::ColumnsDiffer);
    }
    for (a, b) in pin.columns.iter().zip(now.columns.iter()) {
        current().then_some(())?;
        if !equal_bytes(a, b, &current)? {
            return Some(Comparison::ColumnsDiffer);
        }
    }
    let seed = RandomState::new();
    let mut pinned = Membership {
        page: &pin.page,
        buckets: HashMap::new(),
    };
    let mut hashes = Vec::with_capacity(pin.len());
    for i in pin.indices() {
        current().then_some(())?;
        let hash = fingerprint(&pin.page[i], &seed, &current)?;
        hashes.push(hash);
        if !pinned.contains(hash, &pin.page[i], &current)? {
            pinned.buckets.entry(hash).or_default().push(i);
        }
    }
    let mut present = Membership {
        page: &now.page,
        buckets: HashMap::new(),
    };
    let mut added = Vec::new();
    let mut unchanged = 0;
    for i in now.indices() {
        current().then_some(())?;
        let row = &now.page[i];
        let hash = fingerprint(row, &seed, &current)?;
        if pinned.contains(hash, row, &current)? {
            unchanged += 1;
            if !present.contains(hash, row, &current)? {
                present.buckets.entry(hash).or_default().push(i);
            }
        } else {
            added.push(i);
        }
    }
    let mut removed = Vec::new();
    for (i, hash) in pin.indices().zip(hashes) {
        current().then_some(())?;
        if !present.contains(hash, &pin.page[i], &current)? {
            removed.push(i);
        }
    }
    Some(Comparison::Rows {
        added,
        removed,
        unchanged,
    })
}
