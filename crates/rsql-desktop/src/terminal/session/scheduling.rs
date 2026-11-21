use super::super::{FeedOutcome, FeedStatus, MAX_FEED_BYTES, TerminalModel};

#[derive(Default)]
pub(super) struct Turn(usize);

impl Turn {
    pub fn feed(&mut self, model: &mut TerminalModel, bytes: &[u8]) -> (FeedOutcome, bool) {
        let remaining = MAX_FEED_BYTES - self.0;
        let outcome = model.feed(&bytes[..bytes.len().min(remaining)]);
        self.0 += outcome.processed;
        let must_yield = self.0 == MAX_FEED_BYTES
            || outcome.synchronized_flush
            || outcome.status == FeedStatus::Yield;
        if must_yield {
            self.0 = 0;
        }
        (outcome, must_yield)
    }
}
