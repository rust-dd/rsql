use super::*;
use rsql_core::{
    ApplicationServices, Error,
    error::query_failed,
    postgres::roles_schema_objects::{SchemaObject, extract_schema_objects},
};
use tokio::sync::watch;

struct Interest {
    capture: catalog::Capture,
    request: watch::Receiver<()>,
    stop: watch::Receiver<bool>,
}

fn cancelled() -> String {
    "Schema comparison cancelled".to_owned()
}

impl Interest {
    fn check(&self) -> Result<(), String> {
        if self.capture.route.is_current()
            && matches!(self.capture.generation.has_changed(), Ok(false))
            && matches!(self.request.has_changed(), Ok(false))
            && !*self.stop.borrow()
        {
            Ok(())
        } else {
            Err(cancelled())
        }
    }

    async fn retired(&self) {
        let mut request = self.request.clone();
        let mut generation = self.capture.generation.clone();
        let mut stop = self.stop.clone();
        tokio::select! {
            _ = request.changed() => (),
            _ = generation.changed() => (),
            _ = self.capture.route.retired() => (),
            _ = stop.wait_for(|s| *s) => (),
        }
    }
}

pub(super) async fn compare(
    services: Arc<ApplicationServices>,
    capture: catalog::Capture,
    schemas: [String; 2],
    request: watch::Receiver<()>,
    stop: watch::Receiver<bool>,
) -> Result<Comparison, String> {
    let interest = Interest {
        capture,
        request,
        stop,
    };
    let (left, right) = services
        .run_metadata(
            &interest.capture.project,
            interest.retired(),
            async |client| {
                if !interest.capture.route.is_current() {
                    return Err(Error::ConnectionFailed("Connection changed".into()));
                }
                client
                    .batch_execute("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
                    .await
                    .map_err(query_failed)?;
                for schema in &schemas {
                    let row = client
                        .query_opt(
                            "SELECT pg_catalog.has_schema_privilege(n.oid, 'USAGE')
                         FROM pg_catalog.pg_namespace n WHERE n.nspname = $1",
                            &[schema],
                        )
                        .await
                        .map_err(query_failed)?;
                    if !row.is_some_and(|r| r.get::<_, bool>(0)) {
                        return Err(Error::QueryFailed(format!(
                            "Schema {schema} is missing or lacks USAGE permission"
                        )));
                    }
                }
                let left = extract_schema_objects(client, &schemas[0]).await?;
                let right = extract_schema_objects(client, &schemas[1]).await?;
                client.batch_execute("COMMIT").await.map_err(query_failed)?;
                Ok((left, right))
            },
        )
        .await
        .map_err(|e| e.to_string())?;
    compute(left, right, interest).await
}

async fn compute(
    left: Vec<SchemaObject>,
    right: Vec<SchemaObject>,
    interest: Interest,
) -> Result<Comparison, String> {
    interest.check()?;
    let executor = tokio::runtime::Handle::current();

    // Keep the vectors with the cancellable future until a blocking thread can take them.
    let (admit, slot) = oneshot::channel();
    let job = executor.spawn_blocking(move || {
        let (send, work) = oneshot::channel::<(Vec<SchemaObject>, Vec<SchemaObject>, Interest)>();
        admit.send(send).map_err(|_| cancelled())?;
        let (left, right, interest) = work.blocking_recv().map_err(|_| cancelled())?;
        interest.check()?;
        Comparison::new(left, right, || interest.check())
    });

    let sender = tokio::select! {
        biased;
        _ = interest.retired() => {
            job.abort();
            return Err(cancelled());
        }
        sender = slot => sender.map_err(|_| "Schema comparison worker unavailable".to_owned())?,
    };
    sender
        .send((left, right, interest))
        .map_err(|_| cancelled())?;
    job.await.map_err(|e| e.to_string())?
}
