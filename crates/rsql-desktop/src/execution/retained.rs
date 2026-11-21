use rsql_core::{
    Error,
    query::{self, VirtualCache},
};
use std::{collections::BTreeSet, sync::Arc};
use tokio::{
    runtime::Handle,
    sync::{Mutex, mpsc, oneshot},
};
use uuid::Uuid;

type Cache = Arc<Mutex<VirtualCache>>;

enum Command {
    Open(String),
    Close(String, Option<oneshot::Sender<()>>),
    Shutdown(oneshot::Sender<()>),
}

/// The cleanup worker owns releases even when the UI drops its outcome or
/// a result outlives the window that started the query.
pub(super) struct Releases {
    pub(super) pages: crate::results::PageService,
    sender: mpsc::UnboundedSender<Command>,
    cache: Cache,
}

impl Releases {
    pub fn new(runtime: &Handle, cache: Cache, pages: crate::results::PageService) -> Self {
        let (sender, mut rx) = mpsc::unbounded_channel();
        let owned = cache.clone();
        runtime.spawn(async move {
            let mut ids = BTreeSet::new();
            let mut shutdown = None;
            while let Some(command) = rx.recv().await {
                match command {
                    Command::Open(id) => {
                        ids.insert(id);
                    }
                    Command::Close(id, done) => {
                        if ids.remove(&id) {
                            let _ = query::close_virtual(&owned, &id).await;
                        }
                        if let Some(done) = done {
                            let _ = done.send(());
                        }
                    }
                    Command::Shutdown(done) => {
                        shutdown = Some(done);
                        break;
                    }
                }
            }
            for id in ids {
                let _ = query::close_virtual(&owned, &id).await;
            }
            if let Some(done) = shutdown {
                let _ = done.send(());
            }
        });
        Self {
            sender,
            cache,
            pages,
        }
    }

    pub fn open(&self, page_size: usize) -> RetainedResult {
        let id = Uuid::new_v4().to_string();
        let _ = self.sender.send(Command::Open(id.clone()));
        RetainedResult(Arc::new(Lease {
            sender: self.sender.clone(),
            cache: self.cache.clone(),
            id,
            page_size,
            service: self.pages.clone(),
            closed: Default::default(),
        }))
    }

    pub async fn shutdown(&self) {
        let (done, wait) = oneshot::channel();
        let _ = self.sender.send(Command::Shutdown(done));
        let _ = wait.await;
    }
}

struct Lease {
    service: crate::results::PageService,
    closed: std::sync::atomic::AtomicBool,
    cache: Cache,
    sender: mpsc::UnboundedSender<Command>,
    id: String,
    page_size: usize,
}

impl Drop for Lease {
    fn drop(&mut self) {
        let _ = self.sender.send(Command::Close(self.id.clone(), None));
    }
}

/// A retained packed result is paged after collection. Offset selects the
/// containing page in the core cache, independent of any PostgreSQL session.
#[derive(Clone)]
pub struct RetainedResult(Arc<Lease>);

impl RetainedResult {
    pub(crate) fn service(&self) -> &crate::results::PageService {
        &self.0.service
    }

    pub(crate) fn is_open(&self) -> bool {
        !self.0.closed.load(std::sync::atomic::Ordering::Acquire)
    }

    pub(crate) fn readable_now(&self) -> bool {
        self.is_open()
            && self
                .0
                .cache
                .try_lock()
                .is_ok_and(|cache| cache.contains_live(self.id()))
    }

    pub(crate) async fn available(&self) -> bool {
        self.is_open() && self.0.cache.lock().await.contains_live(self.id())
    }

    pub fn id(&self) -> &str {
        &self.0.id
    }

    pub async fn fetch_packed(&self, offset: usize) -> Result<String, Error> {
        query::fetch_virtual_page(&self.0.cache, self.id(), 0, offset, self.0.page_size).await
    }

    pub(crate) async fn shared_page(&self, offset: usize) -> Result<query::SharedPage, Error> {
        if self.is_open()
            && let Some(page) = query::shared_virtual_page(&self.0.cache, self.id(), offset).await?
        {
            return Ok(page);
        }
        Err(Error::QueryFailed(
            "This result is incomplete or has expired.".into(),
        ))
    }

    /// Release immediately, including for other handles to this result.
    pub async fn close(&self) {
        self.0
            .closed
            .store(true, std::sync::atomic::Ordering::Release);
        let (done, wait) = oneshot::channel();
        let _ = self
            .0
            .sender
            .send(Command::Close(self.id().into(), Some(done)));
        let _ = wait.await;
    }
}
