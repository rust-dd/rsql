use tokio_postgres::Client;

use crate::error::{Error, query_failed};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RelationKind {
    Table,
    PartitionedTable,
    ForeignTable,
    View,
    MaterializedView,
}

impl RelationKind {
    pub(crate) fn from_catalog(value: &str) -> Result<Self, Error> {
        match value {
            "r" => Ok(Self::Table),
            "p" => Ok(Self::PartitionedTable),
            "f" => Ok(Self::ForeignTable),
            "v" => Ok(Self::View),
            "m" => Ok(Self::MaterializedView),
            other => Err(Error::QueryFailed(format!(
                "Unsupported relation kind: {other}"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RelationIdentity {
    pub oid: u32,
    pub schema: String,
    pub name: String,
    pub kind: RelationKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttributeIdentity {
    pub relation_oid: u32,
    pub number: i16,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnMetadata {
    pub identity: AttributeIdentity,
    pub type_oid: u32,
    pub type_modifier: i32,
    pub formatted_type: String,
    pub nullable: bool,
    pub local_not_null: bool,
    pub domain_not_null: bool,
    pub collation: Option<ColumnCollation>,
    pub default_value: Option<String>,
    pub identity_generation: String,
    pub generated: String,
    pub generation_expression: Option<String>,
    pub comment: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnCollation {
    pub oid: u32,
    pub schema: String,
    pub name: String,
    pub preserve_explicitly: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConstraintIdentity {
    pub oid: u32,
    pub relation_oid: u32,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForeignKeyPair {
    pub source: AttributeIdentity,
    pub target: AttributeIdentity,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForeignKeyMetadata {
    /// The target index PostgreSQL selected to enforce this reference (pg_constraint.conindid).
    pub referenced_index_oid: u32,
    pub target: RelationIdentity,
    pub columns: Vec<ForeignKeyPair>,
    pub on_update: String,
    pub on_delete: String,
    pub match_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConstraintMetadata {
    pub identity: ConstraintIdentity,
    pub kind: String,
    /// Declared attribute keys only; expression keys remain in the definition and backing index.
    pub columns: Vec<AttributeIdentity>,
    pub definition: String,
    pub backing_index_oid: Option<u32>,
    pub foreign_key: Option<ForeignKeyMetadata>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexEntry {
    pub position: i64,
    pub column: Option<AttributeIdentity>,
    pub included: bool,
    pub definition: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexMetadata {
    pub oid: u32,
    pub relation_oid: u32,
    pub name: String,
    pub unique: bool,
    pub primary: bool,
    pub valid: bool,
    pub method: String,
    pub definition: String,
    pub predicate: Option<String>,
    pub constraint: Option<ConstraintIdentity>,
    pub entries: Vec<IndexEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructureMetadata {
    pub namespace_oid: u32,
    pub relation: RelationIdentity,
    pub columns: Vec<ColumnMetadata>,
    pub constraints: Vec<ConstraintMetadata>,
    pub indexes: Vec<IndexMetadata>,
}

pub async fn load_relation(
    client: &Client,
    schema: &str,
    name: &str,
) -> Result<RelationIdentity, Error> {
    Ok(relation_and_namespace(client, schema, name).await?.0)
}

pub(crate) async fn relation_and_namespace(
    client: &Client,
    schema: &str,
    name: &str,
) -> Result<(RelationIdentity, u32), Error> {
    let row = client
        .query_opt(
            "SELECT c.oid, c.relkind::text, n.oid FROM pg_catalog.pg_class c
         JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
         WHERE n.nspname = $1 AND c.relname = $2",
            &[&schema, &name],
        )
        .await
        .map_err(query_failed)?
        .ok_or_else(|| Error::QueryFailed(format!("Relation not found: {schema}.{name}")))?;
    Ok((
        RelationIdentity {
            oid: row.get(0),
            schema: schema.into(),
            name: name.into(),
            kind: RelationKind::from_catalog(row.get(1))?,
        },
        row.get(2),
    ))
}

pub async fn load_structure(
    client: &Client,
    schema: &str,
    table: &str,
) -> Result<StructureMetadata, Error> {
    let (relation, namespace_oid) = relation_and_namespace(client, schema, table).await?;
    Ok(StructureMetadata {
        relation,
        namespace_oid,
        columns: load_columns_typed(client, schema, table).await?,
        constraints: super::load_constraints_typed(client, schema, table).await?,
        indexes: super::load_indexes_typed(client, schema, table).await?,
    })
}

pub async fn load_columns_typed(
    client: &Client,
    schema: &str,
    table: &str,
) -> Result<Vec<ColumnMetadata>, Error> {
    let rows = client
        .query(
            concat!("SELECT c.oid, a.attnum, a.attname::text, a.atttypid, a.atttypmod,
                pg_catalog.format_type(a.atttypid, a.atttypmod), a.attnotnull,
                CASE WHEN a.attgenerated = '' THEN pg_catalog.pg_get_expr(d.adbin, d.adrelid) END, a.attidentity::text,
                a.attgenerated::text, pg_catalog.col_description(c.oid, a.attnum),
                CASE WHEN a.attgenerated <> '' THEN pg_catalog.pg_get_expr(d.adbin, d.adrelid) END,
                ", include_str!("domain_not_null.sql"), ", coll.oid, cn.nspname::text, coll.collname::text,
                a.attcollation <> t.typcollation OR coll.oid <> 'pg_catalog.\"default\"'::pg_catalog.regcollation
         FROM pg_catalog.pg_class c
         JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
         JOIN pg_catalog.pg_attribute a ON a.attrelid = c.oid
         JOIN pg_catalog.pg_type t ON t.oid = a.atttypid
         LEFT JOIN pg_catalog.pg_collation coll ON coll.oid = a.attcollation
         LEFT JOIN pg_catalog.pg_namespace cn ON cn.oid = coll.collnamespace
         LEFT JOIN pg_catalog.pg_attrdef d ON d.adrelid = c.oid AND d.adnum = a.attnum
         WHERE n.nspname = $1 AND c.relname = $2 AND a.attnum > 0 AND NOT a.attisdropped
         ORDER BY a.attnum"),
            &[&schema, &table],
        )
        .await
        .map_err(query_failed)?;
    Ok(rows
        .iter()
        .map(|r| ColumnMetadata {
            identity: AttributeIdentity {
                relation_oid: r.get(0),
                number: r.get(1),
                name: r.get(2),
            },
            type_oid: r.get(3),
            type_modifier: r.get(4),
            formatted_type: r.get(5),
            nullable: super::nullable(r.get(6), r.get(12)),
            local_not_null: r.get(6),
            domain_not_null: r.get(12),
            collation: r.get::<_, Option<u32>>(13).map(|oid| ColumnCollation {
                oid,
                schema: r.get(14),
                name: r.get(15),
                preserve_explicitly: r.get(16),
            }),
            default_value: r.get(7),
            identity_generation: r.get(8),
            generated: r.get(9),
            generation_expression: r.get(11),
            comment: r.get(10),
        })
        .collect())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelationSummary {
    pub identity: RelationIdentity,
    pub size: String,
}

pub async fn load_relations(client: &Client, schema: &str) -> Result<Vec<RelationSummary>, Error> {
    load_relation_group(client, schema, &["r", "p", "f", "v", "m"], true).await
}

pub async fn load_relation_group(
    client: &Client,
    schema: &str,
    kinds: &[&str],
    include_size: bool,
) -> Result<Vec<RelationSummary>, Error> {
    let rows = client.query(
        "SELECT c.oid, c.relname::text, c.relkind::text, CASE WHEN $3 THEN pg_catalog.pg_size_pretty(pg_catalog.pg_total_relation_size(c.oid)) END
         FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
         WHERE n.nspname = $1 AND c.relkind::text = ANY($2::text[]) AND (
             (c.relkind IN ('r','p','f') AND EXISTS (SELECT 1 FROM information_schema.tables visible
                 WHERE visible.table_schema = n.nspname AND visible.table_name = c.relname))
             OR (c.relkind = 'v' AND EXISTS (SELECT 1 FROM information_schema.views visible
                 WHERE visible.table_schema = n.nspname AND visible.table_name = c.relname))
             OR c.relkind = 'm') ORDER BY c.relname, c.oid", &[&schema, &kinds, &include_size]
    ).await.map_err(query_failed)?;
    rows.iter()
        .map(|r| {
            Ok(RelationSummary {
                identity: RelationIdentity {
                    oid: r.get(0),
                    schema: schema.into(),
                    name: r.get(1),
                    kind: RelationKind::from_catalog(r.get(2))?,
                },
                size: r
                    .get::<_, Option<String>>(3)
                    .unwrap_or_else(|| "Unavailable".into()),
            })
        })
        .collect()
}
