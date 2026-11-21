//! One retained owner per terminal tab. All PTY and emulator work runs on Tokio.

mod actor;
mod scheduling;
mod view_queue;
mod writes;
use view_queue::ViewQueue;
pub(crate) use view_queue::{Edit, EditAdmission};
pub use writes::{MAX_INPUT_BYTES, MAX_INPUT_COMMANDS};

use super::{Scroll, SelectionType, Side, TerminalSnapshot, TerminalTheme};
use futures_util::{
    FutureExt,
    future::{BoxFuture, Shared},
};
use rsql_core::terminal::{OutputAfterFailure, OwnedTerminal, Terminals};
use std::sync::Arc;
use tokio::{
    runtime::Handle,
    sync::{Semaphore, mpsc, oneshot, watch},
};
use uuid::Uuid;

type Result<T> = std::result::Result<T, String>;
type Completion = Shared<BoxFuture<'static, Result<()>>>;

struct Spawned {
    after_failure: BoxFuture<'static, Result<OutputAfterFailure>>,
    output: mpsc::Receiver<Vec<u8>>,
    completion: Completion,
}

impl From<OwnedTerminal> for Spawned {
    fn from(opened: OwnedTerminal) -> Self {
        Self {
            after_failure: opened.completion.clone().settle_control_failure().boxed(),
            output: opened.output,
            completion: opened.completion.wait().boxed().shared(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Viewport {
    columns: u16,
    rows: u16,
}

impl Viewport {
    pub fn new(columns: u16, rows: u16) -> Result<Self> {
        if !(2..=1024).contains(&columns) || !(1..=256).contains(&rows) {
            return Err("Terminal dimensions must be 2–1024 columns and 1–256 rows".into());
        }
        Ok(Self { columns, rows })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionOwner {
    pub tab: String,
    pub generation: Uuid,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionPhase {
    Starting,
    Running,
    Exited,
    Closed,
    Failed(String),
}

impl SessionPhase {
    /// Display after the final viewport, even if EOF interrupted an escape sequence.
    pub fn notice(&self) -> Option<&str> {
        match self {
            Self::Exited => Some("[Process exited]"),
            Self::Failed(error) => Some(error),
            _ => None,
        }
    }
}

pub struct SessionUpdate {
    pub owner: SessionOwner,
    pub phase: SessionPhase,
    pub screen: Option<Arc<TerminalSnapshot>>,
    pub(crate) displayed: Option<Arc<super::DisplayedRows>>,
    /// Ordered replies retained by the emulator at a fatal protocol boundary.
    pub fault_replies: Vec<Vec<u8>>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum CommandError {
    Busy,
    TooLarge,
    Closed,
}

pub const MAX_COPY_BYTES: usize = 4 * 1024 * 1024;
type CopyResult = std::result::Result<Option<String>, CommandError>;

pub struct SelectionRequest(oneshot::Sender<CopyResult>);

pub enum ViewCommand {
    CopySelection(SelectionRequest),
    Scroll(Scroll),
    ApplyEdit { reveal: bool, clear: bool },
    SelectDisplayed(Option<SelectionType>, super::DisplayedPoint, Side),
    SelectAll,
    ScrollSelection(i32, usize, usize, Side),
    ScrollDisplayed(i32, usize, super::DisplayedPoint, Side),
    StartSelection(SelectionType, usize, usize, Side),
    UpdateSelection(usize, usize, Side),
    ClearSelection,
    Theme(TerminalTheme),
    CellSize(u16, u16),
}

#[derive(Clone)]
struct Work {
    owner: SessionOwner,
    stop: watch::Sender<bool>,
    done: Completion,
}

impl Work {
    fn stop(&self) {
        self.stop.send_replace(true);
    }
}

/// Each subscription observes the latest frame and returns owned data. Readers
/// can keep a frame across an await without holding the publisher's watch lock.
pub struct SessionUpdates(watch::Receiver<Arc<SessionUpdate>>);

impl SessionUpdates {
    pub fn current(&mut self) -> Arc<SessionUpdate> {
        self.0.borrow_and_update().clone()
    }

    pub async fn changed(
        &mut self,
    ) -> std::result::Result<Arc<SessionUpdate>, watch::error::RecvError> {
        self.0.changed().await?;
        Ok(self.current())
    }
}

pub struct TerminalSession {
    work: Work,
    writes: Arc<writes::Writes>,
    resize: watch::Sender<Viewport>,
    view: ViewQueue,
    copy_slot: Arc<Semaphore>,
    updates: watch::Receiver<Arc<SessionUpdate>>,
}

impl TerminalSession {
    pub fn owner(&self) -> &SessionOwner {
        &self.work.owner
    }

    /// Check at the point a publication is applied to a retained tab.
    pub fn accepts(&self, update: &SessionUpdate) -> bool {
        self.owner() == &update.owner && !*self.work.stop.borrow()
    }

    pub fn latest(&self) -> Arc<SessionUpdate> {
        self.updates.borrow().clone()
    }
    pub fn updates(&self) -> SessionUpdates {
        SessionUpdates(self.updates.clone())
    }

    /// Request cancellation immediately. Any waiter can join the same worker.
    pub fn close(&self) -> impl Future<Output = Result<()>> + Send + use<> {
        self.work.stop();
        self.work.done.clone()
    }

    /// Accept the whole slice or return it to the caller's control. On Busy,
    /// retry the same bytes. Split an oversized paste before submitting it.
    pub fn input(&self, bytes: &[u8]) -> std::result::Result<(), CommandError> {
        let stopped = self.work.stop.borrow();
        if *stopped {
            return Err(CommandError::Closed);
        }
        self.writes.input(bytes)
    }

    pub(crate) fn input_mode(&self) -> super::TermMode {
        self.view.state.mode()
    }

    /// Returns the active mouse generation on enqueue, or None when the report is obsolete.
    pub(crate) fn report_input(
        &self,
        bytes: &[u8],
        enabled: impl FnOnce(super::TermMode, u64) -> bool,
    ) -> std::result::Result<Option<u64>, CommandError> {
        let stopped = self.work.stop.borrow();
        if *stopped {
            return Err(CommandError::Closed);
        }
        self.view.state.report_input(bytes, enabled, &self.writes)
    }

    pub(crate) fn edit(&self, edit: Edit<'_>) -> std::result::Result<EditAdmission, CommandError> {
        let stopped = self.work.stop.borrow();
        if *stopped {
            return Err(CommandError::Closed);
        }
        self.view.edit(edit, &self.writes)
    }

    /// Request text after previously admitted selection gestures on the same actor.
    pub fn selected_text(
        &self,
    ) -> std::result::Result<impl Future<Output = CopyResult> + Send + use<>, CommandError> {
        if *self.work.stop.borrow() {
            return Err(CommandError::Closed);
        }
        let permit = self
            .copy_slot
            .clone()
            .try_acquire_owned()
            .map_err(|_| CommandError::Busy)?;
        let (reply, response) = oneshot::channel();
        self.view(ViewCommand::CopySelection(SelectionRequest(reply)))?;
        let stopped = self.work.stop.subscribe();
        Ok(async move {
            let _permit = permit;
            let text = response.await.unwrap_or(Err(CommandError::Closed));
            if *stopped.borrow() {
                return Err(CommandError::Closed);
            }
            text
        })
    }

    pub fn resize(&self, size: Viewport) -> std::result::Result<(), CommandError> {
        let stopped = self.work.stop.borrow();
        if *stopped {
            return Err(CommandError::Closed);
        }
        self.resize.send(size).map_err(|_| CommandError::Closed)
    }

    pub fn view(&self, command: ViewCommand) -> std::result::Result<(), CommandError> {
        let stopped = self.work.stop.borrow();
        if *stopped {
            return Err(CommandError::Closed);
        }
        self.view.send(command)
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        self.work.stop();
    }
}

type SpawnPty = dyn Fn(Arc<Terminals>, String, u16, u16) -> BoxFuture<'static, Result<OwnedTerminal>>
    + Send
    + Sync;

pub struct TerminalSessions {
    spawn_pty: Arc<SpawnPty>,
    runtime: Handle,
    core: Arc<Terminals>,
    work: Vec<Work>,
    shutdown: Option<Completion>,
}

fn joined(task: tokio::task::JoinHandle<Result<()>>) -> Completion {
    async move { task.await.map_err(|e| format!("Terminal worker: {e}"))? }
        .boxed()
        .shared()
}

impl TerminalSessions {
    pub fn new(runtime: Handle) -> Self {
        Self::with_spawner(runtime, |core, id, cols, rows| async move {
            core.spawn_owned(&id, cols, rows).await
        })
    }

    /// Choose the child command while retaining the same PTY owner and shutdown contract.
    pub fn with_spawner<F, Fut>(runtime: Handle, spawn: F) -> Self
    where
        F: Fn(Arc<Terminals>, String, u16, u16) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<OwnedTerminal>> + Send + 'static,
    {
        Self {
            spawn_pty: Arc::new(move |core, id, cols, rows| spawn(core, id, cols, rows).boxed()),
            runtime,
            core: Default::default(),
            work: Vec::new(),
            shutdown: None,
        }
    }

    pub fn start(&mut self, tab: &str, size: Viewport) -> Result<TerminalSession> {
        self.start_with_theme(tab, size, TerminalTheme::Light)
    }

    pub fn start_with_theme(
        &mut self,
        tab: &str,
        size: Viewport,
        theme: TerminalTheme,
    ) -> Result<TerminalSession> {
        let spawn = self.spawn_pty.clone();
        self.start_using(tab, size, theme, move |core, id, size| {
            spawn(core, id, size.columns, size.rows)
        })
    }

    fn start_using<F, Fut, O>(
        &mut self,
        tab: &str,
        size: Viewport,
        theme: TerminalTheme,
        spawn: F,
    ) -> Result<TerminalSession>
    where
        F: FnOnce(Arc<Terminals>, String, Viewport) -> Fut + Send + 'static,
        Fut: Future<Output = Result<O>> + Send + 'static,
        O: Into<Spawned> + Send,
    {
        if self.shutdown.is_some() {
            return Err("Terminals closed".into());
        }
        self.work.retain(|work| {
            if let Some(result) = work.done.clone().now_or_never() {
                if let Err(error) = result {
                    eprintln!("{error}");
                }
                return false;
            }
            if work.owner.tab == tab {
                work.stop();
            }
            true
        });
        let owner = SessionOwner {
            tab: tab.into(),
            generation: Uuid::new_v4(),
        };
        let id = owner.generation.to_string();
        let (stop, stopping) = watch::channel(false);
        let (resize, dimensions) = watch::channel(size);
        let (view, commands) = mpsc::channel(32);
        let (publish, updates) = watch::channel(Arc::new(SessionUpdate {
            owner: owner.clone(),
            phase: SessionPhase::Starting,
            screen: None,
            displayed: None,
            fault_replies: Vec::new(),
        }));
        let writes = Arc::new(writes::Writes::default());
        let view = ViewQueue::new(view);
        let actor = actor::Actor {
            theme,
            admission: view.state.clone(),
            core: self.core.clone(),
            id: id.clone(),
            stopping,
            dimensions,
            commands,
            publish,
            writes: writes.clone(),
            owner: owner.clone(),
        };
        let core = self.core.clone();
        let done = joined(self.runtime.spawn(async move {
            // Once submitted to core, spawn must run to completion, even on close.
            if *actor.stopping.borrow() {
                actor.finish(SessionPhase::Closed, None, Vec::new());
                return Ok(());
            }
            let opened = spawn(core, id, size).await;
            actor.run(opened.map(Into::into), size).await
        }));
        let work = Work { owner, stop, done };
        self.work.push(work.clone());
        Ok(TerminalSession {
            work,
            writes,
            resize,
            view,
            copy_slot: Arc::new(Semaphore::new(1)),
            updates,
        })
    }

    pub fn shutdown(&mut self) -> impl Future<Output = Result<()>> + Send + use<> {
        self.shutdown
            .get_or_insert_with(|| {
                let work = std::mem::take(&mut self.work);
                for owner in &work {
                    owner.stop();
                }
                let core = self.core.clone();
                joined(self.runtime.spawn(async move {
                    let mut result = Ok(());
                    for owner in work {
                        merge(&mut result, owner.done.await);
                    }
                    merge(&mut result, core.shutdown().await);
                    result
                }))
            })
            .clone()
    }
}

impl Drop for TerminalSessions {
    fn drop(&mut self) {
        for work in &self.work {
            work.stop();
        }
    }
}

fn merge(result: &mut Result<()>, next: Result<()>) {
    if let Err(error) = next {
        match result {
            Ok(()) => *result = Err(error),
            Err(previous) => {
                previous.push_str("; ");
                previous.push_str(&error);
            }
        }
    }
}
