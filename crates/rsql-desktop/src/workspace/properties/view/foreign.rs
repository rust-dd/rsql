use super::super::editor::Editor;
use super::structure::{line, stack};
use super::*;
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use rsql_desktop::structure::{EditId, ForeignKey, ForeignTarget, NewObject, StructureDraft};

pub(super) fn foreign(d: &mut StructureDraft, id: EditId) -> Option<&mut ForeignKey> {
    if let Some(NewObject::ForeignKey(fk)) = d.new_object_mut(id) {
        Some(fk)
    } else {
        None
    }
}
impl Panel {
    fn target_menu(
        &self,
        id: EditId,
        schema: Option<String>,
        label: &str,
        cx: &App,
    ) -> impl IntoElement {
        let modal = self.id;
        let view = self.owner.clone();
        let kind = if schema.is_some() { "table" } else { "schema" };
        compact_button(Button::new(format!("choose-{kind}-{id:?}")))
            .label(label.to_owned())
            .w_full()
            .outline()
            .rounded(px(6.))
            .bg(cx.theme().background)
            .border_color(cx.theme().border.opacity(0.3))
            .dropdown_menu(move |menu, _, cx| {
                let mut menu = menu.scrollable(true).max_h(px(220.));
                let Some(workspace) = view.upgrade() else {
                    return menu;
                };
                let v = workspace.read(cx);
                let Some(p) = v
                    .properties
                    .as_ref()
                    .filter(|p| p.id == modal && !p.busy && p.capture.current(v))
                else {
                    return menu;
                };
                if let Some(schema) = &schema {
                    if let Ok(tables) = v.fk_tables(&p.object.project, schema) {
                        for table in tables {
                            let table = table.clone();
                            let view = view.clone();
                            menu = menu.item(PopupMenuItem::new(table.name.clone()).on_click(
                                move |_, _, cx| {
                                    let _ = view.update(cx, |v, cx| {
                                        if v.property_current(modal)
                                            && v.properties.as_ref().is_some_and(|p| !p.busy)
                                        {
                                            v.select_fk_relation(id, table.clone(), cx);
                                        }
                                    });
                                },
                            ));
                        }
                    }
                } else if let Ok(schemas) = v.fk_schemas(&p.object.project) {
                    for schema in schemas {
                        let schema = schema.clone();
                        let view = view.clone();
                        menu = menu.item(PopupMenuItem::new(schema.clone()).on_click(
                            move |_, _, cx| {
                                let _ = view.update(cx, |v, cx| {
                                    if !v.property_current(modal)
                                        || v.properties.as_ref().is_none_or(|p| p.busy)
                                    {
                                        return;
                                    }
                                    let project =
                                        v.properties.as_ref().unwrap().object.project.clone();
                                    v.set_fk_target(
                                        id,
                                        ForeignTarget::Relation {
                                            schema: schema.clone(),
                                            table: String::new(),
                                            columns: vec![],
                                        },
                                        cx,
                                    );
                                    v.load_fk_tables(&project, &schema, cx);
                                });
                            },
                        ));
                    }
                }
                menu
            })
    }
    /// Source layout: TARGET SCHEMA and TARGET TABLE side by side above the column mapping.
    pub(super) fn foreign(&self, e: &Editor, id: EditId, fk: &ForeignKey, cx: &App) -> Div {
        let view = self.owner.upgrade().unwrap();
        let v = view.read(cx);
        let project = &e.draft.snapshot().object().project;
        let caption = |label: &'static str| {
            div()
                .text_size(px(9.))
                .line_height(px(13.5))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(cx.theme().muted_foreground.opacity(0.6))
                .child(label)
        };
        let schema_control = match v.fk_schemas(project) {
            Ok(_) => {
                let label = match &fk.target {
                    ForeignTarget::ThisTable(_) => "Choose schema",
                    ForeignTarget::Relation { schema, .. } => schema,
                };
                self.target_menu(id, None, label, cx).into_any_element()
            }
            Err(msg) => line()
                .child(msg.to_owned())
                .child(
                    self.button(format!("schemas-{id:?}"), |v, _, cx| {
                        let project = v.properties.as_ref().unwrap().object.project.clone();
                        v.load_fk_schemas(&project, cx);
                    })
                    .label("Load schemas"),
                )
                .into_any_element(),
        };
        let table_control = match &fk.target {
            ForeignTarget::ThisTable(_) => div()
                .h(px(28.))
                .px(px(8.))
                .flex()
                .items_center()
                .rounded(px(6.))
                .border_1()
                .border_color(cx.theme().border.opacity(0.3))
                .bg(cx.theme().background)
                .child("This table")
                .into_any_element(),
            ForeignTarget::Relation { schema, table, .. } => match v.fk_tables(project, schema) {
                Ok(_) => {
                    let label = if table.is_empty() {
                        "Select table..."
                    } else {
                        table
                    };
                    self.target_menu(id, Some(schema.clone()), label, cx)
                        .into_any_element()
                }
                Err(msg) => {
                    let schema = schema.clone();
                    line()
                        .child(msg.to_owned())
                        .child(
                            self.button(format!("load-tables-{id:?}"), move |v, _, cx| {
                                let project = v.properties.as_ref().unwrap().object.project.clone();
                                v.load_fk_tables(&project, &schema, cx);
                            })
                            .label("Load tables"),
                        )
                        .into_any_element()
                }
            },
        };
        let muted = cx.theme().muted_foreground;
        let foreground = cx.theme().foreground;
        let this_table = self
            .press(format!("self-{id:?}"), move |v, _, cx| {
                v.set_fk_target(id, ForeignTarget::ThisTable(vec![]), cx)
            })
            .accessibility_label("This table")
            .text_size(px(9.))
            .line_height(px(13.5))
            .text_color(muted)
            .hover(move |s| s.text_color(foreground))
            .child("This table");
        stack()
            .gap(px(10.))
            .child(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(
                        stack()
                            .flex_1()
                            .min_w_0()
                            .gap(px(4.))
                            .child(caption("TARGET SCHEMA"))
                            .child(schema_control),
                    )
                    .child(
                        stack()
                            .flex_1()
                            .min_w_0()
                            .gap(px(4.))
                            .child(
                                div()
                                    .flex()
                                    .justify_between()
                                    .child(caption("TARGET TABLE"))
                                    .child(this_table),
                            )
                            .child(table_control),
                    ),
            )
            .child(self.foreign_pairs(e, id, fk, cx))
    }
}

