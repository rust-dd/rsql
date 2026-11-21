use crate::terminal::{HISTORY_LINES, session::CommandError};
use std::collections::VecDeque;

const LIMIT: i32 = HISTORY_LINES as i32;
const MAX_RUNS: usize = 2 * HISTORY_LINES;

#[derive(Default)]
pub(super) struct PendingScroll {
    runs: VecDeque<i16>,
}

impl PendingScroll {
    pub fn push(&mut self, delta: i32) {
        let mut delta = delta.clamp(-LIMIT, LIMIT);
        if delta == 0 {
            return;
        }
        while let Some(&last) = self.runs.back() {
            let last = i32::from(last);
            if last.signum() == delta.signum() {
                self.runs.pop_back();
                delta = (last + delta).clamp(-LIMIT, LIMIT);
                continue;
            }
            let Some(&previous) = self.runs.iter().rev().nth(1) else {
                break;
            };
            let previous = i32::from(previous);
            if last.abs() > previous.abs() || last.abs() > delta.abs() {
                break;
            }
            // Both outer moves cover the reversal, even when the first hits a boundary.
            self.runs.truncate(self.runs.len() - 2);
            delta = (previous + last + delta).clamp(-LIMIT, LIMIT);
        }
        // Remaining magnitudes strictly rise, then fall, with at most two equal peaks.
        debug_assert!(self.runs.len() < MAX_RUNS);
        if self.runs.len() == self.runs.capacity() {
            let extra = self.runs.len().max(4).min(MAX_RUNS - self.runs.len());
            self.runs.reserve_exact(extra);
        }
        self.runs.push_back(delta as i16);
    }

    pub fn drain(&mut self, mut submit: impl FnMut(i32) -> Result<(), CommandError>) {
        for _ in 0..32 {
            let Some(&delta) = self.runs.front() else {
                break;
            };
            match submit(i32::from(delta)) {
                Ok(()) => {
                    self.runs.pop_front();
                }
                Err(CommandError::Busy) => break,
                Err(CommandError::Closed | CommandError::TooLarge) => {
                    self.runs.clear();
                    break;
                }
            }
        }
    }

    pub fn is_pending(&self) -> bool {
        !self.runs.is_empty()
    }
}
