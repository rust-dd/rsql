pub(crate) mod routines;
pub use routines::{
    RoutineIdentity, RoutineKind, RoutineMetadata, generate_routine_ddl, load_routine_info,
    load_routines, resolve_routine,
};

use tokio_postgres::Client;

use crate::error::{Error, query_failed};

use super::{FunctionInfo, ObjectStats};

pub async fn load_views(client: &Client, schema: &str) -> Result<Vec<String>, Error> {
    let rows = client
        .query(
            r#"SELECT table_name
               FROM information_schema.views
               WHERE table_schema = $1
               ORDER BY table_name"#,
            &[&schema],
        )
        .await
        .map_err(query_failed)?;

    Ok(rows.iter().map(|r| r.get::<_, String>(0)).collect())
}

pub async fn load_materialized_views(client: &Client, schema: &str) -> Result<Vec<String>, Error> {
    let rows = client
        .query(
            r#"SELECT matviewname
               FROM pg_matviews
               WHERE schemaname = $1
               ORDER BY matviewname"#,
            &[&schema],
        )
        .await
        .map_err(query_failed)?;

    Ok(rows.iter().map(|r| r.get::<_, String>(0)).collect())
}

pub async fn load_functions(client: &Client, schema: &str) -> Result<Vec<FunctionInfo>, Error> {
    Ok(load_routines(client, schema)
        .await?
        .into_iter()
        .filter(|r| !r.is_trigger && r.return_type.is_some())
        .map(|r| {
            (
                r.identity.name,
                r.return_type.unwrap_or_default(),
                r.arguments,
            )
        })
        .collect())
}

pub async fn load_trigger_functions(
    client: &Client,
    schema: &str,
) -> Result<Vec<(String, String)>, Error> {
    Ok(load_routines(client, schema)
        .await?
        .into_iter()
        .filter(|r| r.is_trigger)
        .map(|r| (r.identity.name, r.arguments))
        .collect())
}

pub async fn load_view_info(
    client: &Client,
    schema: &str,
    view: &str,
) -> Result<ObjectStats, Error> {
    let rows = client
        .query(
            r#"SELECT
                 COALESCE(v.is_updatable, 'NO'),
                 COALESCE(v.check_option, 'NONE'),
                 pg_get_viewdef(c.oid, true)
               FROM information_schema.views v
               JOIN pg_class c ON c.relname = v.table_name
               JOIN pg_namespace n ON n.oid = c.relnamespace AND n.nspname = v.table_schema
               WHERE v.table_schema = $1 AND v.table_name = $2
               LIMIT 1"#,
            &[&schema, &view],
        )
        .await
        .map_err(query_failed)?;

    if let Some(row) = rows.first() {
        Ok(vec![
            ("is_updatable".into(), row.get::<_, String>(0)),
            ("check_option".into(), row.get::<_, String>(1)),
            ("definition".into(), row.get::<_, String>(2)),
        ])
    } else {
        Ok(Vec::new())
    }
}

pub async fn load_matview_info(
    client: &Client,
    schema: &str,
    matview: &str,
) -> Result<ObjectStats, Error> {
    let sql = r#"SELECT
         c.reltuples::bigint::text,
         pg_size_pretty(pg_total_relation_size(c.oid)),
         CASE WHEN m.ispopulated THEN 'YES' ELSE 'NO' END,
         m.definition
       FROM pg_matviews m
       JOIN pg_class c ON c.relname = m.matviewname
       JOIN pg_namespace n ON n.oid = c.relnamespace AND n.nspname = m.schemaname
       WHERE m.schemaname = $1 AND m.matviewname = $2
       LIMIT 1"#;

    let rows = client
        .query(sql, &[&schema, &matview])
        .await
        .map_err(query_failed)?;

    if let Some(row) = rows.first() {
        Ok(vec![
            ("row_estimate".into(), row.get::<_, String>(0)),
            ("total_size".into(), row.get::<_, String>(1)),
            ("is_populated".into(), row.get::<_, String>(2)),
            ("definition".into(), row.get::<_, String>(3)),
        ])
    } else {
        Ok(Vec::new())
    }
}

pub async fn load_function_info(
    client: &Client,
    schema: &str,
    func_name: &str,
) -> Result<ObjectStats, Error> {
    match routines::lookup(client, schema, func_name).await? {
        Some(routine) => routines::info(client, routine.identity.oid).await,
        None => Ok(Vec::new()),
    }
}
