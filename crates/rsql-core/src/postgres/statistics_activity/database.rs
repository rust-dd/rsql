use tokio_postgres::Client;

use crate::error::{Error, query_failed};

use super::super::DbStat;

pub async fn load_activity(client: &Client) -> Result<Vec<Vec<String>>, Error> {
    let rows = client
        .query(
            r#"SELECT
                 pid::text,
                 COALESCE(usename, '') AS usename,
                 COALESCE(datname, '') AS datname,
                 COALESCE(state, 'unknown') AS state,
                 COALESCE(wait_event_type, '') AS wait_event_type,
                 COALESCE(wait_event, '') AS wait_event,
                 COALESCE(LEFT(query, 500), '') AS query,
                 COALESCE(EXTRACT(EPOCH FROM (now() - query_start))::text, '0') AS duration_sec,
                 COALESCE(backend_type, '') AS backend_type,
                 COALESCE(client_addr::text, 'local') AS client_addr
               FROM pg_stat_activity
               WHERE datname = current_database()
               ORDER BY state, query_start NULLS LAST"#,
            &[],
        )
        .await
        .map_err(query_failed)?;

    Ok(rows
        .iter()
        .map(|r| (0..10).map(|i| r.get::<_, String>(i)).collect())
        .collect())
}

pub async fn load_database_stats(client: &Client) -> Result<Vec<DbStat>, Error> {
    let rows = client
        .query(
            r#"SELECT
                 'Active Connections' AS stat, numbackends::text AS val FROM pg_stat_database WHERE datname = current_database()
               UNION ALL
               SELECT 'Transactions Committed', xact_commit::text FROM pg_stat_database WHERE datname = current_database()
               UNION ALL
               SELECT 'Transactions Rolled Back', xact_rollback::text FROM pg_stat_database WHERE datname = current_database()
               UNION ALL
               SELECT 'Blocks Read (disk)', blks_read::text FROM pg_stat_database WHERE datname = current_database()
               UNION ALL
               SELECT 'Blocks Hit (cache)', blks_hit::text FROM pg_stat_database WHERE datname = current_database()
               UNION ALL
               SELECT 'Cache Hit Ratio',
                 CASE WHEN (blks_hit + blks_read) > 0
                   THEN ROUND(blks_hit::numeric / (blks_hit + blks_read) * 100, 2)::text || '%'
                   ELSE 'N/A'
                 END
               FROM pg_stat_database WHERE datname = current_database()
               UNION ALL
               SELECT 'Rows Returned', tup_returned::text FROM pg_stat_database WHERE datname = current_database()
               UNION ALL
               SELECT 'Rows Fetched', tup_fetched::text FROM pg_stat_database WHERE datname = current_database()
               UNION ALL
               SELECT 'Rows Inserted', tup_inserted::text FROM pg_stat_database WHERE datname = current_database()
               UNION ALL
               SELECT 'Rows Updated', tup_updated::text FROM pg_stat_database WHERE datname = current_database()
               UNION ALL
               SELECT 'Rows Deleted', tup_deleted::text FROM pg_stat_database WHERE datname = current_database()
               UNION ALL
               SELECT 'Temp Files', temp_files::text FROM pg_stat_database WHERE datname = current_database()
               UNION ALL
               SELECT 'Temp Bytes', pg_size_pretty(temp_bytes) FROM pg_stat_database WHERE datname = current_database()
               UNION ALL
               SELECT 'Deadlocks', deadlocks::text FROM pg_stat_database WHERE datname = current_database()
               UNION ALL
               SELECT 'Database Size', pg_size_pretty(pg_database_size(current_database()))"#,
            &[],
        )
        .await
        .map_err(query_failed)?;

    Ok(rows
        .iter()
        .map(|r| {
            let name: String = r.get(0);
            let val: String = r.get(1);
            (name, val)
        })
        .collect())
}

