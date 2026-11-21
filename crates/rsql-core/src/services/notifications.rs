mod send;

use super::{ApplicationServices, ProjectTicket, closed};
use crate::{Error, error::query_failed};
use futures_util::{
    FutureExt, StreamExt,
    future::{BoxFuture, Shared},
};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use tokio::sync::{mpsc, watch};
use tokio::task::AbortHandle;
use tokio_postgres::{AsyncMessage, NoTls};

#[derive(Clone, serde::Serialize)]
pub struct Notification {
    pub channel: String,
    pub payload: String,
}

type Ready = watch::Receiver<Option<Result<(), Error>>>;
type Joined = Shared<BoxFuture<'static, ()>>;
pub struct Notifications {
    joined: Joined,
    events: mpsc::Receiver<Notification>,
    status: Ready,
}

impl Notifications {
    /// Release this receiver and wait for its registered worker to finish.
    pub async fn close(self) {
        drop(self.events);
        self.joined.await;
    }

    pub async fn recv(&mut self) -> Option<Notification> {
        tokio::select! {
            biased;
            _ = self.status.changed() => None,
            event = self.events.recv() => event,
        }
    }
}

type Key = (String, String, Option<uuid::Uuid>);
struct Worker {
    task: AbortHandle,
    joined: Joined,
    ready: Ready,
    consumer: Option<mpsc::WeakSender<Notification>>,
}

#[derive(Default)]
pub(super) struct Listeners(Arc<Mutex<BTreeMap<Key, Worker>>>);

impl Listeners {
    async fn stop_matching(&self, predicate: impl Fn(&Key) -> bool) {
        let workers: Vec<_> = self
            .0
            .lock()
            .unwrap()
            .extract_if(.., |k, _| predicate(k))
            .map(|(_, w)| w)
            .collect();
        for w in &workers {
            w.task.abort();
        }
        for w in workers {
            w.joined.await;
        }
    }
    pub(super) async fn stop_project(&self, project: &str) {
        self.stop_matching(|k| k.0 == project).await;
    }
    pub(super) async fn stop_all(&self) {
        self.stop_matching(|_| true).await;
    }
}

impl Drop for Listeners {
    fn drop(&mut self) {
        for w in self.0.lock().unwrap().values() {
            w.task.abort();
        }
    }
}

impl ApplicationServices {
    fn add_listener(
        &self,
        key: Key,
        ready: Ready,
        consumer: Option<mpsc::WeakSender<Notification>>,
        task: impl Future<Output = ()> + Send + 'static,
    ) -> (bool, Ready, Joined) {
        let mut workers = self.notify_handles.0.lock().unwrap();
        if let Some(w) = workers.get(&key)
            && !w.task.is_finished()
            && !matches!(&*w.ready.borrow(), Some(Err(_)))
            && w.consumer
                .as_ref()
                .is_none_or(|weak| weak.upgrade().is_some_and(|tx| !tx.is_closed()))
        {
            return (false, w.ready.clone(), w.joined.clone());
        }
        let registry = Arc::downgrade(&self.notify_handles.0);
        let worker_key = key.clone();
        let handle = tokio::spawn(async move {
            task.await;
            if let Some(registry) = registry.upgrade() {
                let mut entries = registry.lock().unwrap();
                if entries
                    .get(&worker_key)
                    .is_some_and(|w| w.task.id() == tokio::task::id())
                {
                    entries.remove(&worker_key);
                }
            }
        });
        let task = handle.abort_handle();
        let joined = handle.map(|_| ()).boxed().shared();
        let replaced = workers.insert(
            key,
            Worker {
                task,
                joined: joined.clone(),
                ready: ready.clone(),
                consumer,
            },
        );
        if let Some(previous) = replaced {
            previous.task.abort();
        }
        (true, ready, joined)
    }

    pub async fn register_listener(
        &self,
        ticket: ProjectTicket,
        channel: String,
        task: impl Future<Output = ()> + Send + 'static,
    ) -> Result<(), Error> {
        let _guard = self.lifecycle.lock().await;
        if *self.stopped.borrow() || ticket.0.has_changed().is_err() {
            return Err(closed());
        }
        let _ = self.add_listener(
            (ticket.1, channel, None),
            watch::channel(Some(Ok(()))).1,
            None,
            task,
        );
        Ok(())
    }

    /// Register an independent native receiver on this route. Cancellation during
    /// registration joins any worker already admitted before returning an error.
    pub async fn listen_guarded(
        &self,
        ticket: &ProjectTicket,
        channel: &str,
        cancelled: impl Future<Output = ()>,
    ) -> Result<Notifications, Error> {
        self.start_listener(
            &ticket.1,
            channel,
            Some(uuid::Uuid::new_v4()),
            Some(ticket),
            cancelled,
        )
        .await?
        .ok_or_else(closed)
    }

