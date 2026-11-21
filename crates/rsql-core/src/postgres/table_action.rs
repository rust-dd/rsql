use tokio_postgres::Client;

use crate::Error;
use crate::mutation::quote_ident;
use crate::query::execute_query;

pub async fn table_action(
    client: &Client,
    action: &str,
    schema: &str,
    table: &str,
    object_type: &str,
) -> Result<String, Error> {
    let qualified = format!("{}.{}", quote_ident(schema), quote_ident(table));

    let sql = match (object_type, action) {
        ("table", "ANALYZE") => format!("ANALYZE {qualified}"),
        ("table", "VACUUM") => format!("VACUUM {qualified}"),
        ("table", "VACUUM FULL") => format!("VACUUM FULL {qualified}"),
        ("table", "REINDEX") => format!("REINDEX TABLE {qualified}"),
        ("table", "TRUNCATE") => format!("TRUNCATE TABLE {qualified}"),
        ("table", "DROP TABLE") => format!("DROP TABLE {qualified}"),
        ("view", "DROP VIEW") => format!("DROP VIEW {qualified}"),
        ("view", "DROP VIEW CASCADE") => format!("DROP VIEW {qualified} CASCADE"),
        ("matview", "REFRESH") => format!("REFRESH MATERIALIZED VIEW {qualified}"),
        ("matview", "REFRESH CONCURRENTLY") => {
            format!("REFRESH MATERIALIZED VIEW CONCURRENTLY {qualified}")
        }
        ("matview", "DROP MATERIALIZED VIEW") => format!("DROP MATERIALIZED VIEW {qualified}"),
        ("function" | "trigger-function", "DROP FUNCTION") => format!("DROP FUNCTION {qualified}"),
        ("function" | "trigger-function", "DROP FUNCTION CASCADE") => {
            format!("DROP FUNCTION {qualified} CASCADE")
        }
        _ => {
            return Err(Error::QueryFailed(format!(
                "Unknown action '{}' for object type '{}'",
                action, object_type
            )));
        }
    };

    execute_query(client, &sql).await?;

    Ok(format!("{action} completed successfully."))
}
