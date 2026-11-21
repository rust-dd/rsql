use crate::error::{Error, query_failed};
use tokio_postgres::Client;

pub struct VisibleColumn {
    pub number: i16,
    pub name: String,
    pub formatted_type: String,
    pub nullable: bool,
    pub default_value: Option<String>,
    pub generation_expression: Option<String>,
    pub identity_generation: Option<String>,
    pub primary: bool,
}

pub async fn load_visible_columns(
    client: &Client,
    schema: &str,
    table: &str,
) -> Result<Vec<VisibleColumn>, Error> {
    let rows = client.query(
        concat!("SELECT a.attnum, v.column_name::text, pg_catalog.format_type(a.atttypid, a.atttypmod),
            a.attnotnull, v.column_default::text, v.generation_expression::text, v.identity_generation::text,
            EXISTS (SELECT FROM pg_catalog.pg_constraint pk WHERE pk.conrelid = c.oid
                AND pk.contype = 'p' AND a.attnum = ANY(pk.conkey)), ", include_str!("domain_not_null.sql"), "
         FROM information_schema.columns v
         JOIN pg_catalog.pg_namespace n ON n.nspname = v.table_schema
         JOIN pg_catalog.pg_class c ON c.relnamespace = n.oid AND c.relname = v.table_name
         JOIN pg_catalog.pg_attribute a ON a.attrelid = c.oid AND a.attnum = v.ordinal_position
         JOIN pg_catalog.pg_type t ON t.oid = a.atttypid
         WHERE v.table_schema = $1 AND v.table_name = $2 ORDER BY a.attnum"), &[&schema, &table]
    ).await.map_err(query_failed)?;
    Ok(rows
        .into_iter()
        .map(|r| VisibleColumn {
            number: r.get(0),
            name: r.get(1),
            formatted_type: r.get(2),
            nullable: super::nullable(r.get(3), r.get(8)),
            default_value: r.get(4),
            generation_expression: r.get(5),
            identity_generation: r.get(6),
            primary: r.get(7),
        })
        .collect())
}

pub async fn owns_relation(client: &Client, oid: u32) -> Result<bool, Error> {
    Ok(client.query_one(
        "SELECT pg_catalog.pg_has_role(c.relowner, 'USAGE') FROM pg_catalog.pg_class c WHERE c.oid = $1",
        &[&oid]).await.map_err(query_failed)?.get(0))
}

pub async fn all_columns_visible(client: &Client, oid: u32) -> Result<bool, Error> {
    Ok(client.query_one(
        "SELECT NOT EXISTS (SELECT FROM pg_catalog.pg_attribute a
            JOIN pg_catalog.pg_class c ON c.oid = a.attrelid
            JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
            WHERE a.attrelid = $1 AND a.attnum > 0 AND NOT a.attisdropped
            AND NOT EXISTS (SELECT FROM information_schema.columns v WHERE v.table_schema = n.nspname
                AND v.table_name = c.relname AND v.ordinal_position = a.attnum))", &[&oid]
    ).await.map_err(query_failed)?.get(0))
}
