use super::{Analysis, analyze};
use crate::document::Side;
use std::{
    collections::VecDeque,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use tokio::sync::oneshot;

const PARSE_TIMEOUT: Duration = Duration::from_secs(8);
const PENDING_DOCUMENTS: usize = 16;
type Reply = Result<Analysis, String>;
type Owner = (String, Side);

pub struct AnalysisRequest {
    pub owner: Owner,
    pub revision: u64,
    pub caret: usize,
    pub project_generation: u64,
    pub text: Arc<str>,
}

struct Work {
    cancelled: AtomicBool,
    reply: Mutex<Option<oneshot::Sender<Reply>>>,
}

impl Work {
    fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        self.reply.lock().unwrap().take();
    }
}

#[derive(Default)]
struct Queue {
    pending: VecDeque<(AnalysisRequest, Arc<Work>)>,
    active: Option<(Owner, Arc<Work>)>,
    stopped: bool,
}

#[derive(Default)]
struct Shared {
    queue: Mutex<Queue>,
    ready: Condvar,
}

pub struct AnalysisTicket {
    shared: Arc<Shared>,
    work: Arc<Work>,
    receiver: oneshot::Receiver<Reply>,
}

impl AnalysisTicket {
    pub async fn result(self) -> Reply {
        self.receive().await.ok_or("Analysis cancelled.")?
    }

    /// A withdrawn request has no parser reply.
    pub async fn receive(mut self) -> Option<Reply> {
        (&mut self.receiver).await.ok()
    }

    pub fn cancel(&self) {
        let mut queue = self.shared.queue.lock().unwrap();
        self.work.cancel();
        queue
            .pending
            .retain(|(_, work)| !Arc::ptr_eq(work, &self.work));
    }
}

impl Drop for AnalysisTicket {
    fn drop(&mut self) {
        self.cancel();
    }
}

pub struct AnalysisWorker {
    shared: Arc<Shared>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl Default for AnalysisWorker {
    fn default() -> Self {
        Self::with_timeout(PARSE_TIMEOUT)
    }
}

impl AnalysisWorker {
    pub fn with_timeout(timeout: Duration) -> Self {
        Self::start(timeout, |request, flag, deadline| {
            analyze(&request.text, request.caret, flag, deadline)
        })
    }

    fn start(
        timeout: Duration,
        parse: impl Fn(&AnalysisRequest, &AtomicBool, Instant) -> Reply + Send + 'static,
    ) -> Self {
        let shared = Arc::new(Shared::default());
        let executor = shared.clone();
        let thread = thread::spawn(move || {
            loop {
                let mut queue = executor
                    .ready
                    .wait_while(executor.queue.lock().unwrap(), |q| {
                        q.pending.is_empty() && !q.stopped
                    })
                    .unwrap();
                if queue.stopped {
                    break;
                }
                let (request, work) = queue.pending.pop_front().unwrap();
                queue.active = Some((request.owner.clone(), work.clone()));
                drop(queue);
                let reply = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    parse(&request, &work.cancelled, Instant::now() + timeout)
                }))
                .unwrap_or_else(|_| Err("SQL analysis failed.".into()));
                let mut queue = executor.queue.lock().unwrap();
                queue.active.take();
                if let Some(tx) = work.reply.lock().unwrap().take() {
                    let _ = tx.send(reply);
                }
            }
        });
        Self {
            shared,
            thread: Mutex::new(Some(thread)),
        }
    }

    pub fn submit(&self, request: AnalysisRequest) -> AnalysisTicket {
        let (sender, receiver) = oneshot::channel();
        let work = Arc::new(Work {
            cancelled: AtomicBool::new(false),
            reply: Mutex::new(Some(sender)),
        });
        let mut queue = self.shared.queue.lock().unwrap();
        if queue.stopped {
            work.cancel();
        } else {
            if let Some((owner, running)) = &queue.active
                && *owner == request.owner
            {
                running.cancel();
            }
            if let Some(i) = queue
                .pending
                .iter()
                .position(|(r, _)| r.owner == request.owner)
            {
                queue.pending.remove(i).unwrap().1.cancel();
            }
            if queue.pending.len() == PENDING_DOCUMENTS {
                queue.pending.pop_front().unwrap().1.cancel();
            }
            queue.pending.push_back((request, work.clone()));
            self.shared.ready.notify_one();
        }
        AnalysisTicket {
            shared: self.shared.clone(),
            work,
            receiver,
        }
    }

    pub fn shutdown(&self) {
        let mut queue = self.shared.queue.lock().unwrap();
        queue.stopped = true;
        if let Some((_, active)) = &queue.active {
            active.cancel();
        }
        for (_, pending) in queue.pending.drain(..) {
            pending.cancel();
        }
        self.shared.ready.notify_one();
        drop(queue);
        if let Some(thread) = self.thread.lock().unwrap().take() {
            thread
                .join()
                .expect("SQL analysis worker stopped unexpectedly");
        }
    }
}
impl Drop for AnalysisWorker {
    fn drop(&mut self) {
        self.shutdown();
    }
}