    /// Dropping the receiver releases this subscription.
    pub async fn listen(&self, project: &str, channel: &str) -> Result<Notifications, Error> {
        self.start_listener(
            project,
            channel,
            Some(uuid::Uuid::new_v4()),
            None,
            std::future::pending(),
        )
        .await?
        .ok_or_else(closed)
    }

    /// The desktop event bridge owns one registration per project and channel.
    pub async fn listen_legacy(
        &self,
        project: &str,
        channel: &str,
    ) -> Result<Option<Notifications>, Error> {
        self.start_listener(project, channel, None, None, std::future::pending())
            .await
    }

    async fn start_listener(
        &self,
        project: &str,
        channel: &str,
        subscriber: Option<uuid::Uuid>,
        ticket: Option<&ProjectTicket>,
        cancelled: impl Future<Output = ()>,
    ) -> Result<Option<Notifications>, Error> {
        tokio::pin!(cancelled);
        let guard = tokio::select! {
            biased;
            _ = &mut cancelled => return Err(closed()),
            guard = self.lifecycle.lock() => guard,
        };
        if *self.stopped.borrow() || ticket.is_some_and(|ticket| !self.owns_ticket(ticket)) {
            return Err(closed());
        }
        let (config, ssl) = self.published_route(project).await?;
        let (events, rx) = mpsc::channel(64);
        let (ready, status) = watch::channel(None);
        let name = channel.to_owned();
        let consumer = events.downgrade();
        let exact_channel = ticket.is_some();
        let worker = async move {
            let connect = async {
                if let Some(tls) = ssl.connector()? {
                    let (client, conn) = config.connect(tls).await.map_err(query_failed)?;
                    listen_loop(client, conn, &name, exact_channel, &events, &ready).await
                } else {
                    let (client, conn) = config.connect(NoTls).await.map_err(query_failed)?;
                    listen_loop(client, conn, &name, exact_channel, &events, &ready).await
                }
            };
            tokio::select! {
                _ = events.closed() => {}
                result = connect => {
                    if let Err(error) = result {
                        ready.send_replace(Some(Err(error)));
                    }
                }
            }
        };
        let (owner, mut status, joined) = self.add_listener(
            (project.to_owned(), channel.to_owned(), subscriber),
            status,
            Some(consumer),
            worker,
        );
        drop(guard);
        let result = tokio::select! {
            biased;
            _ = &mut cancelled => Err(closed()),
            ready = status.wait_for(|r| r.is_some()) => {
                ready.map_err(|_| closed()).and_then(|r| r.as_ref().unwrap().clone())
            }
        };
        if let Err(error) = result {
            drop(rx);
            if owner {
                joined.await;
            }
            return Err(error);
        }
        Ok(owner.then_some(Notifications {
            events: rx,
            status,
            joined,
        }))
    }

    pub async fn stop_listening(&self, project: &str, channel: &str) {
        let _guard = self.lifecycle.lock().await;
        self.notify_handles
            .stop_matching(|k| k.0 == project && k.1 == channel && k.2.is_none())
            .await
    }
}

async fn listen_loop<S, T>(
    client: tokio_postgres::Client,
    mut conn: tokio_postgres::Connection<S, T>,
    channel: &str,
    exact_channel: bool,
    events: &mpsc::Sender<Notification>,
    ready: &watch::Sender<Option<Result<(), Error>>>,
) -> Result<(), Error>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let subscribe = async {
        if exact_channel {
            // LISTEN would silently truncate the identifier and subscribe to a different channel.
            let limit = client
                .query_one(
                    "SELECT current_setting('max_identifier_length')::integer",
                    &[],
                )
                .await
                .map_err(query_failed)?
                .get::<_, i32>(0);
            if channel.len() > limit as usize {
                return Err(Error::QueryFailed(format!(
                    "Channel name exceeds the server's {limit}-byte identifier limit"
                )));
            }
        }
        let sql = format!("LISTEN \"{}\"", channel.replace('"', "\"\""));
        client.batch_execute(&sql).await.map_err(query_failed)
    };
    let drive = async {
        let mut stream = futures_util::stream::poll_fn(|cx| conn.poll_message(cx));
        while let Some(msg) = stream.next().await {
            if let AsyncMessage::Notification(n) = msg.map_err(query_failed)? {
                events
                    .send(Notification {
                        channel: n.channel().into(),
                        payload: n.payload().into(),
                    })
                    .await
                    .map_err(|_| closed())?;
            }
        }
        Err(closed())
    };
    tokio::pin!(drive);
    tokio::select! {
        result = &mut drive => return result,
        result = subscribe => result?,
    }
    ready.send_replace(Some(Ok(())));
    drive.await
}
