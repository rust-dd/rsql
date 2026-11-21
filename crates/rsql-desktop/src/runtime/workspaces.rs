use super::Session;
use ropey::Rope;
use rsql_core::{ApplicationServices, storage::workspace};
use rsql_desktop::named_workspace::{self, NamedTab};
use std::{collections::BTreeMap, sync::Arc};
use tokio::{sync::watch, task::JoinHandle};
use uuid::Uuid;

pub(crate) enum CapturedSql {
    Retained(Rope),
    Draft(String),
}

pub(crate) type Capture = Arc<[NamedTab<CapturedSql>]>;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum WriteKind {
    Save,
    Delete,
}

impl WriteKind {
    pub(crate) fn verb(self) -> &'static str {
        match self {
            Self::Save => "save",
            Self::Delete => "delete",
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct WorkspaceTarget {
    pub name: String,
    lifetime: Uuid,
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) enum WriteTarget {
    Save(String),
    Delete(WorkspaceTarget),
}

impl WriteTarget {
    pub(crate) fn name(&self) -> &str {
        match self {
            Self::Save(name) => name,
            Self::Delete(target) => &target.name,
        }
    }

    pub(crate) fn kind(&self) -> WriteKind {
        match self {
            Self::Save(_) => WriteKind::Save,
            Self::Delete(_) => WriteKind::Delete,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum WriteStatus {
    Committed,
    Superseded,
}

pub(crate) type WriteTask = JoinHandle<Result<WriteStatus, String>>;

enum Write {
    Save(Capture),
    Delete,
}

pub(crate) struct SaveFailure {
    snapshot: Capture,
    pub error: String,
}

pub(crate) struct Entry {
    lifetime: Uuid,
    pub tabs: Result<Arc<[NamedTab]>, String>,
    tab_label: String,
}

impl Entry {
    fn new(tabs: Result<Vec<NamedTab>, String>) -> Self {
        let tab_label = match tabs.as_ref().map(Vec::len) {
            Ok(1) => "1 tab".into(),
            Ok(count) => format!("{count} tabs"),
            Err(_) => "Invalid workspace".into(),
        };
        Self {
            lifetime: Uuid::new_v4(),
            tabs: tabs.map(Arc::from),
            tab_label,
        }
    }

    pub(crate) fn target(&self, name: &str) -> WorkspaceTarget {
        WorkspaceTarget {
            name: name.into(),
            lifetime: self.lifetime,
        }
    }

    pub(crate) fn tab_label(&self) -> &str {
        &self.tab_label
    }
}

type Rows = BTreeMap<String, Entry>;

#[derive(Default)]
pub(crate) struct Library {
    pub rows: Rows,
    pub loading: bool,
    pub load_error: Option<String>,
    pub save_failures: BTreeMap<String, SaveFailure>,
    pub delete_failures: BTreeMap<String, String>,
    loaded: bool,
    write_revision: u64,
}

impl Library {
    pub(crate) fn entry(&self, target: &WorkspaceTarget) -> Option<&Entry> {
        self.rows
            .get(&target.name)
            .filter(|row| row.lifetime == target.lifetime)
    }

    fn publish_rows(&mut self, mut rows: Rows) {
        for (name, entry) in &mut rows {
            if let Some(old) = self.rows.get(name) {
                entry.lifetime = old.lifetime;
            }
        }
        self.delete_failures
            .retain(|name, _| rows.contains_key(name));
        self.rows = rows;
        self.loaded = true;
    }

    pub(crate) fn write_error(&self, kind: WriteKind, name: &str) -> Option<&str> {
        match kind {
            WriteKind::Save => self
                .save_failures
                .get(name)
                .map(|failure| failure.error.as_str()),
            WriteKind::Delete => self.delete_failures.get(name).map(String::as_str),
        }
    }
}

async fn store_save(
    services: &ApplicationServices,
    name: &str,
    snapshot: &Capture,
) -> Result<Vec<NamedTab>, String> {
    let tabs = snapshot
        .iter()
        .map(|tab| {
            tab.with_text(|sql| match sql {
                CapturedSql::Retained(text) => text.to_string(),
                CapturedSql::Draft(text) => text.clone(),
            })
        })
        .collect::<Vec<_>>();
    let json = named_workspace::encode_tabs(&tabs)?;
    let conn = services
        .local_db
        .connect()
        .map_err(|error| error.to_string())?;
    workspace::save_workspace(&conn, name, &json)
        .await
        .map_err(|error| error.to_string())?;
    Ok(tabs)
}

async fn load(services: &ApplicationServices) -> Result<Rows, String> {
    let conn = services
        .local_db
        .connect()
        .map_err(|error| error.to_string())?;
    let rows = workspace::load_workspaces(&conn)
        .await
        .map_err(|error| error.to_string())?;
    let mut entries = Rows::new();
    for (name, source) in rows {
        tokio::task::yield_now().await;
        entries.insert(name, Entry::new(named_workspace::decode(&source)));
    }
    Ok(entries)
}

impl Session {
    pub(crate) fn named_workspaces(&self) -> watch::Ref<'_, Library> {
        self.named_workspaces.borrow()
    }

    pub(crate) fn named_workspace_updates(&self) -> watch::Receiver<Library> {
        self.named_workspaces.subscribe()
    }

    pub(crate) fn ensure_named_workspaces(&self) {
        if !self.named_workspaces.borrow().loaded {
            self.refresh_named_workspaces();
        }
    }

    pub(crate) fn refresh_named_workspaces(&self) {
        let services = self.services.clone();
        self.refresh_named_workspaces_with(move || {
            let services = services.clone();
            async move { load(&services).await }
        });
    }

    fn refresh_named_workspaces_with<F>(&self, mut read: impl FnMut() -> F + Send + 'static)
    where
        F: Future<Output = Result<Rows, String>> + Send + 'static,
    {
        if self.stopped.get() || self.named_workspaces.borrow().loading {
            return;
        }
        self.named_workspaces.send_modify(|library| {
            library.loading = true;
            library.load_error = None;
        });
        let workspaces = self.named_workspaces.clone();
        let job = self.runtime.spawn(async move {
            loop {
                let revision = workspaces.borrow().write_revision;
                let result = read().await;
                let published = workspaces.send_if_modified(|library| {
                    if library.write_revision != revision {
                        return false;
                    }
                    library.loading = false;
                    match result {
                        Ok(rows) => library.publish_rows(rows),
                        Err(error) => library.load_error = Some(error),
                    }
                    true
                });
                if published {
                    break;
                }
                // A fresh read merges unrelated stored rows with the completed write.
                tokio::task::yield_now().await;
            }
        });
        self.named_workspace_load.replace(Some(job));
    }

    pub(crate) fn save_named_workspace(&self, name: String, snapshot: Capture) -> WriteTask {
        self.write_named_workspace(name, move |_, _| Some(Write::Save(snapshot)))
    }

    pub(crate) fn delete_named_workspace(&self, target: WorkspaceTarget) -> WriteTask {
        self.write_named_workspace(target.name.clone(), move |library, _| {
            library.entry(&target).map(|_| Write::Delete)
        })
    }

    pub(crate) fn retry_named_workspace(&self, target: WriteTarget) -> WriteTask {
        self.write_named_workspace(target.name().into(), move |library, name| match target {
            WriteTarget::Save(_) => Some(Write::Save(
                library.save_failures.get(name)?.snapshot.clone(),
            )),
            WriteTarget::Delete(target) => {
                library.entry(&target)?;
                library
                    .delete_failures
                    .contains_key(name)
                    .then_some(Write::Delete)
            }
        })
    }

    fn write_named_workspace(
        &self,
        name: String,
        resolve: impl FnOnce(&Library, &str) -> Option<Write> + Send + 'static,
    ) -> WriteTask {
        let services = self.services.clone();
        let workspaces = self.named_workspaces.clone();
        self.spawn_profile_write(async move {
            let write = resolve(&workspaces.borrow(), &name);
            match write {
                Some(Write::Save(snapshot)) => {
                    match store_save(&services, &name, &snapshot).await {
                        Ok(tabs) => {
                            let mut entry = Entry::new(Ok(tabs));
                            workspaces.send_modify(|library| {
                                library.write_revision += 1;
                                library.save_failures.remove(&name);
                                if let Some(old) = library.rows.get(&name) {
                                    entry.lifetime = old.lifetime;
                                }
                                library.rows.insert(name, entry);
                            });
                        }
                        Err(error) => {
                            workspaces.send_modify(|library| {
                                library.save_failures.insert(
                                    name,
                                    SaveFailure {
                                        snapshot,
                                        error: error.clone(),
                                    },
                                );
                            });
                            return Err(error);
                        }
                    }
                }
                Some(Write::Delete) => {
                    let result = async {
                        workspace::delete_workspace(&services.local_db.connect()?, &name).await
                    }
                    .await;
                    match result {
                        Ok(()) => {
                            workspaces.send_modify(|library| {
                                library.write_revision += 1;
                                library.delete_failures.remove(&name);
                                library.rows.remove(&name);
                            });
                        }
                        Err(error) => {
                            let error = error.to_string();
                            workspaces.send_modify(|library| {
                                library.delete_failures.insert(name, error.clone());
                            });
                            return Err(error);
                        }
                    }
                }
                None => return Ok(WriteStatus::Superseded),
            }
            Ok(WriteStatus::Committed)
        })
    }
}
