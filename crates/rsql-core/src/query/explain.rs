use std::error::Error as StdError;

use tokio_postgres::Client;
use tokio_postgres::types::{FromSql, Type};

use crate::error::{Error, query_failed};

/// The binary format of `json` is the JSON text itself.
struct JsonText(String);

impl<'a> FromSql<'a> for JsonText {
    fn from_sql(_: &Type, raw: &'a [u8]) -> Result<Self, Box<dyn StdError + Sync + Send>> {
        Ok(Self(std::str::from_utf8(raw)?.to_owned()))
    }

    fn accepts(ty: &Type) -> bool {
        *ty == Type::JSON
    }
}

/// Returns the JSON plan of one statement. The extended protocol rejects multiple
/// commands, so text after the target statement can never execute.
pub async fn explain_plan(client: &Client, sql: &str, analyze: bool) -> Result<String, Error> {
    let options = if analyze {
        "ANALYZE, FORMAT JSON"
    } else {
        "FORMAT JSON"
    };
    let rows = client
        .query(format!("EXPLAIN ({options}) {sql}").as_str(), &[])
        .await
        .map_err(query_failed)?;
    let row = rows
        .first()
        .ok_or_else(|| Error::QueryFailed("EXPLAIN returned no plan".into()))?;
    let JsonText(plan) = row.try_get(0).map_err(query_failed)?;
    Ok(plan)
}
