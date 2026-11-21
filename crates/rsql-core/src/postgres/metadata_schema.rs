mod constraints;
mod display;
mod indexes;
mod structure;

pub use constraints::load_constraints_typed;
pub use display::{VisibleColumn, all_columns_visible, load_visible_columns, owns_relation};
pub use indexes::load_indexes_typed;
pub use structure::*;

use tokio::time as tokio_time;
use tokio_postgres::Client;

use crate::error::{Error, query_failed};
pub type PgsqlLoadSchemas = Vec<String>;
pub type PgsqlLoadTables = Vec<(String, String)>;
pub type PgsqlLoadColumns = Vec<String>;

use super::{ColumnDetail, ConstraintDetail, IndexDetail, PolicyDetail, RuleDetail, TriggerDetail};

pub const SCHEMAS_SQL: &str = r#"SELECT schema_name FROM information_schema.schemata
           WHERE schema_name NOT IN ('pg_catalog', 'information_schema')
           ORDER BY schema_name"#;

pub const TABLES_SQL: &str = r#"SELECT table_name,
                  pg_size_pretty(pg_total_relation_size(format('%I.%I', table_schema, table_name))) AS size
           FROM information_schema.tables
           WHERE table_schema = $1
           ORDER BY table_name"#;

pub async fn load_schemas(client: &Client, query_sql: &str) -> Result<PgsqlLoadSchemas, Error> {
    let rows = tokio_time::timeout(
        tokio_time::Duration::from_secs(10),
        client.query(query_sql, &[]),
    )
    .await
    .map_err(|_| Error::QueryTimeout)?
    .map_err(query_failed)?;

    Ok(rows.iter().map(|r| r.get(0)).collect())
}

pub async fn load_databases(client: &Client) -> Result<Vec<String>, Error> {
    let rows = client
        .query(
            "SELECT datname FROM pg_database WHERE datallowconn = true AND datistemplate = false ORDER BY datname",
            &[],
        )
        .await
        .map_err(|e| Error::DatabaseError(e.to_string()))?;
    Ok(rows.iter().map(|r| r.get::<_, String>(0)).collect())
}

pub async fn load_tablespaces(client: &Client) -> Result<Vec<(String, String, String)>, Error> {
    let rows = client
        .query(
            "SELECT spcname, pg_catalog.pg_get_userbyid(spcowner) AS owner, \
             COALESCE(pg_catalog.pg_tablespace_location(oid), '') AS location \
             FROM pg_catalog.pg_tablespace ORDER BY spcname",
            &[],
        )
        .await
        .map_err(|e| Error::DatabaseError(e.to_string()))?;
    Ok(rows
        .iter()
        .map(|r| {
            (
                r.get::<_, String>(0),
                r.get::<_, String>(1),
                r.get::<_, String>(2),
            )
        })
        .collect())
}

pub async fn load_tables(
    client: &Client,
    query_sql: &str,
    schema: &str,
) -> Result<PgsqlLoadTables, Error> {
    let rows = client
        .query(query_sql, &[&schema])
        .await
        .map_err(query_failed)?;

    Ok(rows.iter().map(|r| (r.get(0), r.get(1))).collect())
}

pub async fn load_columns(
    client: &Client,
    schema: &str,
    table: &str,
) -> Result<PgsqlLoadColumns, Error> {
    let rows = client
        .query(
            r#"SELECT column_name
               FROM information_schema.columns
               WHERE table_schema = $1 AND table_name = $2
               ORDER BY ordinal_position"#,
            &[&schema, &table],
        )
        .await
        .map_err(query_failed)?;

    Ok(rows.iter().map(|r| r.get::<_, String>(0)).collect())
}

pub async fn load_column_details(
    client: &Client,
    schema: &str,
    table: &str,
) -> Result<Vec<ColumnDetail>, Error> {
    Ok(load_visible_columns(client, schema, table)
        .await?
        .into_iter()
        .map(|c| (c.name, c.formatted_type, c.nullable, c.default_value))
        .collect())
}

fn nullable(local_not_null: bool, domain_not_null: bool) -> bool {
    !(local_not_null || domain_not_null)
}

