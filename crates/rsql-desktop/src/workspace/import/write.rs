use super::*;
use crate::desktop::notifications::localized_row_count;
use rsql_core::{ApplicationServices, Error};
use tokio::sync::{mpsc, oneshot};

type Probe = oneshot::Sender<Result<(), Error>>;

async fn observe(
    services: Arc<ApplicationServices>,
    project: String,
    table: Arc<ImportTable>,
    route: ProjectTicket,
    mut requests: mpsc::Receiver<Probe>,
    cancel: oneshot::Receiver<()>,
) {
    let Some(first) = requests.recv().await else {
        return;
    };
    let mut pending = Some(first);
    let result = services
        .run_metadata(
            &project,
            async {
                let _ = cancel.await;
            },
            async |client| {
                loop {
                    let result = table.check_binding(client).await;
                    let reply = pending.take().unwrap();
                    if !route.is_current() {
                        let _ =
                            reply.send(Err(Error::QueryFailed("Import connection closed".into())));
                        break;
                    }
                    let _ = reply.send(result);
                    let Some(next) = requests.recv().await else {
                        break;
                    };
                    pending = Some(next);
                }
                Ok(())
            },
        )
        .await;
    if let (Err(error), Some(reply)) = (result, pending) {
        let _ = reply.send(Err(Error::QueryFailed(bounded_message(error))));
    }
}

pub(super) struct Batch {
    pub services: Arc<ApplicationServices>,
    pub project: String,
    pub route: ProjectTicket,
    pub table: Arc<ImportTable>,
    pub path: PathBuf,
    pub headers: Arc<Vec<String>>,
    pub selection: Arc<Vec<Option<usize>>>,
}

impl Batch {
    pub async fn run(
        self,
        mut stop: watch::Receiver<bool>,
        mut close: watch::Receiver<bool>,
    ) -> (Phase, String) {
        let Self {
            services,
            project,
            route,
            table,
            path,
            headers,
            selection,
        } = self;
        let mapping = selection
            .iter()
            .enumerate()
            .filter_map(|(source, col)| col.map(|i| (source, table.columns[i].name.clone())))
            .collect::<Vec<_>>();
        let (mut rows, reader) = CsvRows::open(path, Some(headers));
        let (checks, requests) = mpsc::channel(1);
        let (cancel, closed) = oneshot::channel();
        let observer = tokio::spawn(observe(
            services.clone(),
            project,
            table.clone(),
            route.clone(),
            requests,
            closed,
        ));
        let lost = || {
            Error::QueryFailed(
                "Connection closed before the import target could be verified".into(),
            )
        };
        let outcome = run_guarded_write(
            &services,
            &route,
            &Uuid::new_v4().to_string(),
            None,
            async |tx| {
                import_bound_csv(tx, &mut rows, &table, &mapping, || {
                    let checks = checks.clone();
                    async move {
                        let (reply, response) = oneshot::channel();
                        checks.send(reply).await.map_err(|_| lost())?;
                        response.await.map_err(|_| lost())?
                    }
                })
                .await
            },
            async {
                tokio::select! {_=stop.changed()=>(),_=close.changed()=>()}
            },
        )
        .await;
        drop(rows);
        drop(checks);
        if !matches!(outcome, WriteOutcome::Committed { .. }) {
            let _ = cancel.send(());
        }
        let _ = tokio::join!(reader, observer);
        drop(stop);
        match outcome {
            WriteOutcome::Committed {
                value,
                cleanup_error,
            } => {
                let message = match cleanup_error {
                    Some(error) => bounded_message(format_args!(
                        "Imported {value} rows. Connection cleanup: {error}"
                    )),
                    None => format!("Successfully imported {} rows", localized_row_count(value)),
                };
                (Phase::Finished { committed: true }, message)
            }
            WriteOutcome::Failed(e) => (Phase::Ready, bounded_message(e)),
            WriteOutcome::Indeterminate(e) => (
                Phase::Finished { committed: false },
                bounded_message(format_args!(
                    "Commit outcome unknown. Check the table before importing again. {e}"
                )),
            ),
        }
    }
}
