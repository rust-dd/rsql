use crate::error::{Error, query_failed};

#[derive(Debug, Clone, serde::Serialize)]
pub struct PgRole {
    pub name: String,
    pub superuser: bool,
    pub create_db: bool,
    pub create_role: bool,
    pub login: bool,
    pub replication: bool,
    pub bypass_rls: bool,
    pub conn_limit: i32,
    pub valid_until: String,
    pub member_of: Vec<String>,
}

pub async fn load_roles(client: &tokio_postgres::Client) -> Result<Vec<PgRole>, Error> {
    let rows = client
        .query(
            "SELECT r.rolname, r.rolsuper, r.rolcreatedb, r.rolcreaterole,
                    r.rolcanlogin, r.rolreplication, r.rolbypassrls, r.rolconnlimit,
                    COALESCE(r.rolvaliduntil::text, ''),
                    COALESCE(array_agg(m.rolname) FILTER (WHERE m.rolname IS NOT NULL), '{}')::text[]
             FROM pg_roles r
             LEFT JOIN pg_auth_members am ON am.member = r.oid
             LEFT JOIN pg_roles m ON m.oid = am.roleid
             GROUP BY r.oid, r.rolname, r.rolsuper, r.rolcreatedb, r.rolcreaterole,
                      r.rolcanlogin, r.rolreplication, r.rolbypassrls, r.rolconnlimit, r.rolvaliduntil
             ORDER BY r.rolname",
            &[],
        )
        .await
        .map_err(query_failed)?;

    let mut roles = Vec::new();
    for row in rows {
        roles.push(PgRole {
            name: row.get::<_, String>(0),
            superuser: row.get::<_, bool>(1),
            create_db: row.get::<_, bool>(2),
            create_role: row.get::<_, bool>(3),
            login: row.get::<_, bool>(4),
            replication: row.get::<_, bool>(5),
            bypass_rls: row.get::<_, bool>(6),
            conn_limit: row.get::<_, i32>(7),
            valid_until: row.get::<_, String>(8),
            member_of: row.get::<_, Vec<String>>(9),
        });
    }

    Ok(roles)
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct TableGrant {
    pub schema: String,
    pub table: String,
    pub grantee: String,
    pub privileges: Vec<String>,
}

pub async fn load_table_grants(
    client: &tokio_postgres::Client,
    role_name: &str,
) -> Result<Vec<TableGrant>, Error> {
    let rows = client
        .query(
            "SELECT table_schema, table_name, grantee,
                    array_agg(privilege_type ORDER BY privilege_type)::text[]
             FROM information_schema.table_privileges
             WHERE grantee = $1
             GROUP BY table_schema, table_name, grantee
             ORDER BY table_schema, table_name",
            &[&role_name],
        )
        .await
        .map_err(query_failed)?;

    let mut grants = Vec::new();
    for row in rows {
        grants.push(TableGrant {
            schema: row.get(0),
            table: row.get(1),
            grantee: row.get(2),
            privileges: row.get::<_, Vec<String>>(3),
        });
    }

    Ok(grants)
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DbGrant {
    pub database: String,
    pub privilege: String,
}

pub async fn load_database_grants(
    client: &tokio_postgres::Client,
    role_name: &str,
) -> Result<Vec<DbGrant>, Error> {
    let rows = client
        .query(
            "SELECT datname, privilege_type
             FROM pg_database
             CROSS JOIN LATERAL (
               SELECT privilege_type
               FROM (VALUES ('CONNECT'), ('CREATE'), ('TEMPORARY')) AS privs(privilege_type)
               WHERE has_database_privilege($1, datname, privilege_type)
             ) t
             WHERE NOT datistemplate
             ORDER BY datname, privilege_type",
            &[&role_name],
        )
        .await
        .map_err(query_failed)?;

    let mut grants = Vec::new();
    for row in rows {
        grants.push(DbGrant {
            database: row.get(0),
            privilege: row.get(1),
        });
    }

    Ok(grants)
}
