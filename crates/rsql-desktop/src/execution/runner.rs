use super::{Mode, Outcome, QueryResult, RunSpec, Status, retained::Releases};
use crate::{profile_service::Challenge, sql::changes_schema_with};
use rsql_core::{
    ApplicationServices, Error, query,
    services::{ProjectTicket, run_guarded_query},
};
use std::{
    sync::Arc,
    time::{Instant, SystemTime},
};
use tokio::{
    runtime::Handle,
    sync::{oneshot, watch},
    task::JoinHandle,
};

pub struct Runner {
    runtime: Handle,
    services: Arc<ApplicationServices>,
    releases: Arc<Releases>,
    stop: watch::Sender<bool>,
}

impl Runner {
    pub fn new(runtime: Handle, services: Arc<ApplicationServices>) -> Self {
        Self::with_budget(runtime, services, Default::default())
    }

    pub fn with_budget(
        runtime: Handle,
        services: Arc<ApplicationServices>,
        budget: crate::results::DecodedBudget,
    ) -> Self {
        let pages = crate::results::PageService::new(runtime.clone(), budget);
        let releases = Arc::new(Releases::new(
            &runtime,
            services.virtual_cache.clone(),
            pages,
        ));
        Self {
            runtime,
            services,
            releases,
            stop: watch::channel(false).0,
        }
    }

    pub fn start(
        &self,
        mut spec: RunSpec,
        cancelled: oneshot::Receiver<()>,
    ) -> JoinHandle<Outcome> {
        let drain = self.stop.subscribe();
        let cancellation = Cancellation {
            owner: cancelled,
            shutdown: self.stop.subscribe(),
        };
        let ticket = self
            .services
            .capture_project_ticket(&spec.profile.id)
            .ok()
            .filter(|_| spec.guard.as_ref().is_none_or(ProjectTicket::is_current))
            .map(Arc::new);
        let services = self.services.clone();
        let releases = self.releases.clone();
        let started = SystemTime::now();
        let clock = Instant::now();
        // The worker retains the backend future even if the UI aborts its delivery handle.
        let worker = self.runtime.spawn(async move {
            let _drain = drain;
            let mut submitted_ddl = false;
            spec.generated = spec.generated.take().filter(|query| {
                spec.row_tools
                    && spec.mode == Mode::Query
                    && ticket
                        .as_ref()
                        .is_some_and(|route| query.relation_for(&spec.sql, route).is_some())
            });
            let connected = spec.connected.take();
            let status = match ticket.as_ref() {
                Some(ticket) => {
                    execute(
                        &services,
                        &releases,
                        &spec,
                        ticket,
                        cancellation,
                        connected,
                        &mut submitted_ddl,
                    )
                    .await
                }
                None => Status::Cancelled,
            };
            Outcome {
                generated: spec.generated,
                elapsed: clock.elapsed(),
                started,
                mode: spec.mode,
                may_change_catalog: submitted_ddl,
                sql: spec.sql,
                project_id: spec.profile.id,
                route_ticket: ticket
                    .map(|ticket| Arc::into_inner(ticket).expect("plan worker has finished")),
                database: spec.profile.database,
                status,
            }
        });
        self.runtime
            .spawn(async move { worker.await.expect("query worker failed") })
    }

    pub async fn shutdown(&self) {
        self.stop.send_replace(true);
        self.stop.closed().await;
        self.releases.pages.shutdown().await;
        self.releases.shutdown().await;
    }
}

struct Cancellation {
    owner: oneshot::Receiver<()>,
    shutdown: watch::Receiver<bool>,
}

impl Cancellation {
    async fn wait(&mut self) {
        tokio::select! {
            _ = &mut self.owner => (),
            _ = self.shutdown.wait_for(|stopped| *stopped) => (),
        }
    }

    fn stopped(&mut self) -> bool {
        self.owner.try_recv() != Err(oneshot::error::TryRecvError::Empty)
            || *self.shutdown.borrow()
            || self.shutdown.has_changed().is_err()
    }
}

async fn decode_plan(
    raw: String,
    ticket: Arc<ProjectTicket>,
    mut cancel: Cancellation,
) -> Result<Status, Error> {
    tokio::task::spawn_blocking(move || {
        let mut stopped = || cancel.stopped() || !ticket.is_current();
        let plan = crate::explain::PlanOutput::decode(raw, &mut stopped);
        if stopped() {
            return Ok(Status::Cancelled);
        }
        plan.map(Status::Plan)
            .map_err(|e| Error::QueryFailed(format!("Invalid EXPLAIN result: {e}")))
    })
    .await
    .map_err(|e| Error::QueryFailed(format!("EXPLAIN decoding failed: {e}")))?
}

