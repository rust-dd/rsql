use crate::terminal::{
    TermMode,
    session::{CommandError, MAX_INPUT_BYTES, TerminalSession},
};

#[derive(Default)]
pub(super) struct PendingReports {
    focus: Vec<u8>,
    release: Option<Release>,
}

struct Release {
    bytes: String,
    generation: u64,
    earlier_focus: usize,
}

impl PendingReports {
    pub fn is_pending(&self) -> bool {
        !self.focus.is_empty() || self.release.is_some()
    }

    pub fn focus(&mut self, report: &[u8; 3]) {
        if self.focus.len() + report.len() > MAX_INPUT_BYTES {
            if let Some(release) = &mut self.release
                && release.earlier_focus == self.focus.len()
            {
                // Collapse a pair on the same side of the release to preserve its focus state.
                release.earlier_focus -= 2 * report.len();
                self.focus.truncate(release.earlier_focus);
            } else {
                self.focus.truncate(self.focus.len() - report.len());
                return;
            }
        }
        self.focus.extend_from_slice(report);
    }

    pub fn release(&mut self, bytes: String, generation: u64) {
        debug_assert!(self.release.is_none());
        self.release = Some(Release {
            bytes,
            generation,
            earlier_focus: self.focus.len(),
        });
    }

    pub fn flush(&mut self, session: &TerminalSession) {
        let focus = |bytes: &[u8]| {
            bytes.is_empty()
                || session.report_input(bytes, |mode, _| mode.contains(TermMode::FOCUS_IN_OUT))
                    != Err(CommandError::Busy)
        };
        if let Some(release) = &mut self.release {
            if !focus(&self.focus[..release.earlier_focus]) {
                return;
            }
            self.focus.drain(..release.earlier_focus);
            release.earlier_focus = 0;
            if session.report_input(release.bytes.as_bytes(), |mode, generation| {
                generation == release.generation
                    && mode.contains(TermMode::SGR_MOUSE)
                    && mode.intersects(TermMode::MOUSE_MODE)
            }) == Err(CommandError::Busy)
            {
                return;
            }
            self.release = None;
        }
        if focus(&self.focus) {
            self.focus.clear();
        }
    }
}
