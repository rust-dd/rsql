use gpui_kit::{
    Context,
    component::input::{EditorState, InputEdit, InputEditsEvent, Rope},
};
use std::{collections::BTreeMap, ops::Range};

pub(super) struct Snippet {
    // Ranges stay in source order, including empty stops at the same byte.
    ranges: Vec<Range<usize>>,
    groups: Vec<Vec<usize>>,
    primary: usize,
    at_start: bool,
    pending_edits: Vec<InputEdit>,
    active: usize,
    inserted: Option<Rope>,
    text: Rope,
}

impl Snippet {
    pub fn expand(template: &str, start: usize) -> (String, Self) {
        let mut result = String::new();
        let mut groups = BTreeMap::<usize, Vec<usize>>::new();
        let mut ranges = Vec::new();
        let mut defaults = BTreeMap::new();
        let mut chars = template.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '\\' && chars.peek().is_some_and(|c| matches!(c, '$' | '}' | '\\')) {
                result.push(chars.next().unwrap());
                continue;
            }
            if ch != '$' {
                result.push(ch);
                continue;
            }
            let mut token = chars.clone();
            let braced = token.next_if_eq(&'{').is_some();
            let mut digits = String::new();
            while let Some(n) = token.next_if(char::is_ascii_digit) {
                digits.push(n);
            }
            let Ok(index) = digits.parse::<usize>() else {
                result.push(ch);
                continue;
            };
            let mut value = String::new();
            if braced {
                if token.next_if_eq(&':').is_some() {
                    while token.peek().is_some_and(|c| *c != '}') {
                        let mut c = token.next().unwrap();
                        if c == '\\' && token.peek().is_some_and(|v| matches!(*v, '$' | '}' | '\\'))
                        {
                            c = token.next().unwrap();
                        }
                        value.push(c);
                    }
                }
                if token.next_if_eq(&'}').is_none() {
                    result.push(ch);
                    continue;
                }
            }
            chars = token;
            let from = start + result.len();
            result.push_str(defaults.entry(index).or_insert(value));
            groups.entry(index).or_default().push(ranges.len());
            ranges.push(from..start + result.len());
        }
        let final_group = groups.remove(&0).unwrap_or_else(|| {
            let end = start + result.len();
            ranges.push(end..end);
            vec![ranges.len() - 1]
        });
        let mut groups: Vec<_> = groups.into_values().collect();
        groups.push(final_group);
        (
            result,
            Self {
                ranges,
                groups,
                primary: 0,
                at_start: true,
                pending_edits: Vec::new(),
                active: 0,
                inserted: None,
                text: Rope::new(),
            },
        )
    }

    pub fn begin(&mut self, state: &mut EditorState, cx: &mut Context<EditorState>) -> bool {
        self.text = state.text().clone();
        self.inserted = Some(self.text.clone());
        self.select(state, cx)
    }

    fn select(&mut self, state: &mut EditorState, cx: &mut Context<EditorState>) -> bool {
        let group = &self.groups[self.active];
        self.primary = group[0];
        self.at_start = true;
        state.set_selected_range(self.ranges[self.primary].clone(), cx);
        let peers = group[1..].iter().map(|&i| self.ranges[i].clone()).collect();
        state.replace_secondary_selections(peers, cx) && self.active + 1 < self.groups.len()
    }

    /// Returns false when the peers no longer form a valid linked selection.
    pub fn synchronize_selection(
        &mut self,
        state: &mut EditorState,
        cx: &mut Context<EditorState>,
    ) -> bool {
        let group = &self.groups[self.active];
        if group.len() < 2 || !self.pending_edits.is_empty() || *state.text() != self.text {
            return true;
        }
        let selected = state.selected_range();
        let contains = |&i: &usize| {
            let stop = &self.ranges[i];
            stop.start <= selected.start && selected.end <= stop.end
        };
        let mut candidates = group.iter().copied().filter(contains);
        let Some(first) = candidates.next() else {
            return true;
        };
        self.primary = if let Some(second) = candidates.next() {
            // Undo can restore the other side of a shared endpoint. Recover it
            // from the native peers before synchronizing any selections.
            let mut native: Vec<_> = state.secondary_selection_ranges().collect();
            native.sort_unstable_by_key(|r| (r.start, r.end));
            let preferred = if contains(&self.primary) {
                self.primary
            } else {
                first
            };
            [first, second]
                .into_iter()
                .chain(candidates)
                .find(|&i| {
                    native.len() + 1 == group.len()
                        && self.peer_ranges(i, &selected).as_ref() == Some(&native)
                })
                .unwrap_or(preferred)
        } else {
            first
        };
        self.at_start = selected.start == self.ranges[self.primary].start;
        let peers = self.peer_ranges(self.primary, &selected);
        if peers.is_some_and(|peers| state.replace_secondary_selections(peers, cx)) {
            return true;
        }
        // Invalidated stops must not direct the next user edit to a stale peer.
        state.replace_secondary_selections(Vec::new(), cx);
        false
    }

    fn peer_ranges(&self, primary: usize, selected: &Range<usize>) -> Option<Vec<Range<usize>>> {
        let start = selected.start - self.ranges[primary].start;
        let end = selected.end - self.ranges[primary].start;
        self.groups[self.active]
            .iter()
            .copied()
            .filter(|&i| i != primary)
            .map(|i| {
                let stop = &self.ranges[i];
                (end <= stop.len()).then(|| stop.start + start..stop.start + end)
            })
            .collect()
    }

    pub fn contains(&self, caret: usize) -> bool {
        self.groups[self.active].iter().any(|&i| {
            let r = &self.ranges[i];
            r.start <= caret && caret <= r.end
        })
    }

    pub fn step(
        &mut self,
        reverse: bool,
        state: &mut EditorState,
        cx: &mut Context<EditorState>,
    ) -> Option<bool> {
        if !self.contains(state.cursor()) {
            return None;
        }
        self.active = if reverse {
            self.active.saturating_sub(1)
        } else {
            self.active + 1
        };
        Some(self.select(state, cx))
    }

    pub fn edited(&mut self, event: &InputEditsEvent, current: &Rope) -> bool {
        // History can publish the members of one gesture in separate events.
        // Wait for its current text before reconciling the restored selection.
        for (edit, after) in event.edits() {
            self.text = after.clone();
            if self.inserted.take().as_ref() != Some(after) {
                self.pending_edits.push(*edit);
            }
        }
        if !super::editor_services::same_text(&self.text, current) {
            return true;
        }
        let mut edits = std::mem::take(&mut self.pending_edits);
        let valid = self.apply_edits(&edits);
        edits.clear();
        self.pending_edits = edits;
        valid
    }

    fn apply_edits(&mut self, mut edits: &[InputEdit]) -> bool {
        let count = self.groups[self.active].len();
        while let Some((first, rest)) = edits.split_first() {
            if count > 1
                && let Some(batch) = edits.get(..count)
            {
                // Native edits and Redo visit descending offsets. Undo replays
                // the same gesture in reverse and shifts later offsets as it goes.
                let descending = self.matches_batch(batch, true);
                if descending || self.matches_batch(batch, false) {
                    for (n, edit) in batch.iter().enumerate() {
                        let i = if descending { count - 1 - n } else { n };
                        self.apply_edit(self.groups[self.active][i], edit);
                    }
                    edits = &edits[count..];
                    continue;
                }
            }
            let mut owners = self.groups[self.active].iter().copied().filter(|&i| {
                let r = &self.ranges[i];
                r.start <= first.start_byte && first.old_end_byte <= r.end
            });
            let owner = if self.at_start {
                owners.next_back()
            } else {
                owners.next()
            };
            let Some(owner) = owner else { return false };
            self.apply_edit(owner, first);
            edits = rest;
        }
        true
    }

    fn matches_batch(&self, batch: &[InputEdit], descending: bool) -> bool {
        let group = &self.groups[self.active];
        let mut shift = 0isize;
        let mut relative = None;
        batch.iter().enumerate().all(|(n, edit)| {
            let i = if descending { group.len() - 1 - n } else { n };
            let stop = &self.ranges[group[i]];
            let start = stop.start.saturating_add_signed(shift);
            if edit.start_byte < start || edit.old_end_byte > stop.end.saturating_add_signed(shift)
            {
                return false;
            }
            let local = (edit.start_byte - start, edit.old_end_byte - start);
            if !descending {
                shift += edit.new_end_byte as isize - edit.old_end_byte as isize;
            }
            *relative.get_or_insert(local) == local
        })
    }

    fn apply_edit(&mut self, owner: usize, edit: &InputEdit) {
        let delta = edit.new_end_byte as isize - edit.old_end_byte as isize;
        self.ranges[owner].end = self.ranges[owner].end.saturating_add_signed(delta);
        for r in &mut self.ranges[owner + 1..] {
            r.start = r.start.saturating_add_signed(delta);
            r.end = r.end.saturating_add_signed(delta);
        }
    }
}
