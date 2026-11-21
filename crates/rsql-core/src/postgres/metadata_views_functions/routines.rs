use crate::error::{Error, query_failed};
use crate::postgres::ObjectStats;
use tokio_postgres::{Client, Row};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RoutineKind {
    Function,
    Procedure,
    Window,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RoutineIdentity {
    pub oid: u32,
    pub schema: String,
    pub name: String,
    pub kind: RoutineKind,
    pub argument_types: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutineMetadata {
    pub identity: RoutineIdentity,
    /// Display text rendered in the reading connection; never an identity key.
    pub identity_arguments: String,
    pub arguments: String,
    pub return_type: Option<String>,
    pub is_trigger: bool,
    /// Fully qualified, server-quoted routine and input type names for a future action draft.
    pub action_identity: String,
}

const ROUTINES: &str = "SELECT p.oid, n.nspname::pg_catalog.text, p.proname::pg_catalog.text, p.prokind::pg_catalog.text,
    pg_catalog.pg_get_function_identity_arguments(p.oid), p.proargtypes::pg_catalog.oid[],
    pg_catalog.pg_get_function_arguments(p.oid), pg_catalog.pg_get_function_result(p.oid),
    p.prorettype = 'pg_catalog.trigger'::pg_catalog.regtype,
    pg_catalog.format('%I.%I(%s)', n.nspname, p.proname,
        COALESCE((SELECT pg_catalog.string_agg(pg_catalog.format('%I.%I', tn.nspname, t.typname), ', ' ORDER BY argument.pos)
                  FROM pg_catalog.unnest(p.proargtypes) WITH ORDINALITY argument(type_oid, pos)
                  JOIN pg_catalog.pg_type t ON t.oid = argument.type_oid
                  JOIN pg_catalog.pg_namespace tn ON tn.oid = t.typnamespace), ''))
    FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid = p.pronamespace";

fn routine(row: &Row) -> RoutineMetadata {
    RoutineMetadata {
        identity: RoutineIdentity {
            oid: row.get(0),
            schema: row.get(1),
            name: row.get(2),
            kind: match row.get::<_, &str>(3) {
                "p" => RoutineKind::Procedure,
                "w" => RoutineKind::Window,
                _ => RoutineKind::Function,
            },
            argument_types: row.get(5),
        },
        identity_arguments: row.get(4),
        arguments: row.get(6),
        return_type: row.get(7),
        is_trigger: row.get(8),
        action_identity: row.get(9),
    }
}

/// Lists only routines for which PostgreSQL can produce a CREATE definition.
pub async fn load_routines(client: &Client, schema: &str) -> Result<Vec<RoutineMetadata>, Error> {
    let sql = format!(
        "{ROUTINES} WHERE n.nspname = $1 AND p.prokind IN ('f','p','w') ORDER BY p.proname, p.oid"
    );
    Ok(client
        .query(&sql, &[&schema])
        .await
        .map_err(query_failed)?
        .iter()
        .map(routine)
        .collect())
}

/// Revalidates server identity without using connection-dependent display arguments.
pub async fn resolve_routine(
    client: &Client,
    expected: &RoutineIdentity,
) -> Result<RoutineMetadata, Error> {
    let sql = format!("{ROUTINES} WHERE p.oid = $1 AND p.prokind IN ('f','p','w')");
    let item = client
        .query_opt(&sql, &[&expected.oid])
        .await
        .map_err(query_failed)?
        .map(|row| routine(&row));
    match item {
        Some(item) if item.identity == *expected => Ok(item),
        _ => Err(Error::QueryFailed(
            "Routine identity changed; refresh the catalog".into(),
        )),
    }
}

pub(crate) async fn lookup(
    client: &Client,
    schema: &str,
    name: &str,
) -> Result<Option<RoutineMetadata>, Error> {
    let sql = format!(
        "{ROUTINES} WHERE n.nspname = $1 AND p.proname = $2
        AND p.prokind IN ('f','p','w')"
    );
    let rows = client
        .query(&sql, &[&schema, &name])
        .await
        .map_err(query_failed)?;
    if rows.len() > 1 {
        return Err(Error::QueryFailed(format!(
            "Routine name is ambiguous: {schema}.{name}; select a specific overload"
        )));
    }
    Ok(rows.first().map(routine))
}

pub async fn load_routine_info(
    client: &Client,
    expected: &RoutineIdentity,
) -> Result<ObjectStats, Error> {
    let item = resolve_routine(client, expected).await?;
    let oid = item.identity.oid;
    let mut fields = info(client, oid).await?;
    let is_sql = fields
        .iter()
        .any(|(key, value)| key == "language" && value == "sql");
    if is_sql
        && let Some((_, source)) = fields
            .iter_mut()
            .find(|(key, value)| key == "source" && value.is_empty())
    {
        let deparser_available = client
            .query_one("SELECT pg_catalog.to_regprocedure('pg_catalog.pg_get_function_sqlbody(pg_catalog.oid)') IS NOT NULL", &[])
            .await.map_err(query_failed)?.get::<_, bool>(0);
        if deparser_available {
            *source = client
                .query_one("SELECT pg_catalog.pg_get_function_sqlbody($1)", &[&oid])
                .await
                .map_err(query_failed)?
                .get::<_, Option<String>>(0)
                .unwrap_or_default();
        }
    }
    Ok(fields)
}

pub(crate) async fn info(client: &Client, oid: u32) -> Result<ObjectStats, Error> {
    let row = client.query_opt(
        "SELECT l.lanname::text,
            CASE p.provolatile WHEN 'i' THEN 'IMMUTABLE' WHEN 's' THEN 'STABLE' WHEN 'v' THEN 'VOLATILE' ELSE '' END,
            p.proisstrict::text, p.prosecdef::text, p.procost::text, p.prorows::text,
            COALESCE(pg_catalog.pg_get_function_result(p.oid), ''), pg_catalog.pg_get_function_arguments(p.oid), p.prosrc
         FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_language l ON l.oid = p.prolang
         WHERE p.oid = $1 AND p.prokind IN ('f','p','w')", &[&oid]
    ).await.map_err(query_failed)?;
    let keys = [
        "language",
        "volatility",
        "is_strict",
        "security_definer",
        "estimated_cost",
        "estimated_rows",
        "return_type",
        "arguments",
        "source",
    ];
    Ok(row
        .map(|r| {
            keys.into_iter()
                .enumerate()
                .map(|(i, k)| (k.into(), r.get(i)))
                .collect()
        })
        .unwrap_or_default())
}

pub async fn generate_routine_ddl(
    client: &Client,
    expected: &RoutineIdentity,
) -> Result<String, Error> {
    let item = resolve_routine(client, expected).await?;
    definition(client, item.identity.oid).await
}

pub(crate) async fn definition(client: &Client, oid: u32) -> Result<String, Error> {
    Ok(client.query_opt("SELECT pg_catalog.pg_get_functiondef(oid) FROM pg_catalog.pg_proc WHERE oid = $1 AND prokind IN ('f','p','w')", &[&oid])
        .await.map_err(query_failed)?.map(|r| r.get(0)).unwrap_or_default())
}

pub(crate) async fn legacy_definition(
    client: &Client,
    schema: &str,
    name: &str,
) -> Result<String, Error> {
    match lookup(client, schema, name).await? {
        Some(routine) => definition(client, routine.identity.oid).await,
        None => Ok(String::new()),
    }
}