impl Workspace {
    fn set_fk_target(&mut self, id: EditId, target: ForeignTarget, cx: &mut Context<Self>) {
        self.edit_draft(cx, |d| {
            if let Some(f) = foreign(d, id) {
                f.target = target;
            }
        });
        if let Some(e) = self.properties.as_mut().and_then(|p| p.editor.as_mut()) {
            e.targets.remove(&id);
        }
    }
    fn select_fk_relation(&mut self, id: EditId, object: ObjectRef, cx: &mut Context<Self>) {
        let p = self.properties.as_ref().unwrap();
        let Some(e) = p.editor.as_ref() else { return };
        let selected = e
            .draft
            .added_objects()
            .iter()
            .find(|(i, _)| *i == id)
            .map(|(_, o)| o);
        if !matches!(selected,Some(NewObject::ForeignKey(ForeignKey{target:ForeignTarget::Relation{schema,..},..})) if schema==&object.schema)
        {
            return;
        }
        self.set_fk_target(
            id,
            ForeignTarget::Relation {
                schema: object.schema.clone(),
                table: object.name.clone(),
                columns: vec![],
            },
            cx,
        );
        let p = self.properties.as_mut().unwrap();
        let modal = p.id;
        let rev = Uuid::new_v4();
        let (cancel, rx) = oneshot::channel();
        p.editor.as_mut().unwrap().targets.insert(
            id,
            LoadState::Loading {
                revision: rev,
                _cancel: cancel,
            },
        );
        let svc = self.session.services.clone();
        let mut generation = p.capture.generation.clone();
        let mut stop = self.session.preflight_stop.subscribe();
        let route = p.capture.route.clone();
        let job = self.session.runtime.spawn(async move {
            if !route.same_generation(&svc.capture_project_ticket(&object.project)?) {
                return Err(rsql_core::Error::QueryFailed("Connection changed".into()));
            }
            svc.run_metadata(
                &object.project,
                async {
                    tokio::select! {_=rx=>(),_=generation.changed()=>(),_=stop.wait_for(|s|*s)=>()}
                },
                async |client| {
                    client
                        .batch_execute("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
                        .await
                        .map_err(rsql_core::error::query_failed)?;
                    let choices =
                        rsql_desktop::structure::ForeignChoices::load(client, &object).await?;
                    client
                        .batch_execute("COMMIT")
                        .await
                        .map_err(rsql_core::error::query_failed)?;
                    Ok(std::sync::Arc::new(choices))
                },
            )
            .await
        });
        cx.spawn(async move|v,cx|{
            let result=job.await.map_err(|e|e.to_string()).and_then(|r|r.map_err(|e|e.to_string()));
            let _=v.update(cx,|v,cx|{
                if !v.property_current(modal){return;}
                let Some(e)=v.properties.as_mut().and_then(|p|p.editor.as_mut()) else{return};
                if matches!(e.targets.get(&id),Some(LoadState::Loading{revision,..}) if *revision==rev) {
                    e.targets.insert(id,match result{Ok(cols)=>LoadState::Ready(cols),Err(err)=>LoadState::Failed(err)});cx.notify();
                }
            });
        }).detach();
        cx.notify();
    }
}
