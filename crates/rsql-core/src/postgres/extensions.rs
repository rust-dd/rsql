use crate::error::{Error, query_failed};

pub async fn load_extensions(client: &tokio_postgres::Client) -> Result<Vec<Vec<String>>, Error> {
    let rows = client
        .query(
            "SELECT
            e.extname AS name,
            e.extversion AS installed_version,
            COALESCE(a.default_version, '') AS default_version,
            COALESCE(a.comment, '') AS comment,
            n.nspname AS schema
         FROM pg_extension e
         JOIN pg_namespace n ON n.oid = e.extnamespace
         LEFT JOIN pg_available_extensions a ON a.name = e.extname
         ORDER BY e.extname",
            &[],
        )
        .await
        .map_err(query_failed)?;

    Ok(rows
        .iter()
        .map(|r| (0..5).map(|i| r.get::<_, String>(i)).collect())
        .collect())
}

pub async fn load_available_extensions(
    client: &tokio_postgres::Client,
) -> Result<Vec<Vec<String>>, Error> {
    let rows = client
        .query(
            "SELECT
            a.name,
            COALESCE(a.default_version, '') AS version,
            COALESCE(a.comment, '') AS comment
         FROM pg_available_extensions a
         LEFT JOIN pg_extension e ON e.extname = a.name
         WHERE e.oid IS NULL
         ORDER BY a.name",
            &[],
        )
        .await
        .map_err(query_failed)?;

    Ok(rows
        .iter()
        .map(|r| (0..3).map(|i| r.get::<_, String>(i)).collect())
        .collect())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnumType {
    pub schema: String,
    pub name: String,
    pub labels: Vec<String>,
}

pub async fn load_enum_type_details(
    client: &tokio_postgres::Client,
) -> Result<Vec<EnumType>, Error> {
    let rows = client
        .query(
            "SELECT n.nspname, t.typname, ARRAY(
             SELECT e.enumlabel::text FROM pg_catalog.pg_enum e
             WHERE e.enumtypid = t.oid ORDER BY e.enumsortorder
         )
         FROM pg_catalog.pg_type t
         JOIN pg_catalog.pg_namespace n ON n.oid = t.typnamespace
         WHERE t.typtype = 'e' AND n.nspname NOT IN ('pg_catalog', 'information_schema')
         ORDER BY n.nspname, t.typname",
            &[],
        )
        .await
        .map_err(query_failed)?;
    Ok(rows
        .into_iter()
        .map(|row| EnumType {
            schema: row.get(0),
            name: row.get(1),
            labels: row.get(2),
        })
        .collect())
}

pub async fn load_enum_types(client: &tokio_postgres::Client) -> Result<Vec<Vec<String>>, Error> {
    Ok(load_enum_type_details(client)
        .await?
        .into_iter()
        // The legacy inner join omitted enums declared with zero labels.
        .filter(|e| !e.labels.is_empty())
        .map(|e| vec![e.schema, e.name, e.labels.join(", ")])
        .collect())
}

pub async fn load_pg_settings(client: &tokio_postgres::Client) -> Result<Vec<Vec<String>>, Error> {
    let rows = client
        .query(
            "SELECT
            name,
            COALESCE(setting, '') AS setting,
            COALESCE(unit, '') AS unit,
            category,
            COALESCE(short_desc, '') AS description,
            context,
            COALESCE(source, '') AS source,
            COALESCE(boot_val, '') AS boot_val,
            COALESCE(reset_val, '') AS reset_val
         FROM pg_settings
         ORDER BY category, name",
            &[],
        )
        .await
        .map_err(query_failed)?;

    Ok(rows
        .iter()
        .map(|r| (0..9).map(|i| r.get::<_, String>(i)).collect())
        .collect())
}
