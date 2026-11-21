use crate::{Error, error::query_failed, structure::StructurePlan};
use tokio_postgres::Transaction;

pub(super) async fn check(tx: &Transaction<'_>, plan: &StructurePlan) -> Result<(), Error> {
    let relation = plan.snapshot().metadata().relation.oid;
    if plan.requires_descendant_review
        && let Some(row) = tx
            .query_opt(
                "SELECT inhrelid::pg_catalog.regclass::pg_catalog.text FROM pg_catalog.pg_inherits
         WHERE inhparent = $1 LIMIT 1",
                &[&relation],
            )
            .await
            .map_err(query_failed)?
    {
        return Err(unreviewed(row.get(0)));
    }
    if plan.columns.is_empty() && plan.constraints.is_empty() && plan.indexes.is_empty() {
        return Ok(());
    }
    if !plan.indexes.is_empty()
        && let Some(row) = tx
            .query_opt(
                "SELECT idx.relname::text FROM pg_catalog.pg_index i
             JOIN pg_catalog.pg_class idx ON idx.oid=i.indexrelid
             WHERE i.indrelid=$1 AND i.indexrelid=ANY($2::oid[]) AND i.indisreplident",
                &[&relation, &plan.indexes],
            )
            .await
            .map_err(query_failed)?
    {
        let table = &plan.snapshot().metadata().relation;
        let index = row.get::<_, String>(0);
        return Err(Error::QueryFailed(format!(
            "Index {index:?} provides the replica identity of table {:?}.{:?}. Select a different replica identity before dropping it.",
            table.schema, table.name
        )));
    }
    let rows = tx
        .query(
            include_str!("dependency_check.sql"),
            &[&relation, &plan.columns, &plan.constraints, &plan.indexes],
        )
        .await
        .map_err(query_failed)?;
    if let Some(row) = rows.first() {
        return Err(unreviewed(row.get(0)));
    }
    Ok(())
}

fn unreviewed(object: String) -> Error {
    Error::QueryFailed(format!(
        "The reviewed change affects {object}. Resolve this dependency explicitly and refresh before applying."
    ))
}
