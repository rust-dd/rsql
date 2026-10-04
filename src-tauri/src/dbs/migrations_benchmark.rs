use super::cleanup_legacy_snapshots;

#[tokio::test]
#[ignore = "prints a local on-disk startup-migration benchmark"]
async fn benchmark_repeated_startup_cleanup() {
    let path =
        std::env::temp_dir().join(format!("rsql-startup-benchmark-{}.db", std::process::id()));
    let db = libsql::Builder::new_local(&path).build().await.unwrap();
    let conn = db.connect().unwrap();
    conn.execute_batch(
        "CREATE TABLE history (sql TEXT);
        WITH RECURSIVE rows(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM rows WHERE x < 50000)
        INSERT INTO history SELECT 'SELECT ' || hex(zeroblob(256)) FROM rows;",
    )
    .await
    .unwrap();
    for populated in [true, false] {
        if !populated {
            conn.execute("DELETE FROM history", ()).await.unwrap();
            conn.execute("VACUUM", ()).await.unwrap();
        }
        let bytes = std::fs::metadata(&path).unwrap().len();
        let mut timings = [Vec::new(), Vec::new()];
        for iteration in 0..11 {
            for mode in [iteration % 2, 1 - iteration % 2] {
                let start = std::time::Instant::now();
                if mode == 0 {
                    conn.execute_batch(
                        "DROP TABLE IF EXISTS virtual_query_pages;
                        DROP TABLE IF EXISTS virtual_query_snapshots; VACUUM;",
                    )
                    .await
                    .unwrap();
                } else {
                    assert!(!cleanup_legacy_snapshots(&conn).await.unwrap());
                }
                if iteration > 0 {
                    timings[mode].push(start.elapsed().as_secs_f64() * 1000.0);
                }
            }
        }
        for (name, values) in ["repeat VACUUM", "production skip"]
            .into_iter()
            .zip(&mut timings)
        {
            values.sort_by(f64::total_cmp);
            println!(
                "Startup migration, {bytes} bytes, {name}: median {:.3} ms, p95 {:.3} ms (10 warm samples)",
                (values[4] + values[5]) / 2.0,
                values[9]
            );
        }
    }
    drop(conn);
    drop(db);
    std::fs::remove_file(path).unwrap();
}
