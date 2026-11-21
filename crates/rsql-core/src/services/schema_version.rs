use crate::{Error, error::query_failed};
use tokio_postgres::Client;

#[derive(Debug, PartialEq, Eq)]
pub(super) struct SchemaVersion {
    pub(super) oid: u32,
    xmin: String,
    ctid: String,
}

impl SchemaVersion {
    pub(super) async fn load(client: &Client, name: &str) -> Result<Option<Self>, Error> {
        let row = client
            .query_opt(
                "SELECT oid, xmin::pg_catalog.text, ctid::pg_catalog.text
             FROM pg_catalog.pg_namespace WHERE nspname = $1",
                &[&name],
            )
            .await
            .map_err(query_failed)?;
        Ok(row.map(|row| Self {
            oid: row.get(0),
            xmin: row.get(1),
            ctid: row.get(2),
        }))
    }
}
