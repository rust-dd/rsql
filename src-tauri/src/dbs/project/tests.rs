use super::{select_projects, set_server_alias, upsert_project};
use crate::dbs::schema::create_local_schema;
use libsql::{Connection, Database};

async fn local_db() -> (Database, Connection) {
    let db = libsql::Builder::new_local(":memory:")
        .build()
        .await
        .unwrap();
    let conn = db.connect().unwrap();
    create_local_schema(&conn).await.unwrap();
    (db, conn)
}

fn details(database: &str) -> Vec<String> {
    [
        "PGSQL", "app", "secret", database, "db.local", "5432", "true", "false", "", "22", "", "",
        "",
    ]
    .map(String::from)
    .to_vec()
}

#[tokio::test]
async fn upserted_project_round_trips_its_details() {
    let (_db, conn) = local_db().await;

    upsert_project(&conn, "k1", &details("orders"))
        .await
        .unwrap();

    let projects = select_projects(&conn).await.unwrap();
    assert_eq!(projects["k1"][..13], details("orders")[..]);
}

fn named(database: &str, name: &str, server_alias: &str) -> Vec<String> {
    let mut saved = details(database);
    saved.push(name.to_string());
    saved.push(server_alias.to_string());
    saved
}

async fn server_alias_of(conn: &Connection, id: &str) -> String {
    select_projects(conn).await.unwrap()[id][14].clone()
}

#[tokio::test]
async fn name_and_server_alias_follow_the_details_and_legacy_id_comes_last() {
    let (_db, conn) = local_db().await;

    upsert_project(&conn, "k1", &named("orders", "Orders (prod)", "Railway"))
        .await
        .unwrap();

    let project = &select_projects(&conn).await.unwrap()["k1"];
    assert_eq!(project.len(), 16);
    assert_eq!(project[13], "Orders (prod)");
    assert_eq!(project[14], "Railway");
    assert_eq!(project[15], "");
}

#[tokio::test]
async fn saving_an_existing_project_keeps_its_legacy_id() {
    let (_db, conn) = local_db().await;
    conn.execute(
        "INSERT INTO projects (id, name, legacy_id) VALUES ('k1', 'prod', 'prod')",
        (),
    )
    .await
    .unwrap();
    upsert_project(&conn, "k1", &named("orders", "renamed", ""))
        .await
        .unwrap();

    let project = &select_projects(&conn).await.unwrap()["k1"];
    assert_eq!(project[13], "renamed");
    assert_eq!(project[15], "prod");
}

#[tokio::test]
async fn editing_a_project_keeps_its_server_alias() {
    let (_db, conn) = local_db().await;
    upsert_project(&conn, "k1", &named("orders", "prod", "Railway"))
        .await
        .unwrap();

    upsert_project(&conn, "k1", &named("orders", "renamed", ""))
        .await
        .unwrap();

    assert_eq!(server_alias_of(&conn, "k1").await, "Railway");
}

#[tokio::test]
async fn server_alias_is_set_on_the_listed_projects_only() {
    let (_db, conn) = local_db().await;
    for id in ["k1", "k2", "k3"] {
        upsert_project(&conn, id, &named("orders", id, ""))
            .await
            .unwrap();
    }

    set_server_alias(&conn, &["k1".to_string(), "k2".to_string()], "Railway")
        .await
        .unwrap();

    assert_eq!(server_alias_of(&conn, "k1").await, "Railway");
    assert_eq!(server_alias_of(&conn, "k2").await, "Railway");
    assert_eq!(server_alias_of(&conn, "k3").await, "");
}
