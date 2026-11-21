use super::{
    Error, Relation, RowMutation, ToSql, Transaction, cardinality_error, query_failed, quote_ident,
};
use crate::postgres::text_param::{TextParam, captured_type};
use std::collections::HashSet;
use tokio_postgres::{Statement, types::Type};

pub(super) struct Position {
    pub cursor: String,
    count: Statement,
}

impl Position {
    pub async fn check_count(
        &self,
        tx: &Transaction<'_>,
        mutation: &RowMutation,
    ) -> Result<(), Error> {
        let values = key_values(mutation);
        let params = values
            .iter()
            .map(|v| v as &(dyn ToSql + Sync))
            .collect::<Vec<_>>();
        let count = tx
            .query_one(&self.count, &params)
            .await
            .map_err(query_failed)?
            .get::<_, i64>(0);
        if count != 1 {
            return Err(cardinality_error(mutation, count as u64));
        }
        Ok(())
    }
}

/// Under the caller's relation lock, each cursor stays on its original
/// row while PostgreSQL follows same-transaction versions for positioned DML.
pub(super) async fn open(
    tx: &Transaction<'_>,
    relation: &Relation,
    mutations: &[RowMutation],
    lock_row: bool,
) -> Result<Vec<Position>, Error> {
    let planner_settings = if relation.partitioned {
        // CURRENT OF visits each target partition. Its cursor plan must include
        // even the inactive children so PostgreSQL can identify the current one.
        let old=tx.query_one("SELECT pg_catalog.current_setting('enable_partition_pruning'), pg_catalog.current_setting('constraint_exclusion')",&[]).await.map_err(query_failed)?;
        tx.batch_execute(
            "SET LOCAL enable_partition_pruning=off; SET LOCAL constraint_exclusion=off",
        )
        .await
        .map_err(query_failed)?;
        Some((old.get::<_, String>(0), old.get::<_, String>(1)))
    } else {
        None
    };
    let table = target(relation);
    let batch = uuid::Uuid::new_v4().simple();
    let mut seen = HashSet::new();
    let mut positions = Vec::with_capacity(mutations.len());
    let mut types = Vec::new();
    let mut predicates = Vec::new();
    let mut keys = Vec::new();
    for (i, number) in relation.primary_key.iter().enumerate() {
        let attr = relation
            .attributes
            .iter()
            .find(|a| a.number == *number)
            .ok_or_else(|| Error::QueryFailed("Primary key changed.".into()))?;
        let name = quote_ident(&attr.name);
        types.push(captured_type(attr.type_id));
        predicates.push(format!("{name} = ${}", i + 1));
        keys.push(format!("{name}::pg_catalog.text"));
    }
    let scan = format!("FROM {table} WHERE {}", predicates.join(" AND "));
    let keys = keys.join(",");
    let count = typed(tx, &format!("SELECT pg_catalog.count(*) {scan}"), &types).await?;
    for (index, mutation) in mutations.iter().enumerate() {
        let cursor = format!("rsql_{batch}_{index}");
        let position = Position {
            cursor: cursor.clone(),
            count: count.clone(),
        };
        if !lock_row {
            position.check_count(tx, mutation).await?;
        }
        let locking = if lock_row { " FOR UPDATE" } else { "" };
        let declare =
            format!("DECLARE {cursor} NO SCROLL CURSOR FOR SELECT {keys} {scan}{locking}");
        let declare = typed(tx, &declare, &types).await?;
        let values = key_values(mutation);
        let params = values
            .iter()
            .map(|v| v as &(dyn ToSql + Sync))
            .collect::<Vec<_>>();
        tx.execute(&declare, &params).await.map_err(query_failed)?;
        let row = tx
            .query_opt(format!("FETCH NEXT FROM {cursor}").as_str(), &[])
            .await
            .map_err(query_failed)?
            .ok_or_else(|| cardinality_error(mutation, 0))?;
        if lock_row {
            position.check_count(tx, mutation).await?;
        }
        let key: Vec<String> = (0..row.len()).map(|i| row.get(i)).collect();
        if !seen.insert(key) {
            return Err(Error::QueryFailed(
                "Multiple changes target the same original row.".into(),
            ));
        }
        positions.push(position);
    }
    if let Some((pruning, exclusion)) = planner_settings {
        tx.query_one("SELECT pg_catalog.set_config('enable_partition_pruning',$1,true),pg_catalog.set_config('constraint_exclusion',$2,true)",&[&pruning,&exclusion]).await.map_err(query_failed)?;
    }
    Ok(positions)
}

pub(super) fn target(relation: &Relation) -> String {
    let only = if relation.partitioned { "" } else { "ONLY " };
    format!(
        "{only}{}.{}",
        quote_ident(&relation.schema),
        quote_ident(&relation.table)
    )
}

fn key_values(mutation: &RowMutation) -> Vec<TextParam<'_>> {
    mutation
        .pk
        .iter()
        .map(|(_, v)| TextParam(v.as_deref()))
        .collect()
}

async fn typed(tx: &Transaction<'_>, sql: &str, types: &[Type]) -> Result<Statement, Error> {
    let stmt = tx.prepare_typed(sql, types).await.map_err(query_failed)?;
    if !stmt
        .params()
        .iter()
        .map(Type::oid)
        .eq(types.iter().map(Type::oid))
    {
        return Err(Error::QueryFailed(
            "Primary key type changed. Run the query again.".into(),
        ));
    }
    Ok(stmt)
}
