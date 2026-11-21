pub(crate) mod diagrams;
pub(crate) mod history;
pub(crate) mod map;
pub(crate) mod resources;
mod saved;
pub(crate) mod workspaces;

use rsql_core::{ApplicationServices, migration::ProfileLease, storage};
use rsql_desktop::tiles::TileService;
use rsql_desktop::{connection::Connections, document::Documents, persistence};
use std::{
    cell::{Cell, RefCell},
    path::Path,
    rc::Rc,
    sync::Arc,
};
use tokio::{runtime::Handle, sync::watch};

pub struct Session {
    pub window_geometry: RefCell<Option<crate::desktop::geometry::Geometry>>,
    pub window_geometry_dirty: Cell<bool>,
    pub map_worker: map::Worker,
    pub tiles: TileService,
    pub updates: Rc<rsql_desktop::update::UpdateController>,
    pub terminals: RefCell<rsql_desktop::terminal::session::TerminalSessions>,
    diagrams: RefCell<Vec<std::rc::Weak<diagrams::LiveDiagram>>>,
    pub preflight_stop: watch::Sender<bool>,
    pub formatter: RefCell<crate::formatter::Formatter>,
    pub runner: rsql_desktop::execution::Runner,
    pub analysis: rsql_desktop::completion::worker::AnalysisWorker,
    pub runtime: Handle,
    pub services: Arc<ApplicationServices>,
    profile: Arc<ProfileLease>,
    pub(crate) resources: resources::Resources,
    pub routes: Arc<dyn crate::routes::Routes>,
    pub pin: RefCell<Option<rsql_desktop::results::diff::Pin>>,
    pub documents: RefCell<Documents>,
    pub connections: RefCell<Connections>,
    history: watch::Sender<history::HistoryLog>,
    pub load_error: RefCell<Option<String>>,
    saved: watch::Sender<saved::SavedLibrary>,
    saved_load: RefCell<Option<tokio::task::JoinHandle<()>>>,
    named_workspaces: watch::Sender<workspaces::Library>,
    named_workspace_load: RefCell<Option<tokio::task::JoinHandle<()>>>,
    profile_write: RefCell<Option<tokio::sync::oneshot::Receiver<()>>>,
    committed_removals: Arc<std::sync::Mutex<Vec<String>>>,
    snapshot: Arc<persistence::SnapshotFile>,
    writer: RefCell<Option<tokio::task::JoinHandle<()>>>,
    save_status: watch::Sender<Option<String>>,
    stopped: Cell<bool>,
}

impl Session {
    pub fn open(runtime: Handle, profile: &Path) -> Result<Rc<Self>, String> {
        Self::open_with_tiles(runtime, profile, |rt, lease| {
            TileService::open_profile(
                rt,
                lease,
                rsql_desktop::tiles::TileProvider::openstreetmap(),
            )
        })
    }

