use super::{SchemaIdentity, TableMetadata, assemble::Builder, owner::Request};
use rsql_core::{
    ApplicationServices, Error,
    error::query_failed,
    postgres::{Client, metadata_schema as meta},
};
use std::sync::Arc;

pub(super) async fn load(
    services: Arc<ApplicationServices>,
    request: Request,
) -> Result<Option<Builder>, String> {
    request.check()?;
    services
        .run_metadata(&request.project, request.retired(), async |client| {
            let ticket = services.capture_project_ticket(&request.project)?;
            if !ticket.same_generation(&request.route) {
                return Err(Error::ConnectionFailed("Connection changed".into()));
            }
            read(client, &request.schema, || request.check()).await
        })
        .await
        .map_err(|e| e.to_string())
}

async fn read(
    client: &Client,
    schema: &str,
    check: impl Fn() -> Result<(), String>,
) -> Result<Option<Builder>, Error> {
    check().map_err(Error::QueryFailed)?;
    client
        .batch_execute("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .await
        .map_err(query_failed)?;
    let row = client.query_opt(
        "SELECT oid FROM pg_catalog.pg_namespace WHERE nspname=$1 AND pg_catalog.has_schema_privilege(oid,'USAGE')",
        &[&schema],
    ).await.map_err(query_failed)?;
    let Some(row) = row else {
        client.batch_execute("COMMIT").await.map_err(query_failed)?;
        return Ok(None);
    };
    let namespace = row.get::<_, u32>(0);
    let copies = client
        .query(
        "SELECT con.oid FROM pg_catalog.pg_constraint con
         JOIN pg_catalog.pg_constraint parent ON parent.oid=con.conparentid AND parent.conrelid=con.conrelid
         JOIN pg_catalog.pg_class source ON source.oid=con.conrelid
         WHERE con.contype='f' AND source.relnamespace=$1",
        &[&namespace],
    )
    .await
    .map_err(query_failed)?;
    let mut internal = std::collections::HashSet::with_capacity(copies.len());
    for row in copies {
        check().map_err(Error::QueryFailed)?;
        internal.insert(row.get::<_, u32>(0));
    }
    let mut builder = Builder::new(SchemaIdentity {
        oid: namespace,
        name: schema.into(),
    });
    let tables = meta::load_relation_group(client, schema, &["r", "p", "f"], false).await?;
    for table in tables {
        check().map_err(Error::QueryFailed)?;
        let relation = table.identity;
        let name = &relation.name;
        let columns = section(client, async {
            meta::load_visible_columns(client, schema, name).await
        })
        .await?;
        check().map_err(Error::QueryFailed)?;
        if columns.as_ref().is_ok_and(Vec::is_empty) {
            let classification = section(client, async {
                meta::all_columns_visible(client, relation.oid).await
            })
            .await?;
            match classification {
                Ok(true) => (),
                Ok(false) => {
                    builder.visibility_failure(
                        relation,
                        "No columns visible with current grants".into(),
                    );
                    continue;
                }
                Err(error) => {
                    builder.visibility_failure(
                        relation,
                        format!("Could not classify the empty column list. {error}"),
                    );
                    continue;
                }
            }
        }
        let constraints =
            section(client, meta::load_constraints_typed(client, schema, name)).await?;
        let constraints = match constraints {
            Ok(rows) => {
                let mut declared = Vec::with_capacity(rows.len());
                for row in rows {
                    check().map_err(Error::QueryFailed)?;
                    if !internal.contains(&row.identity.oid) {
                        declared.push(row);
                    }
                }
                Ok(declared)
            }
            Err(error) => Err(error),
        };
        builder
            .add(
                TableMetadata {
                    relation,
                    columns,
                    constraints,
                },
                &check,
            )
            .map_err(Error::QueryFailed)?;
    }
    client.batch_execute("COMMIT").await.map_err(query_failed)?;
    Ok(Some(builder))
}

// Keep one snapshot across tables even when PostgreSQL aborts an individual statement.
async fn section<T>(
    client: &Client,
    read: impl Future<Output = Result<T, Error>>,
) -> Result<Result<T, String>, Error> {
    client
        .batch_execute("SAVEPOINT erd_section")
        .await
        .map_err(query_failed)?;
    let result = read.await;
    if result.is_err() {
        client
            .batch_execute("ROLLBACK TO erd_section")
            .await
            .map_err(query_failed)?;
    }
    client
        .batch_execute("RELEASE erd_section")
        .await
        .map_err(query_failed)?;
    Ok(result.map_err(|e| e.to_string()))
}
