use super::structure::{
    AttributeIdentity, ConstraintIdentity, ConstraintMetadata, ForeignKeyMetadata, ForeignKeyPair,
    RelationIdentity, RelationKind,
};
use crate::error::{Error, query_failed};
use tokio_postgres::Client;

pub async fn load_constraints_typed(
    client: &Client,
    schema: &str,
    table: &str,
) -> Result<Vec<ConstraintMetadata>, Error> {
    let rows = client.query(
        "SELECT con.oid, con.conrelid, con.conname::text, con.contype::text,
                pg_catalog.pg_get_constraintdef(con.oid, false),
                CASE WHEN con.contype IN ('p','u','x') THEN NULLIF(con.conindid, 0) END,
                ARRAY(SELECT k.num FROM unnest(con.conkey) WITH ORDINALITY k(num, pos) WHERE k.num > 0 ORDER BY k.pos),
                ARRAY(SELECT a.attname::text FROM unnest(con.conkey) WITH ORDINALITY k(num, pos)
                      JOIN pg_catalog.pg_attribute a ON a.attrelid = con.conrelid AND a.attnum = k.num ORDER BY k.pos),
                t.oid, nt.nspname::text, t.relname::text, t.relkind::text,
                COALESCE(con.confkey, '{}'::smallint[]),
                ARRAY(SELECT a.attname::text FROM unnest(con.confkey) WITH ORDINALITY k(num, pos)
                      JOIN pg_catalog.pg_attribute a ON a.attrelid = con.confrelid AND a.attnum = k.num ORDER BY k.pos),
                con.confupdtype::text, con.confdeltype::text, con.confmatchtype::text, con.conindid
         FROM pg_catalog.pg_constraint con
         JOIN pg_catalog.pg_class c ON c.oid = con.conrelid
         JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
         LEFT JOIN pg_catalog.pg_class t ON t.oid = con.confrelid
         LEFT JOIN pg_catalog.pg_namespace nt ON nt.oid = t.relnamespace
         WHERE n.nspname = $1 AND c.relname = $2 ORDER BY con.conname, con.oid",
        &[&schema, &table],
    ).await.map_err(query_failed)?;
    rows.iter()
        .map(|r| {
            let source_oid = r.get(1);
            let columns = attributes(source_oid, r.get(6), r.get(7));
            let target_oid: Option<u32> = r.get(8);
            let foreign_key = if let Some(oid) = target_oid {
                let targets = attributes(oid, r.get(12), r.get(13));
                Some(ForeignKeyMetadata {
                    referenced_index_oid: r.get(17),
                    target: RelationIdentity {
                        oid,
                        schema: r.get(9),
                        name: r.get(10),
                        kind: RelationKind::from_catalog(r.get(11))?,
                    },
                    columns: columns
                        .iter()
                        .cloned()
                        .zip(targets)
                        .map(|(source, target)| ForeignKeyPair { source, target })
                        .collect(),
                    on_update: action(r.get(14)).into(),
                    on_delete: action(r.get(15)).into(),
                    match_type: r.get(16),
                })
            } else {
                None
            };
            Ok(ConstraintMetadata {
                identity: ConstraintIdentity {
                    oid: r.get(0),
                    relation_oid: source_oid,
                    name: r.get(2),
                },
                kind: r.get(3),
                columns,
                definition: r.get(4),
                backing_index_oid: r.get(5),
                foreign_key,
            })
        })
        .collect()
}

fn attributes(relation_oid: u32, numbers: Vec<i16>, names: Vec<String>) -> Vec<AttributeIdentity> {
    numbers
        .into_iter()
        .zip(names)
        .map(|(number, name)| AttributeIdentity {
            relation_oid,
            number,
            name,
        })
        .collect()
}

fn action(value: &str) -> &str {
    match value {
        "a" => "NO ACTION",
        "r" => "RESTRICT",
        "c" => "CASCADE",
        "n" => "SET NULL",
        "d" => "SET DEFAULT",
        other => other,
    }
}
