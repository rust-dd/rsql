mod top_level;
use super::command_notices::NoticeSlot;
use super::{
    ProjectTicket, WriteOutcome, schema_version::SchemaVersion, write_session::run_write_session,
};
use crate::{
    ApplicationServices, Error,
    error::query_failed,
    mutation::quote_ident,
    postgres::{metadata_schema::load_relation, metadata_views_functions::resolve_routine},
    structure::{ObjectKind, ObjectRef},
};
use tokio_postgres::{Client, Transaction};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectAction {
    Analyze,
    Vacuum,
    VacuumFull,
    Reindex,
    Truncate,
    Drop,
    DropCascade,
    Refresh,
    RefreshConcurrently,
}
impl ObjectAction {
    pub fn available(kind: ObjectKind) -> &'static [Self] {
        use ObjectAction::*;
        match kind {
            ObjectKind::View => &[Drop, DropCascade],
            ObjectKind::MaterializedView => &[Refresh, RefreshConcurrently, Drop],
            ObjectKind::Function | ObjectKind::Window | ObjectKind::Procedure => {
                &[Drop, DropCascade]
            }
            _ => &[Analyze, Vacuum, VacuumFull, Reindex, Truncate, Drop],
        }
    }
    pub fn requires_top_level(self, kind: ObjectKind) -> bool {
        matches!(self, Self::Vacuum | Self::VacuumFull)
            || self == Self::Reindex && kind == ObjectKind::PartitionedTable
    }
    pub fn drops(self) -> bool {
        matches!(self, Self::Drop | Self::DropCascade)
    }
    pub fn label(self, kind: ObjectKind) -> &'static str {
        match self {
            Self::Analyze => "ANALYZE",
            Self::Vacuum => "VACUUM",
            Self::VacuumFull => "VACUUM FULL",
            Self::Reindex => "REINDEX",
            Self::Truncate => "TRUNCATE",
            Self::Refresh => "REFRESH",
            Self::RefreshConcurrently => "REFRESH CONCURRENTLY",
            Self::Drop => match kind {
                ObjectKind::View => "DROP VIEW",
                ObjectKind::MaterializedView => "DROP MATERIALIZED VIEW",
                ObjectKind::Procedure => "DROP PROCEDURE",
                ObjectKind::Window | ObjectKind::Function => "DROP FUNCTION",
                ObjectKind::ForeignTable => "DROP FOREIGN TABLE",
                _ => "DROP TABLE",
            },
            Self::DropCascade => match kind {
                ObjectKind::View => "DROP VIEW CASCADE",
                ObjectKind::Procedure => "DROP PROCEDURE CASCADE",
                _ => "DROP FUNCTION CASCADE",
            },
        }
    }
}

#[derive(Debug)]
pub struct ActionOutcome {
    pub result: WriteOutcome<()>,
    pub server_warnings: Option<String>,
    pub verification_warning: Option<String>,
    pub possible_effects: bool,
}

impl ActionOutcome {
    pub fn should_refresh(&self) -> bool {
        self.possible_effects || !matches!(self.result, WriteOutcome::Failed(_))
    }
}
impl From<WriteOutcome<Option<String>>> for ActionOutcome {
    fn from(result: WriteOutcome<Option<String>>) -> Self {
        let (result, server_warnings) = match result {
            WriteOutcome::Committed {
                value,
                cleanup_error,
            } => (
                WriteOutcome::Committed {
                    value: (),
                    cleanup_error,
                },
                value,
            ),
            WriteOutcome::Failed(error) => (WriteOutcome::Failed(error), None),
            WriteOutcome::Indeterminate(error) => (WriteOutcome::Indeterminate(error), None),
        };
        Self {
            result,
            server_warnings,
            verification_warning: None,
            possible_effects: false,
        }
    }
}

