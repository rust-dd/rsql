use super::*;
use deadpool_postgres::{Manager, Pool};
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio_postgres::NoTls;

struct CsvFile(std::path::PathBuf);

impl CsvFile {
    fn new(content: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "rsql-csv-test-{}-{}.csv",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&path, content).unwrap();
        Self(path)
    }
    fn path(&self) -> &str {
        self.0.to_str().unwrap()
    }
}

impl Drop for CsvFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

async fn client() -> deadpool_postgres::Client {
    let config = std::env::var("RSQL_TEST_DATABASE_URL")
        .expect("set RSQL_TEST_DATABASE_URL")
        .parse::<tokio_postgres::Config>()
        .unwrap();
    let pool = Pool::builder(Manager::new(config, NoTls))
        .max_size(1)
        .build()
        .unwrap();
    let client = pool.get().await.unwrap();
    client.batch_execute("CREATE TEMP TABLE csv_test (id int, amount numeric(10,2), born date, ok boolean, note text)").await.unwrap();
    client
}

fn mapping() -> Vec<(usize, String)> {
    ["id", "amount", "born", "ok", "note"]
        .into_iter()
        .enumerate()
        .map(|(i, name)| (i, name.into()))
        .collect()
}

#[tokio::test]
#[ignore = "requires RSQL_TEST_DATABASE_URL"]
async fn production_import_preserves_types_nulls_and_copy_escape_characters() {
    let mut client = client().await;
    let file = CsvFile::new(
        "id,amount,born,ok,note\n1,9.50,2020-01-02,true,\"árvíz 🦀\\N\tline\nnext\"\n2,,,,\n",
    );
    assert_eq!(
        import_csv_to_table(&mut client, file.path(), "pg_temp", "csv_test", &mapping())
            .await
            .unwrap(),
        2
    );
    let row = client
        .query_one(
            "SELECT amount::text, born::text, ok, note FROM csv_test WHERE id=1",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(row.get::<_, String>(0), "9.50");
    assert_eq!(row.get::<_, String>(1), "2020-01-02");
    assert!(row.get::<_, bool>(2));
    assert_eq!(row.get::<_, String>(3), "árvíz 🦀\\N\tline\nnext");
    assert!(client.query_one("SELECT amount IS NULL AND born IS NULL AND ok IS NULL AND note IS NULL FROM csv_test WHERE id=2", &[]).await.unwrap().get::<_, bool>(0));
}

#[tokio::test]
#[ignore = "requires RSQL_TEST_DATABASE_URL"]
async fn a_bad_typed_row_reports_its_position_and_rolls_back_the_whole_import() {
    let mut client = client().await;
    let file = CsvFile::new("id,amount,born,ok,note\n1,9.50,,,valid\n2,not-a-number,,,invalid\n");
    let error = import_csv_to_table(&mut client, file.path(), "pg_temp", "csv_test", &mapping())
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("invalid input syntax"), "{error}");
    assert!(error.contains("line 2"), "{error}");
    assert_eq!(
        client
            .query_one("SELECT count(*) FROM csv_test", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        0
    );
    assert!(
        client
            .query_one("SELECT current_setting('transaction_isolation')", &[])
            .await
            .is_ok()
    );
}

#[tokio::test]
#[ignore = "requires RSQL_TEST_DATABASE_URL"]
async fn malformed_csv_and_unknown_columns_fail_without_partial_writes() {
    let mut client = client().await;
    let file = CsvFile::new("id,amount,born,ok,note\n1,1,,,valid\n2,2\n");
    let error = import_csv_to_table(&mut client, file.path(), "pg_temp", "csv_test", &mapping())
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("row 2"), "{error}");
    assert_eq!(
        client
            .query_one("SELECT count(*) FROM csv_test", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        0
    );
    assert!(
        import_csv_to_table(
            &mut client,
            file.path(),
            "pg_temp",
            "csv_test",
            &[(0, "unknown".into())]
        )
        .await
        .is_err()
    );
}

#[tokio::test]
#[ignore = "requires RSQL_TEST_DATABASE_URL"]
async fn quoted_identifiers_and_reordered_mappings_use_production_copy() {
    let mut client = client().await;
    client
        .batch_execute("CREATE TEMP TABLE \"csv\"\"table\" (\"num\"\"ber\" int, label text)")
        .await
        .unwrap();
    let file = CsvFile::new("label,number\ntext,42\n");
    let mapping = [(1, "num\"ber".into()), (0, "label".into())];
    assert_eq!(
        import_csv_to_table(&mut client, file.path(), "pg_temp", "csv\"table", &mapping)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        client
            .query_one("SELECT \"num\"\"ber\" FROM \"csv\"\"table\"", &[])
            .await
            .unwrap()
            .get::<_, i32>(0),
        42
    );
}

#[tokio::test]
#[ignore = "requires RSQL_TEST_DATABASE_URL"]
async fn row_security_uses_insert_and_rolls_back_rejected_rows() {
    let mut client = client().await;
    let role = format!("rsql_csv_policy_{}", std::process::id());
    let temp_schema = client
        .query_one("SELECT pg_my_temp_schema()::regnamespace::text", &[])
        .await
        .unwrap()
        .get::<_, String>(0);
    let temp_schema = quote_ident(&temp_schema);
    client
        .batch_execute(&format!(
            "CREATE ROLE {role};
         GRANT USAGE ON SCHEMA {temp_schema} TO {role};
         GRANT SELECT, INSERT ON csv_test TO {role};
         ALTER TABLE csv_test ENABLE ROW LEVEL SECURITY;
         CREATE POLICY positive_ids ON csv_test USING (id > 0);
         SET ROLE {role}"
        ))
        .await
        .unwrap();
    let file = CsvFile::new("id,amount,born,ok,note\n1,1,,,valid\n-1,2,,,rejected\n");
    let error = import_csv_to_table(&mut client, file.path(), "pg_temp", "csv_test", &mapping())
        .await
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("row 2") && error.contains("row-level security"),
        "{error}"
    );
    client.batch_execute("RESET ROLE").await.unwrap();
    assert_eq!(
        client
            .query_one("SELECT count(*) FROM csv_test", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        0
    );
    client
        .batch_execute(&format!(
            "DROP TABLE csv_test; REVOKE USAGE ON SCHEMA {temp_schema} FROM {role}; DROP ROLE {role}"
        ))
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires RSQL_TEST_DATABASE_URL"]
async fn generated_always_identity_and_views_preserve_insert_semantics() {
    let mut client = client().await;
    client
        .batch_execute(
            "CREATE TEMP TABLE csv_identity (id int GENERATED ALWAYS AS IDENTITY, note text);
        CREATE TEMP VIEW csv_view AS SELECT id, note FROM csv_test",
        )
        .await
        .unwrap();
    let file = CsvFile::new("id,note\n42,value\n");
    let mapping = [(0, "id".into()), (1, "note".into())];
    let error = import_csv_to_table(
        &mut client,
        file.path(),
        "pg_temp",
        "csv_identity",
        &mapping,
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(error.contains("GENERATED ALWAYS"), "{error}");
    assert_eq!(
        import_csv_to_table(&mut client, file.path(), "pg_temp", "csv_view", &mapping)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        client
            .query_one("SELECT id FROM csv_test", &[])
            .await
            .unwrap()
            .get::<_, i32>(0),
        42
    );
}

#[tokio::test]
#[ignore = "requires RSQL_TEST_DATABASE_URL; prints local import benchmark"]
async fn benchmark_copy_and_per_row_import() {
    let mut client = client().await;
    let columns = vec![
        ("id".into(), "integer".into()),
        ("note".into(), "text".into()),
    ];
    let mapping = [(0, "id".into()), (1, "note".into())];
    let mut content = String::from("id,note\n");
    for row in 0..20_000 {
        use std::fmt::Write;
        writeln!(&mut content, "{row},unicode árvíz 🦀 and some text").unwrap();
    }
    let file = CsvFile::new(&content);
    let mut timings = [Vec::new(), Vec::new()];
    for iteration in 0..6 {
        for mode in [iteration % 2, 1 - iteration % 2] {
            client.batch_execute("TRUNCATE csv_test").await.unwrap();
            let start = std::time::Instant::now();
            if mode == 0 {
                let tx = client.transaction().await.unwrap();
                let statement = tx
                    .prepare(&build_insert("pg_temp", "csv_test", &columns))
                    .await
                    .unwrap();
                let mut reader = csv::Reader::from_path(file.path()).unwrap();
                for record in reader.records() {
                    let record = record.unwrap();
                    let values = record
                        .iter()
                        .map(|value| Some(value.to_string()))
                        .collect::<Vec<_>>();
                    let params = values
                        .iter()
                        .map(|value| value as &(dyn tokio_postgres::types::ToSql + Sync))
                        .collect::<Vec<_>>();
                    tx.execute(&statement, &params).await.unwrap();
                }
                tx.commit().await.unwrap();
            } else {
                assert_eq!(
                    import_csv_to_table(&mut client, file.path(), "pg_temp", "csv_test", &mapping)
                        .await
                        .unwrap(),
                    20_000
                );
            }
            if iteration > 0 {
                timings[mode].push(start.elapsed().as_secs_f64() * 1000.0);
            }
            assert_eq!(
                client
                    .query_one("SELECT count(*) FROM csv_test", &[])
                    .await
                    .unwrap()
                    .get::<_, i64>(0),
                20_000
            );
        }
    }
    for (name, values) in ["per-row INSERT", "production COPY"]
        .into_iter()
        .zip(&mut timings)
    {
        values.sort_by(f64::total_cmp);
        println!(
            "CSV 20K rows, {name}: median {:.2} ms, max {:.2} ms (5 warm samples)",
            values[2], values[4]
        );
    }
}
