use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::watch;

use crate::common::enums::AppError;

#[derive(Default)]
struct Entries {
    active: HashMap<String, (String, watch::Sender<bool>)>,
    early_cancellations: HashMap<String, Instant>,
}

#[derive(Default)]
pub(crate) struct QueryExecutions(Mutex<Entries>);

pub(crate) struct Execution {
    registry: Arc<QueryExecutions>,
    id: String,
    pub cancelled: watch::Receiver<bool>,
}

impl QueryExecutions {
    pub fn start(self: &Arc<Self>, id: &str, project_id: &str) -> Result<Execution, AppError> {
        let mut entries = self.0.lock().expect("query registry poisoned");
        if entries.active.contains_key(id) {
            return Err(AppError::QueryFailed(
                "Execution ID is already in use".into(),
            ));
        }
        entries
            .early_cancellations
            .retain(|_, time| time.elapsed() < Duration::from_secs(60));
        let cancelled = entries.early_cancellations.remove(id).is_some();
        let (sender, receiver) = watch::channel(cancelled);
        entries
            .active
            .insert(id.to_owned(), (project_id.to_owned(), sender));
        Ok(Execution {
            registry: Arc::clone(self),
            id: id.to_owned(),
            cancelled: receiver,
        })
    }

    pub fn cancel(&self, id: &str) -> bool {
        let mut entries = self.0.lock().expect("query registry poisoned");
        if let Some((_, sender)) = entries.active.get(id) {
            sender.send_replace(true);
            true
        } else {
            // Stop can arrive before its execution command is dispatched.
            entries
                .early_cancellations
                .retain(|_, time| time.elapsed() < Duration::from_secs(60));
            if entries.early_cancellations.len() >= 1024 {
                entries.early_cancellations.clear();
            }
            entries
                .early_cancellations
                .insert(id.to_owned(), Instant::now());
            false
        }
    }

    pub fn cancel_project(&self, project_id: &str) {
        let entries = self.0.lock().expect("query registry poisoned");
        for (project, sender) in entries.active.values() {
            if project == project_id {
                sender.send_replace(true);
            }
        }
    }
}

impl Drop for Execution {
    fn drop(&mut self) {
        self.registry
            .0
            .lock()
            .expect("query registry poisoned")
            .active
            .remove(&self.id);
    }
}

pub(crate) async fn wait_for_cancel(receiver: &mut watch::Receiver<bool>) {
    loop {
        if *receiver.borrow_and_update() {
            return;
        }
        if receiver.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancellation_is_scoped_and_entries_are_released() {
        let registry = Arc::new(QueryExecutions::default());
        let first = registry.start("first", "project").unwrap();
        let second = registry.start("second", "project").unwrap();
        assert!(registry.cancel("first"));
        assert!(*first.cancelled.borrow());
        assert!(!*second.cancelled.borrow());
        drop(first);
        drop(second);
        assert!(registry.0.lock().unwrap().active.is_empty());
    }

    #[test]
    fn cancellation_before_registration_is_remembered() {
        let registry = Arc::new(QueryExecutions::default());
        registry.cancel("early");
        assert!(
            *registry
                .start("early", "project")
                .unwrap()
                .cancelled
                .borrow()
        );
    }
}
