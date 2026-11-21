use super::*;
use crate::workspace::toast::Toast;

/// Session history failures already reported, so only new ones toast while the panel is open.
#[derive(Default, PartialEq)]
pub(super) struct Seen {
    status: Option<String>,
    setting: Option<String>,
    clear: Option<String>,
}

impl Seen {
    pub(super) fn of(log: &HistoryLog) -> Self {
        Self {
            status: log.status_error.clone(),
            setting: log.setting_error.as_ref().map(|(_, error)| error.clone()),
            clear: log.clear_error.clone(),
        }
    }
}

impl Panel {
    pub(super) fn report(&self, action: &str, error: &str, cx: &mut App) {
        self.toasts
            .show(Toast::error(format!("{action}: {error}")), cx);
    }

    pub(super) fn report_log(&mut self, cx: &mut App) {
        let next = Seen::of(&self.updates.borrow());
        if next == self.seen {
            return;
        }
        let actions = [
            (
                "Loading history settings failed",
                &next.status,
                &self.seen.status,
            ),
            (
                "Changing history setting failed",
                &next.setting,
                &self.seen.setting,
            ),
            ("Clearing history failed", &next.clear, &self.seen.clear),
        ];
        for (action, error, seen) in actions {
            if let Some(error) = error.as_ref().filter(|error| Some(*error) != seen.as_ref()) {
                self.report(action, error, cx);
            }
        }
        self.seen = next;
    }
}
