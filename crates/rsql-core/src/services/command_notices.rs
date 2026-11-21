//! PostgreSQL delivers warnings on the connection driver, outside command responses.

use crate::Error;
use futures_util::future::{BoxFuture, poll_fn};
use std::sync::{Arc, Mutex};
use tokio::task::JoinHandle;
use tokio_postgres::{
    AsyncMessage, Client, Config, Socket,
    error::{DbError, Severity},
    tls::{MakeTlsConnect, TlsConnect},
};

const WARNING_LIMIT: usize = 4;
const MESSAGE_BYTES: usize = 240;
type PhysicalSlot = Mutex<Option<Warnings>>;

#[derive(Clone, Default, Debug)]
pub(super) struct NoticeSlot(Arc<PhysicalSlot>);

#[derive(Default, Debug)]
struct Warnings {
    messages: Vec<String>,
    omitted: usize,
}
impl Warnings {
    fn push(&mut self, code: &str, message: &str) {
        if self.messages.len() == WARNING_LIMIT {
            self.omitted = self.omitted.saturating_add(1);
            return;
        }
        let end = message.floor_char_boundary(message.len().min(MESSAGE_BYTES));
        self.messages.push(format!(
            "[{code}] {}{}",
            &message[..end],
            if end < message.len() { "…" } else { "" }
        ));
    }
    fn summary(self) -> Option<String> {
        if self.messages.is_empty() {
            return None;
        }
        let mut text = self.messages.join("\n");
        if self.omitted > 0 {
            text.push_str(&format!("\n{} additional warnings.", self.omitted));
        }
        Some(text)
    }
}

pub(super) struct CommandNotices(Arc<PhysicalSlot>);
impl CommandNotices {
    pub(super) fn finish(self) -> Option<String> {
        self.0
            .lock()
            .unwrap()
            .as_mut()
            .map(std::mem::take)
            .and_then(Warnings::summary)
    }
}
impl Drop for CommandNotices {
    fn drop(&mut self) {
        self.0.lock().unwrap().take();
    }
}

impl NoticeSlot {
    pub(super) fn begin(&self) -> Result<CommandNotices, Error> {
        let mut capture = self.0.lock().unwrap();
        if capture.is_some() {
            return Err(Error::QueryFailed(
                "A command already owns this connection's warnings.".into(),
            ));
        }
        *capture = Some(Warnings::default());
        Ok(CommandNotices(self.0.clone()))
    }
}
pub(super) struct ObservedConnect<T>(pub T);
impl<T> deadpool_postgres::Connect for ObservedConnect<T>
where
    T: MakeTlsConnect<Socket> + Clone + Sync + Send + 'static,
    T::Stream: Sync + Send,
    T::TlsConnect: Sync + Send,
    <T::TlsConnect as TlsConnect<Socket>>::Future: Send,
{
    fn connect(
        &self,
        config: &Config,
    ) -> BoxFuture<'_, Result<(Client, JoinHandle<()>), tokio_postgres::Error>> {
        let config = config.clone();
        let tls = self.0.clone();
        let driver_slot = super::pooled::CREATING.with(|slot| slot.0.clone());
        Box::pin(async move {
            let (client, mut connection) = config.connect(tls).await?;
            let driver = tokio::spawn(async move {
                while let Some(event) = poll_fn(|cx| connection.poll_message(cx)).await {
                    match event {
                        Ok(AsyncMessage::Notice(notice)) => record(&driver_slot, notice),
                        Ok(_) => (),
                        Err(error) => {
                            tracing::warn!("PostgreSQL connection error: {error}");
                            break;
                        }
                    }
                }
            });
            Ok((client, driver))
        })
    }
}
fn record(slot: &PhysicalSlot, notice: DbError) {
    if notice.parsed_severity() == Some(Severity::Warning)
        && let Some(warnings) = slot.lock().unwrap().as_mut()
    {
        warnings.push(notice.code().code(), notice.message());
        return;
    }
    tracing::info!(target:"tokio_postgres::connection", "{}: {}", notice.severity(), notice.message());
}
