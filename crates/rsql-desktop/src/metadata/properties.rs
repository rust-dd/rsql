mod details;
mod structure;
mod text;
use super::{ObjectKind, ObjectRef, changed};
pub use details::{DetailKind, DetailRow};
use rsql_core::{
    ApplicationServices, Error,
    postgres::{
        ddl_generation, metadata_schema as relations, metadata_views_functions as routines,
        statistics_activity as stats,
    },
};
use std::future::Future;
pub use structure::{ColumnRow, IndexRow, NO_DEFAULT, policy_line, rule_line, trigger_line};
pub use text::PropertyText;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Section {
    Overview,
    Constraints,
    Triggers,
    Rules,
    Policies,
    Columns,
    Indexes,
    Structure,
    Incoming,
    Outgoing,
    Ddl,
}

pub struct Related {
    pub name: String,
    pub source: String,
    pub target: String,
    pub pairs: Vec<String>,
    pub actions: String,
    pub related: ObjectRef,
}

pub enum Content {
    Overview(Vec<(String, PropertyText)>),
    Structure(Option<std::sync::Arc<crate::structure::StructureDraft>>),
    Columns(Vec<ColumnRow>),
    Indexes(Vec<IndexRow>),
    Details(Vec<DetailRow>),
    Keys(Vec<Related>),
    Ddl(PropertyText),
}

pub fn qualified(schema: &str, name: &str) -> String {
    use rsql_core::mutation::quote_ident;
    format!("{}.{}", quote_ident(schema), quote_ident(name))
}

pub async fn read(
    services: &ApplicationServices,
    object: &ObjectRef,
    section: Section,
    route: &rsql_core::services::ProjectTicket,
    cancelled: impl Future<Output = ()>,
) -> Result<Content, Error> {
    services
        .run_metadata(&object.project, cancelled, async |client| {
            read_on(client, services, object, section, route).await
        })
        .await
}

