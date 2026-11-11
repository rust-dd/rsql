use tokio_postgres::Client;

use crate::common::enums::{AppError, query_failed};

use super::super::{FKDetail, ForeignKeyInfo, ObjectStats};

pub async fn load_table_statistics(
    client: &Client,
    schema: &str,
    table: &str,
) -> Result<ObjectStats, AppError> {
    let rows = client
        .query(
            r#"SELECT
                 c.reltuples::bigint::text,
                 pg_size_pretty(pg_table_size(c.oid)),
                 pg_size_pretty(pg_indexes_size(c.oid)),
                 pg_size_pretty(pg_total_relation_size(c.oid)),
                 COALESCE(s.last_vacuum::text, 'never'),
                 COALESCE(s.last_analyze::text, 'never'),
                 COALESCE(s.last_autovacuum::text, 'never'),
                 COALESCE(s.last_autoanalyze::text, 'never'),
                 COALESCE(s.n_dead_tup, 0)::text,
                 COALESCE(s.n_live_tup, 0)::text,
                 COALESCE(s.seq_scan, 0)::text,
                 COALESCE(s.idx_scan, 0)::text
               FROM pg_class c
               JOIN pg_namespace n ON n.oid = c.relnamespace
               LEFT JOIN pg_stat_user_tables s ON s.relid = c.oid
               WHERE n.nspname = $1 AND c.relname = $2
               LIMIT 1"#,
            &[&schema, &table],
        )
        .await
        .map_err(query_failed)?;

    let keys = [
        "row_estimate",
        "table_size",
        "index_size",
        "total_size",
        "last_vacuum",
        "last_analyze",
        "last_autovacuum",
        "last_autoanalyze",
        "dead_tuples",
        "live_tuples",
        "seq_scan",
        "idx_scan",
    ];

    if let Some(row) = rows.first() {
        Ok(keys
            .iter()
            .enumerate()
            .map(|(i, k)| {
                let val: Option<String> = row.try_get(i).ok();
                (k.to_string(), val.unwrap_or_else(|| "-".into()))
            })
            .collect())
    } else {
        Ok(Vec::new())
    }
}

pub async fn load_fk_details(
    client: &Client,
    schema: &str,
    table: &str,
    direction: &str, // "outgoing" or "incoming"
) -> Result<Vec<FKDetail>, AppError> {
    let where_clause = if direction == "incoming" {
        "nsp_tgt.nspname = $1 AND tgt.relname = $2"
    } else {
        "nsp.nspname = $1 AND src.relname = $2"
    };

    let sql = format!(
        r#"SELECT
             con.conname,
             nsp.nspname,
             src.relname,
             a_src.attname,
             nsp_tgt.nspname,
             tgt.relname,
             a_tgt.attname,
             CASE con.confupdtype
               WHEN 'a' THEN 'NO ACTION' WHEN 'r' THEN 'RESTRICT'
               WHEN 'c' THEN 'CASCADE'   WHEN 'n' THEN 'SET NULL'
               WHEN 'd' THEN 'SET DEFAULT' ELSE '' END,
             CASE con.confdeltype
               WHEN 'a' THEN 'NO ACTION' WHEN 'r' THEN 'RESTRICT'
               WHEN 'c' THEN 'CASCADE'   WHEN 'n' THEN 'SET NULL'
               WHEN 'd' THEN 'SET DEFAULT' ELSE '' END
           FROM pg_constraint con
           JOIN pg_class src ON src.oid = con.conrelid
           JOIN pg_namespace nsp ON nsp.oid = src.relnamespace
           JOIN pg_class tgt ON tgt.oid = con.confrelid
           JOIN pg_namespace nsp_tgt ON nsp_tgt.oid = tgt.relnamespace
           JOIN pg_attribute a_src ON a_src.attrelid = con.conrelid AND a_src.attnum = ANY(con.conkey)
           JOIN pg_attribute a_tgt ON a_tgt.attrelid = con.confrelid AND a_tgt.attnum = ANY(con.confkey)
           WHERE con.contype = 'f' AND {where_clause}
           ORDER BY con.conname"#
    );

    let rows = client
        .query(&sql, &[&schema, &table])
        .await
        .map_err(query_failed)?;

    Ok(rows
        .iter()
        .map(|r| {
            (
                r.get(0),
                r.get(1),
                r.get(2),
                r.get(3),
                r.get(4),
                r.get(5),
                r.get(6),
                r.get(7),
                r.get(8),
            )
        })
        .collect())
}

const FOREIGN_KEY_SQL: &str = include_str!("../sql/foreign_keys.sql");

pub async fn load_foreign_keys(
    client: &Client,
    schema: &str,
) -> Result<Vec<ForeignKeyInfo>, AppError> {
    let rows = client
        .query(FOREIGN_KEY_SQL, &[&schema])
        .await
        .map_err(query_failed)?;

    Ok(rows
        .iter()
        .map(|r| (r.get(0), r.get(1), r.get(2), r.get(3), r.get(4)))
        .collect())
}

pub async fn load_table_bloat(
    client: &deadpool_postgres::Client,
) -> Result<Vec<Vec<String>>, AppError> {
    let rows = client
        .query(
            "SELECT
            schemaname,
            relname AS table,
            n_live_tup::text AS live_tuples,
            n_dead_tup::text AS dead_tuples,
            CASE WHEN n_live_tup > 0
                THEN round(100.0 * n_dead_tup / (n_live_tup + n_dead_tup), 1)::text
                ELSE '0'
            END AS bloat_pct,
            pg_size_pretty(pg_total_relation_size(relid)) AS total_size,
            COALESCE(last_vacuum::text, 'never') AS last_vacuum,
            COALESCE(last_autovacuum::text, 'never') AS last_autovacuum,
            COALESCE(last_analyze::text, 'never') AS last_analyze,
            COALESCE(last_autoanalyze::text, 'never') AS last_autoanalyze
         FROM pg_stat_user_tables
         ORDER BY n_dead_tup DESC",
            &[],
        )
        .await
        .map_err(query_failed)?;

    Ok(rows
        .iter()
        .map(|r| (0..10).map(|i| r.get::<_, String>(i)).collect())
        .collect())
}