async fn decode_rows(
    spec: &RunSpec,
    collected: (
        query::PackedResult,
        Option<rsql_core::postgres::provenance::ResultOrigin>,
    ),
    lease: super::RetainedResult,
    guard: Arc<ProjectTicket>,
    mut cancel: Cancellation,
) -> Result<Status, Error> {
    let sql = spec.sql.clone();
    let generated = spec.generated.clone();
    let size = spec.page_size;
    let (data, origin) = collected;
    let rowset = !data.0.is_empty();
    let budget = lease.service().budget().clone();
    let decoded = tokio::task::spawn_blocking(move || {
        let mut current = || !cancel.stopped() && guard.is_current();
        let (result, inline) = QueryResult::decode_packed(sql, size, data, &budget, &mut current)?;
        if let Some(origin) = origin {
            result.retain_origin((*guard).clone(), origin, generated, &mut current);
        }
        current().then_some((result, inline))
    })
    .await;
    if let Ok(Some((mut result, inline))) = decoded {
        if rowset && !lease.available().await {
            result.cache.publish(
                0,
                Err(crate::results::PageError::Unavailable(
                    "This result has expired.".into(),
                )),
            );
        }
        if rowset && (result.total_rows > size || result.cache.error(0).is_some()) {
            result.attach_source(crate::results::PackedSource::snapshot(lease));
        } else {
            if let Some(body) = inline {
                result.attach_source(crate::results::PackedSource::command(
                    body,
                    lease.service().clone(),
                ));
            }
            lease.close().await;
        }
        return Ok(Status::Rows(Arc::new(result)));
    }
    lease.close().await;
    decoded
        .map(|_| Status::Cancelled)
        .map_err(|e| Error::QueryFailed(format!("Result decoding failed: {e}")))
}

async fn execute(
    services: &ApplicationServices,
    releases: &Releases,
    spec: &RunSpec,
    ticket: &Arc<ProjectTicket>,
    mut cancel: Cancellation,
    connected_event: Option<oneshot::Sender<ProjectTicket>>,
    submitted_ddl: &mut bool,
) -> Status {
    let connected = tokio::select! { biased;
        _ = cancel.wait() => return Status::Cancelled,
        result = spec.profile.connect_for_query(services, ticket) => result,
    };
    if !ticket.is_current() {
        return Status::Cancelled;
    }
    if let Err(err) = connected {
        return failure(spec, err);
    }
    if let Some(event) = connected_event {
        let _ = event.send((**ticket).clone());
    }
    let (ddl, relation) = if spec.mode == Mode::Explain {
        (false, None)
    } else {
        let sql = spec.sql.clone();
        let guard = ticket.clone();
        let generated = spec.generated.clone();
        let row_tools = spec.row_tools && spec.mode == Mode::Query;
        let prepared = tokio::task::spawn_blocking(move || {
            let scan = changes_schema_with(&sql, || {
                if cancel.stopped() || !guard.is_current() {
                    Err(())
                } else {
                    Ok(())
                }
            });
            let relation = if row_tools && matches!(scan, Ok(false)) {
                crate::results::provenance::candidate_relation(&sql, generated.as_deref(), &guard)
            } else {
                None
            };
            (cancel, scan, relation)
        })
        .await;
        match prepared {
            Ok((owner, Ok(ddl), relation)) => {
                cancel = owner;
                (ddl, relation)
            }
            Ok((_, Err(()), _)) => return Status::Cancelled,
            Err(e) => return Status::Error(format!("SQL preparation failed: {e}")),
        }
    };
    let result = match spec.mode {
        Mode::Query => {
            let lease = releases.open(spec.page_size);
            let collected = run_guarded_query(
                services,
                ticket,
                spec.execution_id(),
                spec.timeout_ms,
                async |client| {
                    *submitted_ddl = ddl;
                    query::execute_described(
                        client,
                        &services.virtual_cache,
                        &spec.sql,
                        lease.id(),
                        spec.page_size,
                        relation.as_deref(),
                    )
                    .await
                },
                cancel.wait(),
            )
            .await;
            match collected {
                Ok(data) => decode_rows(spec, data, lease, ticket.clone(), cancel).await,
                Err(e) => {
                    lease.close().await;
                    Err(e)
                }
            }
        }
        mode => {
            let raw = run_guarded_query(
                services,
                ticket,
                spec.execution_id(),
                spec.timeout_ms,
                async |client| {
                    *submitted_ddl = ddl;
                    query::explain_plan(client, &spec.sql, mode == Mode::Analyze).await
                },
                cancel.wait(),
            )
            .await;
            match raw {
                Ok(raw) => decode_plan(raw, ticket.clone(), cancel).await,
                Err(e) => Err(e),
            }
        }
    };
    if !ticket.is_current() {
        return Status::Cancelled;
    }
    result.unwrap_or_else(|e| failure(spec, e))
}

fn failure(spec: &RunSpec, error: Error) -> Status {
    let message = error.to_string();
    if let Error::ConnectionFailed(detail) = &error
        && let Some(challenge) = Challenge::parse(detail)
    {
        return Status::TrustRequired(challenge);
    }
    let safe = spec.profile.safe_error(message);
    match error {
        Error::QueryTimeout | Error::StatementTimeout(_) => Status::TimedOut(safe),
        Error::QueryFailed(s) if s == "Query cancelled" => Status::Cancelled,
        Error::StatementCancelled(_) => Status::ServerCancelled(safe),
        _ => Status::Error(safe),
    }
}
