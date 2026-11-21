use super::*;

pub(super) async fn fetch(
    svc: &ApplicationServices,
    id: &str,
    node: &Node,
    cancel: oneshot::Receiver<()>,
) -> Result<Data, rsql_core::Error> {
    svc.run_metadata(
        id,
        async {
            let _ = cancel.await;
        },
        async |client| {
            Ok(match node {
                Node::Schemas => Data::Names(meta::load_schemas(client, meta::SCHEMAS_SQL).await?),
                Node::Databases => Data::Names(meta::load_databases(client).await?),
                Node::Tablespaces => Data::Details(
                    meta::load_tablespaces(client)
                        .await?
                        .into_iter()
                        .map(|(name, owner, location)| {
                            let detail = if location.is_empty() {
                                owner
                            } else {
                                format!("{owner} {location}")
                            };
                            DetailLeaf::new(name, detail)
                        })
                        .collect(),
                ),
                Node::Schema(s) => Data::Index(Arc::new(load_schema_index(client, s).await?)),
                Node::Children(object, kind) => {
                    if object.project != id {
                        return Err(rsql_core::Error::QueryFailed(
                            "Metadata object belongs to another project".into(),
                        ));
                    }
                    client
                        .batch_execute("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
                        .await
                        .map_err(|e| rsql_core::Error::QueryFailed(e.to_string()))?;
                    let (schema, name) = (&object.schema, &object.name);
                    let actual = meta::load_relation(client, schema, name).await?;
                    if ObjectRef::relation(id, actual) != *object {
                        return Err(rsql_core::Error::QueryFailed(
                            "Object identity changed; refresh the catalog".into(),
                        ));
                    }
                    let data = match kind {
                        Child::Indexes => Data::Indexes(
                            meta::load_indexes_typed(client, schema, name)
                                .await?
                                .into_iter()
                                .map(Into::into)
                                .collect(),
                        ),
                        Child::Columns => Data::Columns(
                            meta::load_visible_columns(client, schema, name)
                                .await?
                                .into_iter()
                                .map(ColumnLeaf::from)
                                .collect(),
                        ),
                        Child::Constraints => Data::Details(
                            meta::load_constraints_typed(client, schema, name)
                                .await?
                                .into_iter()
                                .map(|c| DetailLeaf {
                                    name: c.identity.name,
                                    detail: match c.kind.as_str() {
                                        "c" => "CHECK",
                                        "f" => "FOREIGN KEY",
                                        "p" => "PRIMARY KEY",
                                        "u" => "UNIQUE",
                                        "x" => "EXCLUSION",
                                        kind => kind,
                                    }
                                    .to_owned(),
                                    definition: Some(c.definition.into()),
                                })
                                .collect(),
                        ),
                        Child::Triggers => Data::Details(
                            meta::load_triggers(client, schema, name)
                                .await?
                                .into_iter()
                                .map(|(name, event, timing)| {
                                    DetailLeaf::new(name, format!("{timing} {event}"))
                                })
                                .collect(),
                        ),
                        Child::Rules => Data::Details(
                            meta::load_rules(client, schema, name)
                                .await?
                                .into_iter()
                                .map(|(name, event)| {
                                    let detail = match event.as_str() {
                                        "1" => "SELECT",
                                        "2" => "UPDATE",
                                        "3" => "INSERT",
                                        "4" => "DELETE",
                                        event => event,
                                    };
                                    DetailLeaf::new(name, detail.to_owned())
                                })
                                .collect(),
                        ),
                        Child::Policies => Data::Details(
                            meta::load_policies(client, schema, name)
                                .await?
                                .into_iter()
                                .map(|(name, mode, command)| {
                                    DetailLeaf::new(name, format!("{mode} {command}"))
                                })
                                .collect(),
                        ),
                    };
                    client
                        .batch_execute("COMMIT")
                        .await
                        .map_err(|e| rsql_core::Error::QueryFailed(e.to_string()))?;
                    data
                }
                Node::Section(
                    s,
                    section @ (Section::Tables | Section::Views | Section::MaterializedViews),
                ) => Data::Relations(
                    meta::load_relation_group(
                        client,
                        s,
                        match section {
                            Section::Tables => &["r", "p", "f"],
                            Section::Views => &["v"],
                            Section::MaterializedViews => &["m"],
                            _ => unreachable!(),
                        },
                        *section != Section::Views,
                    )
                    .await?
                    .into_iter()
                    .map(|r| RelationEntry {
                        object: ObjectRef::relation(id, r.identity),
                        size: r.size,
                    })
                    .collect(),
                ),
                Node::Section(s, section @ (Section::Functions | Section::TriggerFunctions)) => {
                    Data::Routines(
                        objects::load_routines(client, s)
                            .await?
                            .into_iter()
                            .filter(|r| r.is_trigger == (*section == Section::TriggerFunctions))
                            .map(|r| RoutineEntry {
                                object: ObjectRef::routine(id, r.identity),
                                arguments: r.arguments,
                                return_type: r.return_type,
                            })
                            .collect(),
                    )
                }
                Node::Relation(..) => unreachable!("display-only folder"),
            })
        },
    )
    .await
}
