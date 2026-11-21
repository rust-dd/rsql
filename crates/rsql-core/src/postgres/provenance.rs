//! Execution-time column origins for a single, already bound PostgreSQL cursor.
use super::statistics_activity::FOREIGN_KEY_SQL;
use crate::{Error, error::query_failed};
use tokio_postgres::{Client, GenericClient, types::Type};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attribute {
    pub number: i16,
    pub name: String,
    pub writable: bool,
    pub(super) type_id: u32,
    modifier: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Relation {
    pub oid: u32,
    pub schema: String,
    pub table: String,
    pub primary_key: Vec<i16>,
    pub attributes: Vec<Attribute>,
    pub descendant_safe: bool,
    pub partitioned: bool,
}

impl Attribute {
    fn lossless_family(oid: u32) -> Option<Type> {
        match Type::from_oid(oid)? {
            Type::INT2 | Type::INT4 | Type::INT8 => Some(Type::INT8),
            Type::TEXT | Type::VARCHAR => Some(Type::TEXT),
            ty @ (Type::BOOL | Type::UUID) => Some(ty),
            _ => None,
        }
    }
}

impl Relation {
    pub fn lossless_key(&self) -> bool {
        self.primary_key.iter().all(|n| {
            self.attributes
                .iter()
                .any(|a| a.number == *n && Attribute::lossless_family(a.type_id).is_some())
        })
    }
}

#[derive(Clone, Debug)]
pub struct ForeignReference {
    pub source_column: String,
    pub schema: String,
    pub table: String,
    pub column: String,
    pub partitioned: bool,
    pub lossless: bool,
}

const RELATION: &str = "
SELECT c.oid, n.nspname::text, c.relname::text,
       COALESCE((SELECT k.conkey FROM pg_catalog.pg_constraint k
                 WHERE k.conrelid=c.oid AND k.contype='p'), ARRAY[]::smallint[]),
       (c.relkind='p' OR NOT EXISTS (SELECT FROM pg_catalog.pg_inherits i WHERE i.inhparent=c.oid)),
       a.attnum, a.attname::text, (a.attgenerated='' AND a.attidentity<>'a'), a.atttypid, a.atttypmod,c.relkind='p'
FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace
JOIN pg_catalog.pg_attribute a ON a.attrelid=c.oid AND a.attnum>0 AND NOT a.attisdropped
WHERE c.oid=pg_catalog.to_regclass($1) AND c.relkind IN ('r','p') AND c.relpersistence <> 't'
ORDER BY a.attnum";

pub async fn load_relation(
    client: &impl GenericClient,
    name: &str,
) -> Result<Option<Relation>, Error> {
    let rows = client
        .query(RELATION, &[&name])
        .await
        .map_err(query_failed)?;
    let Some(first) = rows.first() else {
        return Ok(None);
    };
    Ok(Some(Relation {
        oid: first.get(0),
        schema: first.get(1),
        table: first.get(2),
        primary_key: first.get(3),
        descendant_safe: first.get(4),
        partitioned: first.get(10),
        attributes: rows
            .iter()
            .map(|r| Attribute {
                number: r.get(5),
                name: r.get(6),
                writable: r.get(7),
                type_id: r.get(8),
                modifier: r.get(9),
            })
            .collect(),
    }))
}

#[derive(Clone, Debug)]
pub struct ResultOrigin {
    pub relation: Relation,
    pub output: Vec<Option<i16>>,
    pub labels: Vec<String>,
    pub foreign_keys: Vec<ForeignReference>,
}

pub async fn describe_cursor(
    client: &Client,
    relation_name: &str,
    fetch: &str,
) -> Result<Option<ResultOrigin>, Error> {
    let stmt = client.prepare(fetch).await.map_err(query_failed)?;
    let Some(relation) = load_relation(client, relation_name).await? else {
        return Ok(None);
    };
    let output: Vec<_> = stmt
        .columns()
        .iter()
        .map(|c| {
            (c.table_oid() == Some(relation.oid))
                .then(|| c.column_id())
                .flatten()
                .filter(|n| relation.attributes.iter().any(|a| a.number == *n))
        })
        .collect();
    if output.iter().all(Option::is_none) {
        return Ok(None);
    }
    let labels = stmt.columns().iter().map(|c| c.name().to_owned()).collect();
    let foreign_keys = if relation.descendant_safe {
        client
            .query(FOREIGN_KEY_SQL, &[&relation.schema])
            .await
            .map_err(query_failed)?
            .iter()
            .filter(|r| r.get::<_, &str>(0) == relation.table)
            .map(|r| {
                let source_column: String = r.get(1);
                let source = relation
                    .attributes
                    .iter()
                    .find(|a| a.name == source_column)
                    .and_then(|a| Attribute::lossless_family(a.type_id));
                ForeignReference {
                    source_column,
                    schema: r.get(2),
                    table: r.get(3),
                    column: r.get(4),
                    partitioned: r.get(5),
                    lossless: source.is_some() && source == Attribute::lossless_family(r.get(6)),
                }
            })
            .collect()
    } else {
        Vec::new()
    };
    Ok(Some(ResultOrigin {
        relation,
        output,
        labels,
        foreign_keys,
    }))
}