/// A single-use, route-bound confirmation for one catalog identity.
pub struct ActionReview {
    route: ProjectTicket,
    target: ActionTarget,
}
impl ActionReview {
    pub fn sql(&self) -> &str {
        &self.target.sql
    }
}
struct ActionTarget {
    object: ObjectRef,
    action: ObjectAction,
    namespace: SchemaVersion,
    sql: String,
}
fn changed() -> Error {
    Error::QueryFailed("The selected object or schema changed. Reopen its properties.".into())
}
async fn namespace(client: &Client, name: &str) -> Result<SchemaVersion, Error> {
    SchemaVersion::load(client, name).await?.ok_or_else(changed)
}
async fn target(client: &Client, o: &ObjectRef) -> Result<String, Error> {
    if let Some(r) = &o.routine_identity {
        let actual = resolve_routine(client, r).await?;
        if ObjectRef::routine(&o.project, actual.identity) != *o {
            return Err(changed());
        }
        Ok(actual.action_identity)
    } else {
        if ObjectRef::relation(&o.project, load_relation(client, &o.schema, &o.name).await?) != *o {
            return Err(changed());
        }
        Ok(format!(
            "{}.{}",
            quote_ident(&o.schema),
            quote_ident(&o.name)
        ))
    }
}
impl ActionTarget {
    async fn capture(
        client: &Client,
        object: &ObjectRef,
        action: ObjectAction,
    ) -> Result<Self, Error> {
        if !ObjectAction::available(object.kind).contains(&action) {
            return Err(Error::QueryFailed(
                "Action unavailable for this object kind".into(),
            ));
        }
        let before = namespace(client, &object.schema).await?;
        let name = target(client, object).await?;
        let verb = match action {
            ObjectAction::DropCascade => ObjectAction::Drop.label(object.kind),
            ObjectAction::Reindex => "REINDEX TABLE",
            ObjectAction::Refresh => "REFRESH MATERIALIZED VIEW",
            ObjectAction::RefreshConcurrently => "REFRESH MATERIALIZED VIEW CONCURRENTLY",
            _ => action.label(object.kind),
        };
        let sql = format!(
            "{verb} {name}{}",
            if action == ObjectAction::DropCascade {
                " CASCADE"
            } else {
                ""
            }
        );
        if before != namespace(client, &object.schema).await? {
            return Err(changed());
        }
        Ok(Self {
            object: object.clone(),
            action,
            namespace: before,
            sql,
        })
    }
    async fn verify(&self, client: &Client) -> Result<(), Error> {
        let fresh = Self::capture(client, &self.object, self.action).await?;
        if self.namespace != fresh.namespace || self.sql != fresh.sql {
            return Err(changed());
        }
        Ok(())
    }
    async fn check_maintenance(&self, client: &Client) -> Result<(), Error> {
        if matches!(self.action, ObjectAction::Vacuum | ObjectAction::VacuumFull)
            && self.object.kind == ObjectKind::ForeignTable
        {
            return Err(Error::QueryFailed(
                "PostgreSQL cannot vacuum a foreign table.".into(),
            ));
        }
        let row = client.query_one(
            "SELECT CASE WHEN pg_catalog.current_setting('server_version_num')::integer >= 170000
            THEN pg_catalog.has_table_privilege(c.oid,'MAINTAIN') ELSE pg_catalog.pg_has_role(c.relowner,'USAGE') END
            OR (NOT c.relisshared AND pg_catalog.pg_has_role(d.datdba,'USAGE')),
            c.relpersistence = 't' AND c.relnamespace <> pg_catalog.pg_my_temp_schema()
            FROM pg_catalog.pg_class c, pg_catalog.pg_database d WHERE c.oid=$1 AND d.datname=pg_catalog.current_database()",
            &[&self.object.oid],
        ).await.map_err(query_failed)?;
        if row.get::<_, bool>(1) {
            return Err(Error::QueryFailed(
                "The temporary table belongs to another database session.".into(),
            ));
        }
        if !row.get::<_, bool>(0) {
            return Err(Error::QueryFailed(
                "Insufficient permission to maintain this table.".into(),
            ));
        }
        Ok(())
    }
    async fn transaction(
        &self,
        tx: &Transaction<'_>,
        notices: &NoticeSlot,
    ) -> Result<Option<String>, Error> {
        let o = &self.object;
        tx.batch_execute(
            "SET TRANSACTION ISOLATION LEVEL READ COMMITTED; SET LOCAL search_path = pg_catalog",
        )
        .await
        .map_err(query_failed)?;
        self.verify(tx.client()).await?;
        if let Some(r) = &o.routine_identity {
            let addr = tx.query_one(
                "SELECT a.objid FROM pg_catalog.pg_identify_object_as_address('pg_catalog.pg_proc'::regclass,$1,0) i,
                LATERAL pg_catalog.pg_get_object_address(i.type,i.object_names,i.object_args) a",
                &[&r.oid],
            ).await.map_err(query_failed)?;
            if addr.get::<_, u32>(0) != r.oid {
                return Err(changed());
            }
        } else {
            let relation = format!("{}.{}", quote_ident(&o.schema), quote_ident(&o.name));
            // A prepared SELECT acquires its relation lock without running a view or scanning rows.
            tx.prepare(&format!("SELECT FROM ONLY {relation} LIMIT 0"))
                .await
                .map_err(query_failed)?;
            let locked = tx.query_one(
                "SELECT EXISTS(SELECT FROM pg_catalog.pg_locks WHERE pid=pg_catalog.pg_backend_pid()
                 AND locktype='relation' AND relation=$1 AND granted)",
                &[&o.oid],
            ).await.map_err(query_failed)?.get::<_,bool>(0);
            if !locked {
                return Err(changed());
            }
        }
        self.verify(tx.client()).await?;
        let watch = if self.action == ObjectAction::Analyze {
            self.check_maintenance(tx.client()).await?;
            tx.batch_execute("SET LOCAL client_min_messages = warning")
                .await
                .map_err(query_failed)?;
            Some(notices.begin()?)
        } else {
            None
        };
        tx.batch_execute(&self.sql).await.map_err(query_failed)?;
        let warning = watch.and_then(|scope| scope.finish());
        if self.namespace != namespace(tx.client(), &o.schema).await? {
            return Err(changed());
        }
        if self.action.drops() {
            let query = if o.routine_identity.is_some() {
                "SELECT FROM pg_catalog.pg_proc WHERE oid=$1"
            } else {
                "SELECT FROM pg_catalog.pg_class WHERE oid=$1"
            };
            if tx
                .query_opt(query, &[&o.oid])
                .await
                .map_err(query_failed)?
                .is_some()
            {
                return Err(changed());
            }
        } else {
            self.verify(tx.client()).await?;
        }
        Ok(warning)
    }
}
impl ApplicationServices {
    pub async fn review_object_action(
        &self,
        route: &ProjectTicket,
        object: &ObjectRef,
        action: ObjectAction,
        closed: impl Future<Output = ()>,
    ) -> Result<ActionReview, Error> {
        if object.project != route.1 || !self.owns_ticket(route) {
            return Err(changed());
        }
        let target = self
            .run_metadata(&object.project, closed, async |client| {
                if !self.owns_ticket(route) {
                    return Err(changed());
                }
                ActionTarget::capture(client, object, action).await
            })
            .await?;
        if !self.owns_ticket(route) {
            return Err(changed());
        }
        Ok(ActionReview {
            route: route.clone(),
            target,
        })
    }
    pub async fn run_object_action(
        &self,
        id: &str,
        review: ActionReview,
        closed: impl Future<Output = ()>,
    ) -> ActionOutcome {
        let ActionReview { route, target } = review;
        if target.action.requires_top_level(target.object.kind) {
            top_level::run(self, &route, id, target, closed).await
        } else {
            run_write_session(
                self,
                &route,
                id,
                None,
                async |tx, notices| target.transaction(tx, notices).await,
                closed,
            )
            .await
            .into()
        }
    }
}