pub async fn read_on(
    client: &rsql_core::postgres::Client,
    services: &ApplicationServices,
    object: &ObjectRef,
    section: Section,
    route: &rsql_core::services::ProjectTicket,
) -> Result<Content, Error> {
    let (schema, name) = (&object.schema, &object.name);
    if !route.is_current()
        || !route.same_generation(&services.capture_project_ticket(&object.project)?)
    {
        return Err(Error::QueryFailed("Connection changed".into()));
    }
    client
        .batch_execute("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .await
        .map_err(|e| Error::QueryFailed(e.to_string()))?;
    let actual = if let Some(identity) = &object.routine_identity {
        ObjectRef::routine(
            &object.project,
            routines::resolve_routine(client, identity).await?.identity,
        )
    } else {
        ObjectRef::relation(
            &object.project,
            relations::load_relation(client, schema, name).await?,
        )
    };
    if *object != actual {
        return Err(changed());
    }
    let data = section_data(client, object, section).await?;
    client
        .batch_execute("COMMIT")
        .await
        .map_err(|e| Error::QueryFailed(e.to_string()))?;
    Ok(data)
}

/// Reads sections for an opened dialog over one connection. The concurrent queries pipeline,
/// and each section stays independent so one permission error doesn't fail the rest.
pub async fn read_many(
    client: &rsql_core::postgres::Client,
    services: &ApplicationServices,
    object: &ObjectRef,
    sections: &[Section],
    route: &rsql_core::services::ProjectTicket,
) -> Result<Vec<Result<Content, Error>>, Error> {
    if !route.is_current()
        || !route.same_generation(&services.capture_project_ticket(&object.project)?)
    {
        return Err(Error::QueryFailed("Connection changed".into()));
    }
    let actual = if let Some(identity) = &object.routine_identity {
        ObjectRef::routine(
            &object.project,
            routines::resolve_routine(client, identity).await?.identity,
        )
    } else {
        ObjectRef::relation(
            &object.project,
            relations::load_relation(client, &object.schema, &object.name).await?,
        )
    };
    if *object != actual {
        return Err(changed());
    }
    Ok(futures_util::future::join_all(
        sections
            .iter()
            .map(|&section| section_data(client, object, section)),
    )
    .await)
}

async fn section_data(
    client: &rsql_core::postgres::Client,
    object: &ObjectRef,
    section: Section,
) -> Result<Content, Error> {
    let (schema, name) = (&object.schema, &object.name);
    let data = match section {
        Section::Overview => {
            let rows = match object.kind {
                ObjectKind::View => routines::load_view_info(client, schema, name).await?,
                ObjectKind::MaterializedView => {
                    routines::load_matview_info(client, schema, name).await?
                }
                kind if kind.is_table() => stats::load_table_measurements(client, schema, name)
                    .await?
                    .into_iter()
                    .map(|(k, v)| (k, v.unwrap_or_else(|| "Unavailable".into())))
                    .collect(),
                _ => {
                    routines::load_routine_info(
                        client,
                        object.routine_identity.as_ref().ok_or_else(changed)?,
                    )
                    .await?
                }
            };
            Content::Overview(
                rows.into_iter()
                    .map(|(k, v)| {
                        let v = if v.is_empty()
                            && !matches!(k.as_str(), "arguments" | "source" | "definition")
                            || (k == "row_estimate" && v == "-1")
                        {
                            "Unavailable".into()
                        } else {
                            v
                        };
                        (k.replace('_', " "), v.into())
                    })
                    .collect(),
            )
        }
        Section::Structure => {
            let draft = if relations::owns_relation(client, object.oid).await? {
                let meta = relations::load_structure(client, schema, name).await?;
                let snapshot = crate::structure::StructureSnapshot::new(&object.project, meta);
                Some(std::sync::Arc::new(snapshot.draft()))
            } else {
                None
            };
            Content::Structure(draft)
        }
        Section::Columns => Content::Columns(
            relations::load_visible_columns(client, schema, name)
                .await?
                .into_iter()
                .map(Into::into)
                .collect(),
        ),
        Section::Indexes => Content::Indexes(
            relations::load_indexes_typed(client, schema, name)
                .await?
                .into_iter()
                .map(Into::into)
                .collect(),
        ),
        Section::Constraints => Content::Details(
            relations::load_constraints_typed(client, schema, name)
                .await?
                .into_iter()
                .map(Into::into)
                .collect(),
        ),
        Section::Triggers => Content::Details(
            relations::load_triggers(client, schema, name)
                .await?
                .into_iter()
                .map(DetailRow::trigger)
                .collect(),
        ),
        Section::Rules => Content::Details(
            relations::load_rules(client, schema, name)
                .await?
                .into_iter()
                .map(DetailRow::rule)
                .collect(),
        ),
        Section::Policies => Content::Details(
            relations::load_policies(client, schema, name)
                .await?
                .into_iter()
                .map(DetailRow::policy)
                .collect(),
        ),
        Section::Incoming | Section::Outgoing => {
            let incoming = section == Section::Incoming;
            let rows = stats::load_related_fk_details(client, schema, name, incoming).await?;
            let mut keys = Vec::new();
            for group in rows.chunk_by(|a, b| {
                let (a, b) = (&a.detail, &b.detail);
                (&a.0, &a.1, &a.2) == (&b.0, &b.1, &b.2)
            }) {
                let (key, ss, st, _, ts, tt, _, update, delete) = &group[0].detail;
                let related = group[0].related.clone();
                keys.push(Related {
                    name: key.clone(),
                    source: qualified(ss, st),
                    target: qualified(ts, tt),
                    pairs: group
                        .iter()
                        .map(|row| {
                            format!(
                                "{} → {}",
                                rsql_core::mutation::quote_ident(&row.detail.3),
                                rsql_core::mutation::quote_ident(&row.detail.6)
                            )
                        })
                        .collect(),
                    actions: format!("ON UPDATE {update} · ON DELETE {delete}"),
                    related: ObjectRef::relation(&object.project, related),
                });
            }
            Content::Keys(keys)
        }
        Section::Ddl => {
            let ddl = if let Some(identity) = &object.routine_identity {
                routines::generate_routine_ddl(client, identity).await?
            } else {
                let kind = match object.kind {
                    ObjectKind::View => "view",
                    ObjectKind::MaterializedView => "matview",
                    _ => "table",
                };
                if object.kind.is_table()
                    && !relations::all_columns_visible(client, object.oid).await?
                {
                    let action = if object.kind == ObjectKind::ForeignTable {
                        "the Columns tab in Properties"
                    } else {
                        "Show CREATE TABLE"
                    };
                    return Err(Error::QueryFailed(format!(
                        "Full table DDL requires visibility of all columns. Use {action} to inspect visible columns."
                    )));
                }
                ddl_generation::generate_full_ddl(client, schema, name, kind).await?
            };
            Content::Ddl(ddl.into())
        }
    };
    Ok(data)
}