    fn open_with_tiles(
        runtime: Handle,
        profile: &Path,
        tiles: impl FnOnce(Handle, Arc<ProfileLease>) -> TileService,
    ) -> Result<Rc<Self>, String> {
        let directory = profile.to_owned();
        let loader = runtime.clone();
        let (profile, window_geometry, snapshot, mut documents, services, loaded, updates) =
            runtime
                .block_on(runtime.spawn_blocking(move || {
                    let mut lease = ProfileLease::acquire(&directory)
                        .map_err(|e| format!("Open profile {}: {e}", directory.display()))?;
                    rsql_desktop::migration::recover(&mut lease)
                        .map_err(|e| format!("Recover profile {}: {e}", directory.display()))?;
                    let lease = Arc::new(lease);
                    let path = lease.directory().join(persistence::FILENAME);
                    let (snapshot, documents) = persistence::SnapshotFile::open(&path)
                        .map_err(|e| format!("Load {}: {e}", path.display()))?;
                    let services = loader
                        .block_on(ApplicationServices::open_profile(lease.clone()))
                        .map_err(|e| {
                            format!("Initialize profile {}: {e}", lease.directory().display())
                        })?;
                    let loaded = loader
                        .block_on(async {
                            let db = services.local_db.connect()?;
                            storage::project::select_projects(&db).await
                        })
                        .map(Connections::from_rows)
                        .map_err(|e| format!("Load connections: {e}"));
                    let geometry = crate::desktop::geometry::Geometry::read(lease.directory());
                    let updates = rsql_desktop::update::UpdateController::packaged(loader);
                    Ok::<_, String>((
                        lease, geometry, snapshot, documents, services, loaded, updates,
                    ))
                }))
                .map_err(|e| format!("Profile loader failed: {e}"))??;
        let (connections, error) = match loaded {
            Ok(rows) => (rows, None),
            Err(e) => (Connections::default(), Some(e)),
        };
        documents.remap(&connections.legacy_ids);
        let services = Arc::new(services);
        let executable = std::env::current_exe().map_err(|e| e.to_string())?;
        let session = Rc::new(Self {
            window_geometry: RefCell::new(window_geometry),
            window_geometry_dirty: Cell::new(false),
            map_worker: Default::default(),
            updates: Rc::new(updates),
            tiles: tiles(runtime.clone(), profile.clone()),
            profile,
            terminals: RefCell::new(rsql_desktop::terminal::session::TerminalSessions::new(
                runtime.clone(),
            )),
            diagrams: Default::default(),
            preflight_stop: watch::channel(false).0,
            formatter: RefCell::new(crate::formatter::Formatter::new(&runtime, executable)),
            runner: rsql_desktop::execution::Runner::new(runtime.clone(), services.clone()),
            analysis: Default::default(),
            resources: resources::Resources::new(&runtime, services.clone()),
            history: watch::channel(history::HistoryLog::default()).0,
            pin: RefCell::default(),
            documents: RefCell::new(documents),
            connections: RefCell::new(connections),
            load_error: RefCell::new(error),
            saved: watch::channel(saved::SavedLibrary::default()).0,
            saved_load: Default::default(),
            named_workspaces: watch::channel(workspaces::Library::default()).0,
            named_workspace_load: Default::default(),
            profile_write: Default::default(),
            committed_removals: Default::default(),
            snapshot: Arc::new(snapshot),
            writer: Default::default(),
            runtime,
            routes: services.clone(),
            services,
            stopped: Cell::new(false),
            save_status: watch::channel(None).0,
        });
        session.refresh_history_status();
        session.refresh_saved_queries();
        Ok(session)
    }

