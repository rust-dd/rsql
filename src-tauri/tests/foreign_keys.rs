//! Foreign key listing against a real PostgreSQL. Ignored by default; see
//! tests/row_mutations.rs for how to run them.

use tokio_postgres::{Client, NoTls};

const FOREIGN_KEY_SQL: &str = include_str!("../src/drivers/pgsql/sql/foreign_keys.sql");

type Fk = (String, String, String, String, String);

async fn connect() -> Client {
    let url = std::env::var("RSQL_TEST_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/postgres".to_string());
    let (client, connection) = tokio_postgres::connect(&url, NoTls)
        .await
        .expect("connect to the test database");
    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("connection error: {e}");
        }
    });
    client
}

async fn foreign_keys(client: &Client, schema: &str) -> Vec<Fk> {
    client
        .query(FOREIGN_KEY_SQL, &[&schema])
        .await
        .expect("list foreign keys")
        .iter()
        .map(|r| (r.get(0), r.get(1), r.get(2), r.get(3), r.get(4)))
        .collect()
}

fn fk(table: &str, column: &str, target_schema: &str, target_table: &str, target: &str) -> Fk {
    (
        table.to_string(),
        column.to_string(),
        target_schema.to_string(),
        target_table.to_string(),
        target.to_string(),
    )
}

#[tokio::test]
#[ignore = "requires a PostgreSQL server"]
async fn constraints_sharing_a_name_keep_their_own_targets() {
    let client = connect().await;
    client
        .batch_execute(
            "DROP SCHEMA IF EXISTS rsql_fk_names CASCADE;
             CREATE SCHEMA rsql_fk_names;
             CREATE TABLE rsql_fk_names.customers (id int PRIMARY KEY);
             CREATE TABLE rsql_fk_names.invoices (id int PRIMARY KEY, customer_id int,
                 CONSTRAINT fk_parent FOREIGN KEY (customer_id)
                 REFERENCES rsql_fk_names.customers(id));
             CREATE TABLE rsql_fk_names.payments (id int PRIMARY KEY, invoice_id int,
                 CONSTRAINT fk_parent FOREIGN KEY (invoice_id)
                 REFERENCES rsql_fk_names.invoices(id));",
        )
        .await
        .unwrap();

    assert_eq!(
        foreign_keys(&client, "rsql_fk_names").await,
        vec![
            fk(
                "invoices",
                "customer_id",
                "rsql_fk_names",
                "customers",
                "id"
            ),
            fk("payments", "invoice_id", "rsql_fk_names", "invoices", "id"),
        ]
    );
}

#[tokio::test]
#[ignore = "requires a PostgreSQL server"]
async fn composite_keys_pair_columns_by_position() {
    let client = connect().await;
    client
        .batch_execute(
            "DROP SCHEMA IF EXISTS rsql_fk_composite CASCADE;
             CREATE SCHEMA rsql_fk_composite;
             CREATE TABLE rsql_fk_composite.order_lines (
                 order_id int, line_no int, PRIMARY KEY (order_id, line_no));
             CREATE TABLE rsql_fk_composite.shipments (id int PRIMARY KEY, ord int, line int,
                 FOREIGN KEY (ord, line)
                 REFERENCES rsql_fk_composite.order_lines(order_id, line_no));",
        )
        .await
        .unwrap();

    assert_eq!(
        foreign_keys(&client, "rsql_fk_composite").await,
        vec![
            fk(
                "shipments",
                "line",
                "rsql_fk_composite",
                "order_lines",
                "line_no"
            ),
            fk(
                "shipments",
                "ord",
                "rsql_fk_composite",
                "order_lines",
                "order_id"
            ),
        ]
    );
}

#[tokio::test]
#[ignore = "requires a PostgreSQL server"]
async fn a_target_in_another_schema_keeps_its_schema() {
    let client = connect().await;
    client
        .batch_execute(
            "DROP SCHEMA IF EXISTS rsql_fk_cross, rsql_fk_cross_auth CASCADE;
             CREATE SCHEMA rsql_fk_cross;
             CREATE SCHEMA rsql_fk_cross_auth;
             CREATE TABLE rsql_fk_cross_auth.users (id int PRIMARY KEY);
             CREATE TABLE rsql_fk_cross.profiles (id int PRIMARY KEY,
                 user_id int REFERENCES rsql_fk_cross_auth.users(id));",
        )
        .await
        .unwrap();

    assert_eq!(
        foreign_keys(&client, "rsql_fk_cross").await,
        vec![fk(
            "profiles",
            "user_id",
            "rsql_fk_cross_auth",
            "users",
            "id"
        )]
    );
}