pub async fn load_table_stats(client: &Client) -> Result<Vec<Vec<String>>, Error> {
    let rows = client
        .query(
            r#"SELECT
                 schemaname,
                 relname,
                 COALESCE(seq_scan, 0)::text AS seq_scan,
                 COALESCE(seq_tup_read, 0)::text AS seq_tup_read,
                 COALESCE(idx_scan, 0)::text AS idx_scan,
                 COALESCE(idx_tup_fetch, 0)::text AS idx_tup_fetch,
                 COALESCE(n_tup_ins, 0)::text AS inserts,
                 COALESCE(n_tup_upd, 0)::text AS updates,
                 COALESCE(n_tup_del, 0)::text AS deletes,
                 COALESCE(n_live_tup, 0)::text AS live_tuples,
                 COALESCE(n_dead_tup, 0)::text AS dead_tuples,
                 COALESCE(last_vacuum::text, 'never') AS last_vacuum,
                 COALESCE(last_autovacuum::text, 'never') AS last_autovacuum,
                 COALESCE(last_analyze::text, 'never') AS last_analyze
               FROM pg_stat_user_tables
               ORDER BY seq_scan + COALESCE(idx_scan, 0) DESC
               LIMIT 100"#,
            &[],
        )
        .await
        .map_err(query_failed)?;

    Ok(rows
        .iter()
        .map(|r| (0..14).map(|i| r.get::<_, String>(i)).collect())
        .collect())
}

pub async fn load_active_locks(client: &tokio_postgres::Client) -> Result<Vec<Vec<String>>, Error> {
    let rows = client
        .query(
            "SELECT
            l.pid::text,
            COALESCE(a.usename, '') AS user,
            COALESCE(l.mode, '') AS mode,
            COALESCE(l.locktype, '') AS locktype,
            CASE WHEN l.granted THEN 'granted' ELSE 'waiting' END AS status,
            COALESCE(c.relname, '') AS relation,
            COALESCE(n.nspname, '') AS schema,
            COALESCE(left(a.query, 200), '') AS query,
            COALESCE(extract(epoch from now() - a.query_start)::text, '0') AS duration,
            COALESCE(a.wait_event_type || ':' || a.wait_event, '') AS wait_event
         FROM pg_locks l
         JOIN pg_stat_activity a ON a.pid = l.pid
         LEFT JOIN pg_class c ON c.oid = l.relation
         LEFT JOIN pg_namespace n ON n.oid = c.relnamespace
         WHERE a.pid != pg_backend_pid()
         ORDER BY NOT l.granted, l.pid",
            &[],
        )
        .await
        .map_err(query_failed)?;

    Ok(rows
        .iter()
        .map(|r| (0..10).map(|i| r.get::<_, String>(i)).collect())
        .collect())
}

pub async fn load_index_usage(client: &tokio_postgres::Client) -> Result<Vec<Vec<String>>, Error> {
    let rows = client
        .query(
            "SELECT
            s.schemaname,
            s.relname AS table,
            s.indexrelname AS index,
            pg_size_pretty(pg_relation_size(i.indexrelid)) AS size,
            COALESCE(s.idx_scan, 0)::text AS scans,
            COALESCE(s.idx_tup_read, 0)::text AS tuples_read,
            COALESCE(s.idx_tup_fetch, 0)::text AS tuples_fetched,
            CASE
                WHEN s.idx_scan = 0 THEN 'unused'
                WHEN s.idx_scan < 10 THEN 'rarely_used'
                ELSE 'active'
            END AS status,
            COALESCE(pg_get_indexdef(i.indexrelid), '') AS definition
         FROM pg_stat_user_indexes s
         JOIN pg_index i ON i.indexrelid = s.indexrelid
         WHERE NOT i.indisprimary
         ORDER BY s.idx_scan ASC, pg_relation_size(i.indexrelid) DESC",
            &[],
        )
        .await
        .map_err(query_failed)?;

    Ok(rows
        .iter()
        .map(|r| (0..9).map(|i| r.get::<_, String>(i)).collect())
        .collect())
}
