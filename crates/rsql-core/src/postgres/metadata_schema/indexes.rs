use super::structure::{AttributeIdentity, ConstraintIdentity, IndexEntry, IndexMetadata};
use crate::error::{Error, query_failed};
use tokio_postgres::Client;

pub async fn load_indexes_typed(
    client: &Client,
    schema: &str,
    table: &str,
) -> Result<Vec<IndexMetadata>, Error> {
    let rows = client
        .query(
            "SELECT i.indexrelid, i.indrelid, idx.relname::text, i.indisunique, i.indisprimary,
                i.indisvalid, am.amname::text, pg_catalog.pg_get_indexdef(i.indexrelid),
                pg_catalog.pg_get_expr(i.indpred, i.indrelid), con.oid, con.conname::text,
                k.pos, a.attnum, a.attname::text, k.pos > i.indnkeyatts,
                pg_catalog.pg_get_indexdef(i.indexrelid, k.pos::int, false)
         FROM pg_catalog.pg_index i
         JOIN pg_catalog.pg_class idx ON idx.oid = i.indexrelid
         JOIN pg_catalog.pg_am am ON am.oid = idx.relam
         JOIN pg_catalog.pg_class c ON c.oid = i.indrelid
         JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
         CROSS JOIN LATERAL unnest(i.indkey) WITH ORDINALITY k(num, pos)
         LEFT JOIN pg_catalog.pg_attribute a ON a.attrelid = i.indrelid AND a.attnum = k.num
         LEFT JOIN pg_catalog.pg_constraint con ON con.conrelid = i.indrelid
              AND con.conindid = i.indexrelid AND con.contype IN ('p','u','x')
         WHERE n.nspname = $1 AND c.relname = $2 ORDER BY idx.relname, i.indexrelid, k.pos",
            &[&schema, &table],
        )
        .await
        .map_err(query_failed)?;
    let mut indexes: Vec<IndexMetadata> = Vec::new();
    for r in rows {
        let oid = r.get(0);
        let relation_oid = r.get(1);
        if indexes.last().is_none_or(|i| i.oid != oid) {
            indexes.push(IndexMetadata {
                oid,
                relation_oid,
                name: r.get(2),
                unique: r.get(3),
                primary: r.get(4),
                valid: r.get(5),
                method: r.get(6),
                definition: r.get(7),
                predicate: r.get(8),
                constraint: r.get::<_, Option<u32>>(9).map(|oid| ConstraintIdentity {
                    oid,
                    relation_oid,
                    name: r.get(10),
                }),
                entries: Vec::new(),
            });
        }
        indexes
            .last_mut()
            .expect("index just inserted")
            .entries
            .push(IndexEntry {
                position: r.get(11),
                column: r.get::<_, Option<i16>>(12).map(|number| AttributeIdentity {
                    relation_oid,
                    number,
                    name: r.get(13),
                }),
                included: r.get(14),
                definition: r.get(15),
            });
    }
    Ok(indexes)
}
