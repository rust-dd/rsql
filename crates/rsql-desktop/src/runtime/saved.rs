use super::Session;
use rsql_core::{ApplicationServices, storage::query};
use rsql_desktop::saved::{SavedQuery, SavedQueryKey};
use std::{collections::BTreeMap, sync::Arc};
use tokio::sync::watch;

type Rows = BTreeMap<SavedQueryKey, Arc<SavedQuery>>;

#[derive(Default)]
pub(crate) struct SavedLibrary {
    pub rows: Rows,
    load_request: u64,
    write_revision: u64,
    pub loading: bool,
    pub load_error: Option<String>,
    pub save_failures: BTreeMap<SavedQueryKey, SaveFailure>,
    pub delete_failures: BTreeMap<SavedQueryKey, String>,
}

pub(crate) struct SaveFailure {
    query: Arc<SavedQuery>,
    pub error: String,
}

enum SavedWrite {
    Save(Arc<SavedQuery>),
    Delete(SavedQueryKey),
}

async fn load(services: &ApplicationServices) -> Result<Rows, String> {
    let db = services.local_db.connect().map_err(|e| e.to_string())?;
    let rows = query::select_queries(&db)
        .await
        .map_err(|e| e.to_string())?;
    Ok(rows
        .into_iter()
        .filter_map(|(id, sql)| SavedQuery::decode(&id, sql))
        .map(|query| (query.key.clone(), Arc::new(query)))
        .collect())
}

impl Session {
    pub(crate) fn saved_queries(&self) -> watch::Ref<'_, SavedLibrary> {
        self.saved.borrow()
    }

    pub(crate) fn saved_updates(&self) -> watch::Receiver<SavedLibrary> {
        self.saved.subscribe()
    }

    pub(crate) fn refresh_saved_queries(&self) {
        let services = self.services.clone();
        self.refresh_saved_queries_with(move || {
            let services = services.clone();
            async move { load(&services).await }
        });
    }

    fn refresh_saved_queries_with<F>(&self, mut read: impl FnMut() -> F + Send + 'static)
    where
        F: Future<Output = Result<Rows, String>> + Send + 'static,
    {
        if self.stopped.get() {
            return;
        }
        let previous = self.saved_load.take();
        if let Some(job) = &previous {
            job.abort();
        }
        let mut request = 0;
        self.saved.send_modify(|library| {
            library.load_request += 1;
            request = library.load_request;
            library.loading = true;
            library.load_error = None;
        });
        let saved = self.saved.clone();
        self.saved_load.replace(Some(self.runtime.spawn(async move {
            if let Some(job) = previous {
                let _ = job.await;
            }
            loop {
                let revision = saved.borrow().write_revision;
                let result = read().await;
                let mut retry = false;
                saved.send_if_modified(|library| {
                    if library.load_request != request {
                        return false;
                    }
                    if library.write_revision != revision {
                        retry = true;
                        return false;
                    }
                    library.loading = false;
                    match result {
                        Ok(rows) => library.rows = rows,
                        Err(error) => library.load_error = Some(error),
                    }
                    true
                });
                if !retry {
                    break;
                }
                // A fresh read keeps unrelated rows when a write raced the initial load.
                tokio::task::yield_now().await;
            }
        })));
    }

    pub(crate) fn save_query(&self, query: Arc<SavedQuery>) {
        self.write_saved_query(move |_| Some(SavedWrite::Save(query)));
    }

    pub(crate) fn delete_query(&self, key: SavedQueryKey) {
        self.write_saved_query(move |_| Some(SavedWrite::Delete(key)));
    }

    pub(crate) fn retry_saved_save(&self, key: SavedQueryKey) {
        self.write_saved_query(move |library| {
            Some(SavedWrite::Save(
                library.save_failures.get(&key)?.query.clone(),
            ))
        });
    }

    pub(crate) fn retry_saved_delete(&self, key: SavedQueryKey) {
        self.write_saved_query(move |library| {
            library
                .delete_failures
                .contains_key(&key)
                .then_some(SavedWrite::Delete(key))
        });
    }

    fn write_saved_query(
        &self,
        resolve: impl FnOnce(&SavedLibrary) -> Option<SavedWrite> + Send + 'static,
    ) {
        let services = self.services.clone();
        let saved = self.saved.clone();
        self.spawn_profile_write(async move {
            // Recheck queued retries after any earlier write for the same key.
            let write = resolve(&saved.borrow());
            match write {
                Some(SavedWrite::Save(saved_query)) => {
                    let id = saved_query.key.encode();
                    let result = async {
                        query::upsert_query(&services.local_db.connect()?, &id, &saved_query.sql)
                            .await
                    }
                    .await;
                    let key = saved_query.key.clone();
                    saved.send_modify(|library| match result {
                        Ok(()) => {
                            library.write_revision += 1;
                            library.save_failures.remove(&key);
                            library.rows.insert(key, saved_query);
                        }
                        Err(error) => {
                            library.save_failures.insert(
                                key,
                                SaveFailure {
                                    query: saved_query,
                                    error: error.to_string(),
                                },
                            );
                        }
                    });
                }
                Some(SavedWrite::Delete(key)) => {
                    let result = async {
                        query::delete_query(&services.local_db.connect()?, &key.encode()).await
                    }
                    .await;
                    saved.send_modify(|library| match result {
                        Ok(()) => {
                            library.write_revision += 1;
                            library.rows.remove(&key);
                            library.delete_failures.remove(&key);
                        }
                        Err(error) => {
                            library.delete_failures.insert(key, error.to_string());
                        }
                    });
                }
                None => {}
            }
        });
    }
}
