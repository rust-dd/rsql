use super::super::VirtualCache;
use super::{close_virtual, execute_virtual};
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, get_current_pid};
use tokio::sync::Mutex;

fn rss(system: &mut System) -> u64 {
    let pid = get_current_pid().unwrap();
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        true,
        ProcessRefreshKind::nothing().with_memory(),
    );
    system.process(pid).unwrap().memory()
}

#[tokio::test]
#[ignore = "requires RSQL_TEST_DATABASE_URL; prints backend cache and RSS measurements"]
async fn benchmark_retained_results_and_cleanup() {
    let url = std::env::var("RSQL_TEST_DATABASE_URL").expect("set RSQL_TEST_DATABASE_URL");
    let (client, connection) = tokio_postgres::connect(&url, tokio_postgres::NoTls)
        .await
        .unwrap();
    tokio::spawn(async move { connection.await.unwrap() });
    let cache = Mutex::new(VirtualCache::new());
    let mut system = System::new();
    println!(
        "Backend process RSS before queries: {} bytes",
        rss(&mut system)
    );
    for rows in [10_000, 100_000, 1_000_000] {
        for width in [32, 2048] {
            let sql = format!(
                "SELECT n, json_build_object('text', repeat('x', {width})) FROM generate_series(1, {rows}) n"
            );
            let start = std::time::Instant::now();
            let result = execute_virtual(&client, &cache, &sql, "benchmark", 2000)
                .await
                .unwrap();
            let used = cache.lock().await.budget.used();
            assert!(used <= 256 * 1024 * 1024);
            println!(
                "{rows} rows, text width {width}: retained {}, capped {}, budget {} bytes, RSS {} bytes, elapsed {:.1} ms",
                result.1,
                result.4,
                used,
                rss(&mut system),
                start.elapsed().as_secs_f64() * 1000.0
            );
            drop(result);
            close_virtual(&cache, "benchmark").await.unwrap();
            assert!(cache.lock().await.is_empty());
            assert_eq!(cache.lock().await.budget.used(), 0);
            println!(
                "After close: budget 0 bytes, entries 0, RSS {} bytes",
                rss(&mut system)
            );
        }
    }
}
