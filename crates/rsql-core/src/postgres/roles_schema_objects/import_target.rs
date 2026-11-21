use crate::{Error, error::query_failed, mutation::quote_ident};
use std::collections::{BTreeSet, HashSet};
use tokio_postgres::{GenericClient, Row, Transaction};

type Version = (u32, String, String);
fn version(row: &Row, index: usize) -> Version {
    (row.get(index), row.get(index + 1), row.get(index + 2))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportColumn {
    pub name: String,
    pub(super) position: i16,
    pub(super) type_oid: u32,
    pub(super) bind_domain: bool,
    pub(super) base_oid: u32,
    pub(super) copy_input: bool,
    type_modifier: i32,
    collation: u32,
    identity: String,
    generated: String,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ImportTable {
    pub schema: String,
    pub name: String,
    pub(in crate::postgres) oid: u32,
    namespaces: BTreeSet<Version>,
    types: BTreeSet<Version>,
    pub columns: Vec<ImportColumn>,
}

impl ImportTable {
    pub fn relation_oid(&self) -> u32 {
        self.oid
    }

    pub async fn load(
        client: &impl GenericClient,
        schema: &str,
        name: &str,
    ) -> Result<Self, Error> {
        let copy_column = super::import_input::COPY_COLUMN;
        let rows = client.query(&*format!(
            "SELECT c.oid, a.attname::text, a.attnum, a.atttypid, a.atttypmod, a.attcollation,
                    a.attidentity::text, a.attgenerated::text, n.oid,n.xmin::text,n.ctid::text,
                    named.oid,named.xmin::text,named.ctid::text,
                    tn.oid,tn.xmin::text,tn.ctid::text, base.oid, {copy_column},
                    t.typbasetype<>0 AND base.typreceive=0 AND a.atttypmod=-1,
                    base.oids, base.versions, base.locations
             FROM pg_catalog.pg_class c
             JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace
             JOIN pg_catalog.pg_attribute a ON a.attrelid=c.oid AND a.attnum>0 AND NOT a.attisdropped
             JOIN pg_catalog.pg_type t ON t.oid=a.atttypid
             LEFT JOIN pg_catalog.pg_type e ON e.oid=t.typelem AND e.typarray=t.oid
             JOIN pg_catalog.pg_type named ON named.oid=COALESCE(e.oid,t.oid)
             JOIN pg_catalog.pg_namespace tn ON tn.oid=named.typnamespace
             CROSS JOIN LATERAL (
                 WITH RECURSIVE bases(oid,parent,receive,version,location) AS (
                     SELECT t.oid,t.typbasetype,t.typreceive,t.xmin::text,t.ctid::text
                     UNION ALL
                     SELECT b.oid,b.typbasetype,b.typreceive,b.xmin::text,b.ctid::text FROM bases JOIN pg_catalog.pg_type b ON b.oid=bases.parent
                 ) SELECT leaf.oid,leaf.receive AS typreceive,
                       chain.oids, chain.versions, chain.locations
                   FROM bases leaf CROSS JOIN (
                       SELECT pg_catalog.array_agg(oid ORDER BY oid) AS oids,
                              pg_catalog.array_agg(version ORDER BY oid) AS versions,
                              pg_catalog.array_agg(location ORDER BY oid) AS locations FROM bases
                   ) chain WHERE leaf.parent=0
             ) base
             WHERE n.nspname=$1 AND c.relname=$2 AND c.relkind IN ('r','p') ORDER BY a.attnum"),
            &[&schema,&name],
        ).await.map_err(query_failed)?;
        let oid = rows
            .first()
            .ok_or_else(|| {
                Error::QueryFailed("Import table or its columns are unavailable".into())
            })?
            .get(0);
        let mut namespaces = BTreeSet::from([version(&rows[0], 8)]);
        let mut types = BTreeSet::new();
        for row in &rows {
            let oids = row.get::<_, Vec<u32>>("oids");
            let versions = row.get::<_, Vec<String>>("versions");
            let locations = row.get::<_, Vec<String>>("locations");
            types.extend(
                oids.into_iter()
                    .zip(versions)
                    .zip(locations)
                    .map(|((oid, xmin), ctid)| (oid, xmin, ctid)),
            );
            types.insert(version(row, 11));
            namespaces.insert(version(row, 14));
        }
        Ok(Self {
            namespaces,
            types,
            schema: schema.into(),
            name: name.into(),
            oid,
            columns: rows
                .into_iter()
                .map(|r| ImportColumn {
                    name: r.get(1),
                    position: r.get(2),
                    type_oid: r.get(3),
                    bind_domain: r.get(19),
                    base_oid: r.get(17),
                    copy_input: r.get(18),
                    type_modifier: r.get(4),
                    collation: r.get(5),
                    identity: r.get(6),
                    generated: r.get(7),
                })
                .collect(),
        })
    }

    pub(super) fn mapped_columns(
        &self,
        mapping: &[(usize, String)],
    ) -> Result<Vec<&ImportColumn>, Error> {
        if mapping.is_empty() {
            return Err(Error::QueryFailed("Map at least one column".into()));
        }
        let mut sources = HashSet::new();
        let mut targets = HashSet::new();
        mapping
            .iter()
            .map(|(source, name)| {
                if !sources.insert(*source) || !targets.insert(name) {
                    return Err(Error::QueryFailed("Duplicate column mapping".into()));
                }
                let column = self
                    .columns
                    .iter()
                    .find(|c| &c.name == name)
                    .ok_or_else(|| Error::QueryFailed(format!("Unknown column {name}")))?;
                Ok(column)
            })
            .collect()
    }

    pub async fn lock_current(&self, tx: &Transaction<'_>) -> Result<(), Error> {
        // Parse/Describe takes the relation lock without executing a SELECT or
        // requiring a table-level grant from a column-only INSERT role.
        let column = &self.columns[0];
        let described = tx
            .prepare(&format!(
                "SELECT {} FROM ONLY {}.{}",
                quote_ident(&column.name),
                quote_ident(&self.schema),
                quote_ident(&self.name)
            ))
            .await
            .map_err(query_failed)?;
        if !matches!(described.columns(),[c] if c.table_oid()==Some(self.oid) && c.column_id()==Some(column.position))
            || Self::load(tx, &self.schema, &self.name).await? != *self
        {
            return Err(Error::QueryFailed(
                "Import target changed. Reopen the table import.".into(),
            ));
        }
        Ok(())
    }
    /// A fresh catalog snapshot after Bind checks the schema and type names used
    /// to construct this portal. Rename-back also changes a catalog row version.
    pub async fn check_binding(&self, client: &tokio_postgres::Client) -> Result<(), Error> {
        if self.binding_current(client).await? {
            Ok(())
        } else {
            Err(Error::QueryFailed(
                "Import target changed. Reopen the table import.".into(),
            ))
        }
    }

    pub(in crate::postgres) async fn binding_current(
        &self,
        client: &tokio_postgres::Client,
    ) -> Result<bool, Error> {
        let namespaces = self.namespaces.iter().map(|v| v.0).collect::<Vec<_>>();
        let types = self.types.iter().map(|v| v.0).collect::<Vec<_>>();
        let rows=client.query(
            "SELECT false,oid,xmin::text,ctid::text FROM pg_catalog.pg_namespace WHERE oid=ANY($1)
             UNION ALL SELECT true,oid,xmin::text,ctid::text FROM pg_catalog.pg_type WHERE oid=ANY($2)",
            &[&namespaces,&types]).await.map_err(query_failed)?;
        Ok(rows.len() == namespaces.len() + types.len()
            && rows.iter().all(|r| {
                let expected = if r.get::<_, bool>(0) {
                    &self.types
                } else {
                    &self.namespaces
                };
                expected.contains(&version(r, 1))
            }))
    }
}
