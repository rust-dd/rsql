use futures_util::{
    FutureExt,
    future::{BoxFuture, Shared},
};
use tokio::{sync::oneshot, task::JoinHandle};

type SharedResult<T> = Shared<BoxFuture<'static, Result<T, String>>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputAfterFailure {
    /// Child teardown is underway. Receive the forwarder's remaining bytes.
    Drain,
    /// The supervisor observed a running child and initiated termination.
    CloseReceiver,
}

pub(super) type FailureReply = oneshot::Sender<Result<OutputAfterFailure, String>>;

#[derive(Clone)]
pub struct TerminalCompletion {
    worker: SharedResult<()>,
    failure: SharedResult<OutputAfterFailure>,
}

impl TerminalCompletion {
    pub(super) fn new(
        worker: JoinHandle<Result<(), String>>,
        request: oneshot::Sender<FailureReply>,
    ) -> Self {
        let worker = async move { worker.await.map_err(super::message)? }
            .boxed()
            .shared();
        let failure = async move {
            let (reply, decision) = oneshot::channel();
            if request.send(reply).is_err() {
                return Ok(OutputAfterFailure::Drain);
            }
            // The supervisor drops this request when another event already
            // committed it to teardown. Its exact join still carries any error.
            decision.await.unwrap_or(Ok(OutputAfterFailure::Drain))
        }
        .boxed()
        .shared();
        Self { worker, failure }
    }

    /// After a control failure, ask this exact child's supervisor to stop it
    /// if it is still running. All receipt clones share one request and decision.
    pub async fn settle_control_failure(self) -> Result<OutputAfterFailure, String> {
        self.failure.await
    }

    /// Join this exact child and reader, including after automatic deregistration.
    pub async fn wait(self) -> Result<(), String> {
        self.worker.await
    }
}
