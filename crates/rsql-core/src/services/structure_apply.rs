mod dependencies;
mod foreign_keys;

use super::{
    ApplicationServices, ProjectTicket, WriteOutcome, run_guarded_write,
    schema_version::SchemaVersion,
};
use crate::{
    Error,
    error::query_failed,
    mutation::quote_ident,
    postgres::metadata_schema::{RelationKind, load_structure},
    structure::{DroppedObject, StructurePlan},
};
use tokio_postgres::Transaction;

impl ApplicationServices {
    /// Commit exactly the frozen structure review on its captured project route.
    pub async fn apply_structure(
        &self,
        route: &ProjectTicket,
        execution_id: &str,
        plan: &StructurePlan,
        timeout_ms: impl Into<Option<u32>>,
        closed: impl Future<Output = ()>,
    ) -> WriteOutcome<usize> {
        if plan.statements().is_empty() {
            return WriteOutcome::Failed(Error::QueryFailed(
                "There are no structure changes to apply.".into(),
            ));
        }
        if route.1 != plan.snapshot().object().project {
            return WriteOutcome::Failed(super::closed());
        }
        if let Err(error) = plan.require_reviewed_type_targets() {
            return WriteOutcome::Failed(error);
        }
        run_guarded_write(
            self,
            route,
            execution_id,
            timeout_ms,
            async |tx| apply(tx, plan).await,
            closed,
        )
        .await
    }
}

fn conflict() -> Error {
    Error::QueryFailed(
        "The reviewed table, constraint, index, or schema changed. Refresh its structure and review again."
            .into(),
    )
}

async fn schema_versions(
    tx: &Transaction<'_>,
    plan: &StructurePlan,
) -> Result<Vec<SchemaVersion>, Error> {
    let mut versions = Vec::new();
    for name in &plan.namespaces {
        let version = SchemaVersion::load(tx.client(), name)
            .await?
            .ok_or_else(conflict)?;
        if name == &plan.snapshot().object().schema
            && version.oid != plan.snapshot().metadata().namespace_oid
        {
            return Err(conflict());
        }
        versions.push(version);
    }
    Ok(versions)
}

async fn apply(tx: &Transaction<'_>, plan: &StructurePlan) -> Result<usize, Error> {
    tx.batch_execute("SET TRANSACTION ISOLATION LEVEL READ COMMITTED; SET LOCAL standard_conforming_strings = on")
        .await.map_err(query_failed)?;
    let before = schema_versions(tx, plan).await?;
    let meta = plan.snapshot().metadata();
    let target = &meta.relation;
    let qualified = format!(
        "{}.{}",
        quote_ident(&target.schema),
        quote_ident(&target.name)
    );
    let version = tx
        .query_one(
            "SELECT pg_catalog.current_setting('server_version_num')::integer",
            &[],
        )
        .await
        .map_err(query_failed)?
        .get::<_, i32>(0);
    let lock_table = target.kind != RelationKind::ForeignTable
        && tx
            .query_one(
                "SELECT pg_catalog.has_table_privilege($1::oid, $2::text)",
                &[&target.oid, &exclusive_lock_privileges(version)],
            )
            .await
            .map_err(query_failed)?
            .get::<_, Option<bool>>(0)
            .unwrap_or(false);
    if !lock_table {
        if version < 120000 {
            return Err(Error::QueryFailed("This table requires PostgreSQL 12 or later to acquire an owner-only structure lock.".into()));
        }
        // LOCK TABLE excludes foreign tables and checks table grants even for the owner.
        // With table OIDs removed, this owner-only command locks without modifying the definition.
        let kind = if target.kind == RelationKind::ForeignTable {
            "FOREIGN TABLE"
        } else {
            "TABLE"
        };
        tx.batch_execute(&format!("ALTER {kind} ONLY {qualified} SET WITHOUT OIDS")).await.map_err(|e| {
            if e.code() == Some(&tokio_postgres::error::SqlState::SYNTAX_ERROR)
                || e.code() == Some(&tokio_postgres::error::SqlState::FEATURE_NOT_SUPPORTED) {
                Error::QueryFailed("This server does not support the owner-only structure lock for this table.".into())
            } else { query_failed(e) }
        })?;
    } else {
        tx.batch_execute(&format!(
            "LOCK TABLE ONLY {qualified} IN ACCESS EXCLUSIVE MODE"
        ))
        .await
        .map_err(query_failed)?;
    }
    let pinned = tx.query_one(
        "SELECT EXISTS (SELECT FROM pg_catalog.pg_locks WHERE pid = pg_catalog.pg_backend_pid()
          AND locktype = 'relation' AND relation = $1 AND granted AND mode = 'AccessExclusiveLock')",
        &[&target.oid],
    ).await.map_err(query_failed)?.get::<_, bool>(0);
    if !pinned
        || schema_versions(tx, plan).await? != before
        || load_structure(tx.client(), &target.schema, &target.name).await? != *meta
    {
        return Err(conflict());
    }
    plan.check_type_edits(tx.client()).await?;
    dependencies::check(tx, plan).await?;
    let references = foreign_keys::resolve(tx, plan).await?;
    execute_reviewed(tx, plan).await?;
    foreign_keys::verify(tx, target.oid, &references).await?;
    plan.check_type_edits(tx.client()).await?;
    if schema_versions(tx, plan).await? != before {
        return Err(conflict());
    }
    Ok(plan.statements().len())
}

async fn execute_reviewed(tx: &Transaction<'_>, plan: &StructurePlan) -> Result<(), Error> {
    for (i, statement) in plan.statements().iter().enumerate() {
        tx.batch_execute("SET LOCAL standard_conforming_strings = on")
            .await
            .map_err(query_failed)?;
        tx.execute(statement.as_str(), &[])
            .await
            .map_err(query_failed)?;
        if let Some(object) = plan.drops.get(&i) {
            let (query, oid) = match object {
                DroppedObject::Constraint(oid) => {
                    ("SELECT FROM pg_catalog.pg_constraint WHERE oid = $1", oid)
                }
                DroppedObject::Index(oid) => {
                    ("SELECT FROM pg_catalog.pg_class WHERE oid = $1", oid)
                }
            };
            if tx
                .query_opt(query, &[oid])
                .await
                .map_err(query_failed)?
                .is_some()
            {
                return Err(conflict());
            }
        }
    }
    Ok(())
}

fn exclusive_lock_privileges(version: i32) -> &'static str {
    if version >= 170000 {
        "UPDATE, DELETE, TRUNCATE, MAINTAIN"
    } else {
        "UPDATE, DELETE, TRUNCATE"
    }
}