    pub fn spawn_profile_write<T: Send + 'static>(
        &self,
        work: impl Future<Output = T> + Send + 'static,
    ) -> tokio::task::JoinHandle<T> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let profile = self.profile.clone();
        let previous = self.profile_write.replace(Some(rx));
        self.runtime.spawn(async move {
            if let Some(previous) = previous {
                let _ = previous.await;
            }
            let result = work.await;
            drop(profile);
            let _ = tx.send(());
            result
        })
    }

    pub fn delete_projects(
        &self,
        ids: Vec<String>,
    ) -> impl Future<Output = Result<(), String>> + Send + use<> {
        let services = self.services.clone();
        let committed = self.committed_removals.clone();
        let history = self.history.clone();
        async move {
            let db = services.local_db.connect().map_err(|e| e.to_string())?;
            storage::project::delete_projects(&db, &ids)
                .await
                .map_err(|e| e.to_string())?;
            let status = history::status(&services).await;
            history.send_modify(|log| {
                log.revision += 1;
                log.publish_status(status);
            });
            for id in &ids {
                services.disconnect(id).await;
            }
            committed.lock().unwrap().extend(ids);
            Ok(())
        }
    }

    pub fn apply_removals(&self) {
        let ids = std::mem::take(&mut *self.committed_removals.lock().unwrap());
        if ids.is_empty() {
            return;
        }
        self.connections
            .borrow_mut()
            .profiles
            .retain(|p| !ids.contains(&p.id));
        for tab in &mut self.documents.borrow_mut().tabs {
            if tab.project.as_ref().is_some_and(|id| ids.contains(id)) {
                tab.set_project(None);
            }
        }
    }

    pub fn profile_directory(&self) -> &Path {
        self.profile.directory()
    }

    pub fn retire_profile(
        &self,
    ) -> Result<tokio::sync::oneshot::Receiver<ProfileLease>, rsql_core::migration::SnapshotError>
    {
        self.profile.after_last_owner()
    }

    pub fn save_status(&self) -> watch::Receiver<Option<String>> {
        self.save_status.subscribe()
    }

    pub fn save(&self) {
        if self.stopped.get() {
            return;
        }
        let file = self.snapshot.clone();
        let profile = self.profile.clone();
        let snapshot = persistence::encode(&self.documents.borrow());
        let previous = self.writer.take();
        let status = self.save_status.clone();
        self.writer.replace(Some(self.runtime.spawn(async move {
            if let Some(previous) = previous {
                let _ = previous.await;
            }
            let result = tokio::task::spawn_blocking(move || {
                let _profile = profile;
                let bytes = snapshot?;
                file.save(bytes).map_err(|e| {
                    std::io::Error::new(e.kind(), format!("Save {}: {e}", persistence::FILENAME))
                })
            })
            .await;
            let error = match result {
                Ok(result) => result.err().map(|e| e.to_string()),
                Err(e) => Some(format!("Workspace writer failed: {e}")),
            };
            if let Some(message) = &error {
                eprintln!("{}: {message}", crate::profile::NAME);
            }
            status.send_if_modified(|current| {
                if *current == error {
                    return false;
                }
                *current = error;
                true
            });
        })));
    }

    pub fn shutdown(&self) {
        if !self.stopped.get() {
            self.preflight_stop.send_replace(true);
            for diagram in self.diagrams.take().into_iter().filter_map(|d| d.upgrade()) {
                diagram.retire();
            }
            self.runtime.block_on(async {
                tokio::join!(self.resources.shutdown(), self.updates.shutdown());
            });
            let terminal_shutdown = self.terminals.borrow_mut().shutdown();
            let (terminal, tiles) = self
                .runtime
                .block_on(async { tokio::join!(terminal_shutdown, self.tiles.shutdown()) });
            for error in [terminal, tiles].into_iter().filter_map(Result::err) {
                eprintln!("{}: {error}", crate::profile::NAME);
            }
            for load in [self.saved_load.take(), self.named_workspace_load.take()]
                .into_iter()
                .flatten()
            {
                load.abort();
                let _ = self.runtime.block_on(load);
            }
            self.runtime.block_on(self.preflight_stop.closed());
            self.map_worker.shutdown();
            self.analysis.shutdown();
            self.formatter.borrow_mut().shutdown(&self.runtime);
            self.pin.take();
            self.documents.borrow_mut().release_work();
            self.runtime.block_on(self.runner.shutdown());
            self.runtime.block_on(async {
                if let Some(write) = self.profile_write.take() {
                    let _ = write.await;
                }
            });
            self.apply_removals();
            self.save();
            self.stopped.set(true);
            if let Some(writer) = self.writer.take()
                && let Err(error) = self.runtime.block_on(writer)
            {
                self.save_status
                    .send_replace(Some(format!("Workspace writer failed: {error}")));
            }
            if self.window_geometry_dirty.get()
                && let Some(geometry) = self.window_geometry.borrow().clone()
            {
                let profile = self.profile.clone();
                let result = self.runtime.block_on(
                    self.runtime
                        .spawn_blocking(move || geometry.save(profile.directory())),
                );
                if let Err(error) = result
                    .map_err(|e| e.to_string())
                    .and_then(|r| r.map_err(|e| e.to_string()))
                {
                    self.save_status
                        .send_replace(Some(format!("Save window geometry: {error}")));
                }
            }
            self.runtime.block_on(self.services.shutdown());
        }
    }
}
