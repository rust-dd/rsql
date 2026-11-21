mod table;
mod views;

pub use super::metadata_views_functions::generate_routine_ddl;
use crate::error::Error;
use tokio_postgres::Client;

/// Generates supported object definitions; external dependencies must already exist when replaying.
pub async fn generate_full_ddl(
    client: &Client,
    schema: &str,
    name: &str,
    object_type: &str,
) -> Result<String, Error> {
    match object_type {
        "table" => table::generate(client, schema, name).await,
        "view" => views::generate(client, schema, name, false).await,
        "matview" => views::generate(client, schema, name, true).await,
        "function" | "trigger-function" => {
            super::metadata_views_functions::routines::legacy_definition(client, schema, name).await
        }
        _ => Err(Error::QueryFailed(format!(
            "Unknown object type: {object_type}"
        ))),
    }
}