pub async fn load_indexes(
    client: &Client,
    schema: &str,
    table: &str,
) -> Result<Vec<IndexDetail>, Error> {
    Ok(load_indexes_typed(client, schema, table)
        .await?
        .into_iter()
        .flat_map(|index| {
            index.entries.into_iter().filter_map(move |entry| {
                entry
                    .column
                    .map(|column| (index.name.clone(), column.name, index.unique, index.primary))
            })
        })
        .collect())
}

pub async fn load_triggers(
    client: &Client,
    schema: &str,
    table: &str,
) -> Result<Vec<TriggerDetail>, Error> {
    let rows = client
        .query(
            r#"SELECT DISTINCT trigger_name, event_manipulation, action_timing
               FROM information_schema.triggers
               WHERE trigger_schema = $1 AND event_object_table = $2
               ORDER BY trigger_name"#,
            &[&schema, &table],
        )
        .await
        .map_err(query_failed)?;

    Ok(rows
        .iter()
        .map(|r| {
            let name: String = r.get(0);
            let event: String = r.get(1);
            let timing: String = r.get(2);
            (name, event, timing)
        })
        .collect())
}

pub async fn load_rules(
    client: &Client,
    schema: &str,
    table: &str,
) -> Result<Vec<RuleDetail>, Error> {
    let rows = client
        .query(
            r#"SELECT r.rulename::text, r.ev_type::text
               FROM pg_catalog.pg_rewrite r
               JOIN pg_catalog.pg_class c ON c.oid = r.ev_class
               JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
               WHERE n.nspname = $1 AND c.relname = $2 AND r.rulename <> '_RETURN'
               ORDER BY r.rulename"#,
            &[&schema, &table],
        )
        .await
        .map_err(query_failed)?;

    Ok(rows
        .iter()
        .map(|r| {
            let name: String = r.get(0);
            let event: String = r.get(1);
            (name, event)
        })
        .collect())
}

pub async fn load_policies(
    client: &Client,
    schema: &str,
    table: &str,
) -> Result<Vec<PolicyDetail>, Error> {
    let rows = client
        .query(
            r#"SELECT pol.polname,
                      CASE WHEN pol.polpermissive THEN 'PERMISSIVE' ELSE 'RESTRICTIVE' END,
                      CASE pol.polcmd
                        WHEN 'r' THEN 'SELECT'
                        WHEN 'a' THEN 'INSERT'
                        WHEN 'w' THEN 'UPDATE'
                        WHEN 'd' THEN 'DELETE'
                        WHEN '*' THEN 'ALL'
                        ELSE pol.polcmd::text
                      END
               FROM pg_policy pol
               JOIN pg_class c ON c.oid = pol.polrelid
               JOIN pg_namespace n ON n.oid = c.relnamespace
               WHERE n.nspname = $1 AND c.relname = $2
               ORDER BY pol.polname"#,
            &[&schema, &table],
        )
        .await
        .map_err(query_failed)?;

    Ok(rows
        .iter()
        .map(|r| {
            let name: String = r.get(0);
            let perm: String = r.get(1);
            let cmd: String = r.get(2);
            (name, perm, cmd)
        })
        .collect())
}

pub async fn load_constraints(
    client: &Client,
    schema: &str,
    table: &str,
) -> Result<Vec<ConstraintDetail>, Error> {
    let rows = client
        .query(
            r#"SELECT
                 tc.constraint_name,
                 tc.constraint_type,
                 COALESCE(kcu.column_name, '')
               FROM information_schema.table_constraints tc
               LEFT JOIN information_schema.key_column_usage kcu
                 ON kcu.constraint_name = tc.constraint_name
                 AND kcu.table_schema = tc.table_schema
                 AND kcu.table_name = tc.table_name
               WHERE tc.table_schema = $1 AND tc.table_name = $2
               ORDER BY tc.constraint_name, kcu.ordinal_position"#,
            &[&schema, &table],
        )
        .await
        .map_err(query_failed)?;

    Ok(rows
        .iter()
        .map(|r| {
            let name: String = r.get(0);
            let ctype: String = r.get(1);
            let col: String = r.get(2);
            (name, ctype, col)
        })
        .collect())
}