#[tokio::test]
#[ignore = "requires a PostgreSQL server"]
async fn a_role_with_only_select_still_sees_foreign_keys() {
    let client = connect().await;
    client
        .batch_execute(
            "DROP SCHEMA IF EXISTS rsql_fk_reader CASCADE;
             CREATE SCHEMA rsql_fk_reader;
             CREATE TABLE rsql_fk_reader.parent (id int PRIMARY KEY);
             CREATE TABLE rsql_fk_reader.child (id int PRIMARY KEY,
                 parent_id int REFERENCES rsql_fk_reader.parent(id));
             DO $$ BEGIN CREATE ROLE rsql_fk_reader NOLOGIN;
             EXCEPTION WHEN duplicate_object THEN NULL; END $$;
             GRANT USAGE ON SCHEMA rsql_fk_reader TO rsql_fk_reader;
             GRANT SELECT ON ALL TABLES IN SCHEMA rsql_fk_reader TO rsql_fk_reader;
             SET ROLE rsql_fk_reader;",
        )
        .await
        .unwrap();

    let found = foreign_keys(&client, "rsql_fk_reader").await;
    client.batch_execute("RESET ROLE").await.unwrap();

    assert_eq!(
        found,
        vec![fk("child", "parent_id", "rsql_fk_reader", "parent", "id")]
    );
}

#[tokio::test]
#[ignore = "requires a PostgreSQL server"]
async fn partitions_add_no_copies_but_keep_inherited_keys() {
    let client = connect().await;
    client
        .batch_execute(
            "DROP SCHEMA IF EXISTS rsql_fk_parts CASCADE;
             CREATE SCHEMA rsql_fk_parts;
             CREATE TABLE rsql_fk_parts.events (id int, at date, PRIMARY KEY (id, at))
                 PARTITION BY RANGE (at);
             CREATE TABLE rsql_fk_parts.events_2026 PARTITION OF rsql_fk_parts.events
                 FOR VALUES FROM ('2026-01-01') TO ('2027-01-01');
             CREATE TABLE rsql_fk_parts.event_refs (id int PRIMARY KEY, event_id int, event_at date,
                 FOREIGN KEY (event_id, event_at) REFERENCES rsql_fk_parts.events(id, at));
             CREATE TABLE rsql_fk_parts.logs (id int, event_id int, at date,
                 FOREIGN KEY (event_id, at) REFERENCES rsql_fk_parts.events(id, at))
                 PARTITION BY RANGE (at);
             CREATE TABLE rsql_fk_parts.logs_2026 PARTITION OF rsql_fk_parts.logs
                 FOR VALUES FROM ('2026-01-01') TO ('2027-01-01');",
        )
        .await
        .unwrap();

    assert_eq!(
        foreign_keys(&client, "rsql_fk_parts").await,
        vec![
            fk("event_refs", "event_at", "rsql_fk_parts", "events", "at"),
            fk("event_refs", "event_id", "rsql_fk_parts", "events", "id"),
            fk("logs", "at", "rsql_fk_parts", "events", "at"),
            fk("logs", "event_id", "rsql_fk_parts", "events", "id"),
            fk("logs_2026", "at", "rsql_fk_parts", "events", "at"),
            fk("logs_2026", "event_id", "rsql_fk_parts", "events", "id"),
        ]
    );
}

#[tokio::test]
#[ignore = "requires a PostgreSQL server"]
async fn a_single_column_key_comes_before_a_composite_one() {
    let client = connect().await;
    client
        .batch_execute(
            "DROP SCHEMA IF EXISTS rsql_fk_order CASCADE;
             CREATE SCHEMA rsql_fk_order;
             CREATE TABLE rsql_fk_order.orders (id int PRIMARY KEY);
             CREATE TABLE rsql_fk_order.order_lines (
                 order_id int, line_no int, PRIMARY KEY (order_id, line_no));
             CREATE TABLE rsql_fk_order.shipments (id int PRIMARY KEY, ord int, line int,
                 CONSTRAINT a_composite FOREIGN KEY (ord, line)
                 REFERENCES rsql_fk_order.order_lines(order_id, line_no),
                 CONSTRAINT z_single FOREIGN KEY (ord) REFERENCES rsql_fk_order.orders(id));",
        )
        .await
        .unwrap();

    assert_eq!(
        foreign_keys(&client, "rsql_fk_order").await,
        vec![
            fk(
                "shipments",
                "line",
                "rsql_fk_order",
                "order_lines",
                "line_no"
            ),
            fk("shipments", "ord", "rsql_fk_order", "orders", "id"),
            fk(
                "shipments",
                "ord",
                "rsql_fk_order",
                "order_lines",
                "order_id"
            ),
        ]
    );
}
