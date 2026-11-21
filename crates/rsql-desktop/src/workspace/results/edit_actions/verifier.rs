use rsql_core::{ApplicationServices, Error, postgres::mutation::EditBinding};
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};

type Request = (Arc<EditBinding>, oneshot::Sender<Result<bool, Error>>);

pub(super) fn observe(
    services: Arc<ApplicationServices>,
    project: String,
) -> (
    (mpsc::Sender<Request>, oneshot::Sender<()>),
    impl Future<Output = ()>,
) {
    let (send, mut requests) = mpsc::channel::<Request>(1);
    let (close, mut closed) = oneshot::channel::<()>();
    let observer = async move {
        while let Some((binding, reply)) = requests.recv().await {
            let result = services
                .run_metadata(
                    &project,
                    async {
                        let _ = (&mut closed).await;
                    },
                    async |client| binding.is_current(client).await,
                )
                .await;
            if reply.send(result).is_err() {
                break;
            }
        }
    };
    ((send, close), observer)
}

pub(super) async fn confirm(
    checks: mpsc::Sender<Request>,
    binding: Arc<EditBinding>,
) -> Result<bool, Error> {
    let closed = || Error::QueryFailed("Edit verifier closed".into());
    let (reply, response) = oneshot::channel();
    checks.send((binding, reply)).await.map_err(|_| closed())?;
    response.await.map_err(|_| closed())?
}
