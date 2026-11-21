use crate::postgres::metadata_schema::{RelationKind, load_relation};
use crate::{
    error::{Error, query_failed},
    mutation::quote_ident,
};
use tokio_postgres::Client;

pub(super) async fn generate(
    client: &Client,
    schema: &str,
    name: &str,
    materialized: bool,
) -> Result<String, Error> {
    let relation = load_relation(client, schema, name).await?;
    let expected = if materialized {
        RelationKind::MaterializedView
    } else {
        RelationKind::View
    };
    if relation.kind != expected {
        return Err(Error::QueryFailed(
            "Object is not the requested view kind".into(),
        ));
    }
    let oid = relation.oid;
    let row = client.query_one(
        "SELECT pg_catalog.pg_get_viewdef(c.oid, false), c.relispopulated,
                COALESCE((SELECT string_agg(
                    (SELECT string_agg(quote_ident(part), '.') FROM unnest(string_to_array(o.option_name, '.')) part)
                    || ' = ' || quote_literal(o.option_value), ', ' ORDER BY o.option_name)
                    FROM pg_catalog.pg_options_to_table(c.reloptions) o), '')
         FROM pg_catalog.pg_class c WHERE c.oid = $1", &[&oid]
    ).await.map_err(query_failed)?;
    let definition: String = row.get(0);
    let kind = if materialized {
        "MATERIALIZED VIEW"
    } else {
        "VIEW"
    };
    let options: String = row.get(2);
    let options = if options.is_empty() {
        String::new()
    } else {
        format!(" WITH ({options})")
    };
    let data = if materialized && !row.get::<_, bool>(1) {
        " WITH NO DATA"
    } else {
        ""
    };
    let replace = if materialized { "" } else { "OR REPLACE " };
    let mut ddl = vec![format!(
        "CREATE {replace}{kind} {}.{}{options} AS\n{}{data};",
        quote_ident(schema),
        quote_ident(name),
        definition.trim_end().trim_end_matches(';')
    )];
    if materialized {
        ddl.extend(super::table::indexes(client, oid).await?);
    }
    ddl.extend(super::table::comments(client, oid, schema, name, kind).await?);
    Ok(ddl.join("\n\n"))
}
