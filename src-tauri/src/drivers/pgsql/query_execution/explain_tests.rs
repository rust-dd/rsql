//! EXPLAIN semantics against a disposable PostgreSQL database.

use tokio_postgres::{Client, NoTls};

use super::explain_plan;

async fn client_with_probe() -> Client {
    let url = std::env::var("RSQL_TEST_DATABASE_URL").expect("set RSQL_TEST_DATABASE_URL");
    let (client, connection) = tokio_postgres::connect(&url, NoTls).await.unwrap();
    tokio::spawn(async move {
        connection.await.expect("PostgreSQL connection failed");
    });
    client
        .batch_execute(
            "CREATE TEMP TABLE explain_probe (a int); INSERT INTO explain_probe VALUES (1)",
        )
        .await
        .unwrap();
    client
}

async fn probe_values(client: &Client) -> Vec<i32> {
    let rows = client
        .query("SELECT a FROM explain_probe ORDER BY a", &[])
        .await
        .unwrap();
    rows.iter().map(|row| row.get(0)).collect()
}

fn root_plan(plan: &str) -> serde_json::Value {
    let plans: serde_json::Value = serde_json::from_str(plan).unwrap();
    plans[0]["Plan"].clone()
}

#[tokio::test]
#[ignore = "requires RSQL_TEST_DATABASE_URL"]
async fn explain_estimates_without_running_the_statement() {
    let client = client_with_probe().await;
    let plan = explain_plan(&client, "UPDATE explain_probe SET a = 2", false)
        .await
        .unwrap();
    let root = root_plan(&plan);
    assert_eq!(root["Node Type"], "ModifyTable");
    assert!(root.get("Actual Rows").is_none());
    assert_eq!(probe_values(&client).await, vec![1]);
}

#[tokio::test]
#[ignore = "requires RSQL_TEST_DATABASE_URL"]
async fn trailing_statements_are_rejected_and_never_run() {
    let client = client_with_probe().await;
    for analyze in [false, true] {
        let result = explain_plan(&client, "SELECT 1; DELETE FROM explain_probe", analyze).await;
        assert!(result.is_err(), "analyze={analyze} accepted two statements");
        assert_eq!(probe_values(&client).await, vec![1]);
    }
}

#[tokio::test]
#[ignore = "requires RSQL_TEST_DATABASE_URL"]
async fn analyze_executes_the_statement_and_reports_actuals() {
    let client = client_with_probe().await;
    let plan = explain_plan(&client, "UPDATE explain_probe SET a = 3", true)
        .await
        .unwrap();
    assert!(root_plan(&plan).get("Actual Rows").is_some());
    assert_eq!(probe_values(&client).await, vec![3]);
}
