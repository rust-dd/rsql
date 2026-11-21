use super::Session;
use rsql_core::{
    ApplicationServices,
    storage::history::{self, HistoryStatus, NewHistoryEntry, RETENTION},
};
use std::{collections::VecDeque, sync::Arc, time::SystemTime};
use tokio::sync::watch;

const SESSION_LIMIT: usize = 500;

pub(crate) struct SessionEntry {
    pub sequence: u64,
    pub entry: Arc<NewHistoryEntry>,
    pub error: Option<String>,
}

#[derive(Default)]
pub(crate) struct HistoryLog {
    pub entries: VecDeque<SessionEntry>,
    pub status: Option<HistoryStatus>,
    pub status_error: Option<String>,
    pub clear_error: Option<String>,
    pending_settings: usize,
    pub setting_error: Option<(bool, String)>,
    pub record_error: Option<String>,
    pub revision: u64,
    sequence: u64,
}

impl HistoryLog {
    pub(crate) fn next_recording_setting(&self) -> Option<bool> {
        self.status
            .as_ref()
            .filter(|_| self.pending_settings == 0 && self.status_error.is_none())
            .map(|status| !status.enabled)
    }

    pub(crate) fn footer(&self) -> String {
        match &self.status {
            _ if self.status_error.is_some() => "History unavailable".into(),
            Some(status) if !status.enabled => "History off".into(),
            Some(status) => format!(
                "{} queries in history",
                crate::desktop::notifications::localized_row_count(
                    usize::try_from(status.entries).unwrap_or(0)
                )
            ),
            None => String::new(),
        }
    }

    fn accept(&mut self, entry: Arc<NewHistoryEntry>) -> u64 {
        self.sequence += 1;
        self.entries.push_front(SessionEntry {
            sequence: self.sequence,
            entry,
            error: None,
        });
        self.entries.truncate(SESSION_LIMIT);
        self.sequence
    }

    pub(super) fn publish_status(&mut self, result: Result<HistoryStatus, String>) {
        match result {
            Ok(status) => {
                self.status = Some(status);
                self.status_error = None;
            }
            Err(error) => self.status_error = Some(error),
        }
    }
}

pub(crate) fn millis(time: SystemTime) -> i64 {
    time.duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}

pub(super) async fn status(services: &ApplicationServices) -> Result<HistoryStatus, String> {
    async { history::status(&services.local_db.connect()?, RETENTION).await }
        .await
        .map_err(|e| e.to_string())
}

impl Session {
    pub(crate) fn history(&self) -> watch::Ref<'_, HistoryLog> {
        self.history.borrow()
    }

    pub(crate) fn history_footer(&self) -> (Option<String>, String) {
        let log = self.history();
        (log.record_error.clone(), log.footer())
    }

    pub(crate) fn history_updates(&self) -> watch::Receiver<HistoryLog> {
        self.history.subscribe()
    }

    pub(crate) fn refresh_history_status(&self) {
        let services = self.services.clone();
        let log = self.history.clone();
        self.spawn_profile_write(async move {
            let result = status(&services).await;
            log.send_modify(|log| log.publish_status(result));
        });
    }

    pub(crate) fn accept_history(&self, entry: NewHistoryEntry) {
        let entry = Arc::new(entry);
        let mut sequence = 0;
        self.history
            .send_modify(|log| sequence = log.accept(entry.clone()));
        self.write_history(sequence, Some(entry));
    }

    pub(crate) fn retry_history_record(&self, sequence: u64) {
        self.write_history(sequence, None);
    }

    fn write_history(&self, sequence: u64, new: Option<Arc<NewHistoryEntry>>) {
        let services = self.services.clone();
        let log = self.history.clone();
        self.spawn_profile_write(async move {
            let entry = new.or_else(|| {
                log.borrow()
                    .entries
                    .iter()
                    .find(|e| e.sequence == sequence && e.error.is_some())
                    .map(|e| e.entry.clone())
            });
            let Some(entry) = entry else { return };
            let result = async {
                history::record(
                    &services.local_db.connect()?,
                    &entry,
                    RETENTION,
                    millis(SystemTime::now()),
                )
                .await
            }
            .await;
            let error = match result {
                Ok(false) => return,
                Ok(true) => None,
                Err(e) => Some(format!("Could not record history: {e}")),
            };
            let status = status(&services).await;
            log.send_modify(|log| {
                if error.is_none() {
                    log.revision += 1;
                }
                if let Some(row) = log.entries.iter_mut().find(|e| e.sequence == sequence) {
                    row.error.clone_from(&error);
                }
                log.record_error = error;
                log.publish_status(status);
            });
        });
    }

    pub(crate) fn set_history_enabled(&self, enabled: bool) {
        self.write_history_setting(Some(enabled));
    }

    pub(crate) fn retry_history_setting(&self) {
        self.write_history_setting(None);
    }

    fn write_history_setting(&self, requested: Option<bool>) {
        self.history.send_modify(|log| log.pending_settings += 1);
        let services = self.services.clone();
        let log = self.history.clone();
        self.spawn_profile_write(async move {
            let enabled = requested.or_else(|| {
                log.borrow()
                    .setting_error
                    .as_ref()
                    .map(|(enabled, _)| *enabled)
            });
            let Some(enabled) = enabled else {
                log.send_modify(|log| log.pending_settings -= 1);
                return;
            };
            let result =
                async { history::set_enabled(&services.local_db.connect()?, enabled).await }.await;
            let status = status(&services).await;
            log.send_modify(|log| {
                log.pending_settings -= 1;
                log.setting_error = result.err().map(|e| (enabled, e.to_string()));
                log.publish_status(status);
            });
        });
    }

    pub(crate) fn clear_history(&self) {
        let boundary = self.history.borrow().sequence;
        let services = self.services.clone();
        let log = self.history.clone();
        self.spawn_profile_write(async move {
            let result = async { history::clear(&services.local_db.connect()?, None).await }.await;
            let status = status(&services).await;
            log.send_modify(|log| {
                match result {
                    Ok(()) => {
                        log.entries.retain(|e| e.sequence > boundary);
                        log.revision += 1;
                        log.clear_error = None;
                        log.record_error = None;
                    }
                    Err(e) => log.clear_error = Some(e.to_string()),
                }
                log.publish_status(status);
            });
        });
    }
}
