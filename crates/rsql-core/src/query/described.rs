use super::{VirtualCache, execute_virtual};
use crate::{
    Error,
    error::query_failed,
    postgres::provenance::{ResultOrigin, describe_cursor},
};
use tokio::sync::Mutex;
use tokio_postgres::Client;

pub type PackedResult = (String, usize, String, f32, bool);

/// Describe and fetch the same bound portal. Materialization still consumes the
/// entire query once using the existing cap, memory budget and snapshot cache.
pub async fn execute_described(
    client: &Client,
    cache: &Mutex<VirtualCache>,
    sql: &str,
    id: &str,
    size: usize,
    relation: Option<&str>,
) -> Result<(PackedResult, Option<ResultOrigin>), Error> {
    let Some(name) = relation else {
        return Ok((execute_virtual(client, cache, sql, id, size).await?, None));
    };
    let started = std::time::Instant::now();
    client.batch_execute("BEGIN").await.map_err(query_failed)?;
    // Extended Parse enforces one statement even if the client and server
    // disagree about SQL lexing. No rows have been fetched during setup.
    let declare = format!("DECLARE rsql_result NO SCROLL CURSOR FOR {sql}");
    if client.execute(&*declare, &[]).await.is_err() {
        client
            .batch_execute("ROLLBACK")
            .await
            .map_err(query_failed)?;
        return Ok((execute_virtual(client, cache, sql, id, size).await?, None));
    }
    const FETCH: &str = "FETCH FORWARD ALL FROM rsql_result";
    let mut data = execute_virtual(client, cache, FETCH, id, size).await?;
    data.3 = started.elapsed().as_secs_f32() * 1000.;
    let origin = if data.1 > 0 && data.1 <= size && !data.4 {
        client
            .batch_execute("SAVEPOINT rsql_origin")
            .await
            .map_err(query_failed)?;
        match describe_cursor(client, name, FETCH).await {
            Ok(origin) => origin,
            Err(_) => {
                client
                    .batch_execute("ROLLBACK TO SAVEPOINT rsql_origin")
                    .await
                    .map_err(query_failed)?;
                None
            }
        }
    } else {
        None
    };
    client.batch_execute("COMMIT").await.map_err(query_failed)?;
    Ok((data, origin))
}
