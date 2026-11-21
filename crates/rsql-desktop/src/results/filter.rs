use super::DecodedPage;
use icu_properties::{
    CodePointSetData,
    props::{CaseIgnorable, Cased},
};
use std::sync::Arc;

const INTERVAL: usize = 4096;

/// Unicode default lowercase has one language-independent contextual mapping:
/// final sigma. Carry its cased/ignorable context across cancellation checkpoints.
fn lowercase(
    text: &str,
    current: &impl Fn() -> bool,
    mut emit: impl FnMut(char) -> Option<bool>,
) -> Option<bool> {
    let cased = CodePointSetData::new::<Cased>();
    let ignorable = CodePointSetData::new::<CaseIgnorable>();
    let mut preceded = false;
    let mut chars = text.chars();
    let mut processed = 0;
    while let Some(c) = chars.next() {
        if processed % INTERVAL == 0 && !current() {
            return None;
        }
        processed += 1;
        if c == 'Σ' && preceded {
            let mut followed = false;
            for (i, next) in chars.clone().enumerate() {
                if i % INTERVAL == 0 && !current() {
                    return None;
                }
                if !ignorable.contains(next) {
                    followed = cased.contains(next);
                    break;
                }
            }
            if emit(if followed { 'σ' } else { 'ς' })? {
                return Some(true);
            }
        } else {
            for lower in c.to_lowercase() {
                if emit(lower)? {
                    return Some(true);
                }
            }
        }
        if !ignorable.contains(c) {
            preceded = cased.contains(c);
        }
    }
    Some(false)
}

struct Needle {
    chars: Vec<char>,
    prefix: Vec<usize>,
}

impl Needle {
    fn new(term: &str, current: &impl Fn() -> bool) -> Option<Self> {
        let mut chars = Vec::new();
        let mut prefix = Vec::new();
        lowercase(term, current, |c| {
            let mut n = *prefix.last().unwrap_or(&0);
            while n > 0 && chars[n] != c {
                if !current() {
                    return None;
                }
                n = prefix[n - 1];
            }
            prefix.push(n + usize::from(chars.get(n) == Some(&c)));
            chars.push(c);
            Some(false)
        })?;
        Some(Self { chars, prefix })
    }

    fn matches(&self, cell: &str, current: &impl Fn() -> bool) -> Option<bool> {
        let mut n = 0;
        lowercase(cell, current, |c| {
            while n > 0 && self.chars[n] != c {
                if !current() {
                    return None;
                }
                n = self.prefix[n - 1];
            }
            n += usize::from(self.chars[n] == c);
            Some(n == self.chars.len())
        })
    }
}

/// A nonblank, untrimmed term against every source cell. None means cancelled.
pub fn matching_rows(
    page: &DecodedPage,
    term: &str,
    current: impl Fn() -> bool,
) -> Option<Arc<[usize]>> {
    let needle = Needle::new(term, &current)?;
    let mut rows = Vec::new();
    for (i, row) in page.iter().enumerate() {
        if !current() {
            return None;
        }
        for cell in row {
            if !current() {
                return None;
            }
            if needle.chars.is_empty() || needle.matches(cell.as_deref().unwrap_or(""), &current)? {
                rows.push(i);
                break;
            }
        }
    }
    Some(rows.into())
}
