use super::super::editor::{Editor, fk_target_column_count};
use super::{foreign::foreign, header::icon, structure::stack, *};
use gpui_kit::{
    assets::IconName,
    component::{
        Disableable,
        button::ButtonVariants,
        menu::{DropdownMenu, PopupMenuItem},
    },
};
use rsql_desktop::structure::{Action as FkAction, EditId, ForeignKey, ForeignTarget, NewObject};

#[derive(Clone, PartialEq, Eq)]
enum Target {
    Here(EditId),
    Relation {
        schema: String,
        table: String,
        column: String,
    },
}

fn assign<T: PartialEq>(columns: &mut Vec<T>, position: usize, value: T) {
    if columns
        .iter()
        .enumerate()
        .any(|(index, column)| index != position && column == &value)
    {
        return;
    }
    if let Some(existing) = columns.get_mut(position) {
        *existing = value;
    } else if position == columns.len() {
        columns.push(value);
    }
}

fn remove<T>(columns: &mut Vec<T>, position: usize) {
    if position < columns.len() {
        columns.remove(position);
    }
}

fn field_caption(label: &'static str, cx: &App) -> Div {
    div()
        .text_size(px(9.))
        .line_height(px(13.5))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(cx.theme().muted_foreground.opacity(0.6))
        .child(label)
}

fn pair_track(child: impl IntoElement) -> Div {
    div().flex_1().min_w_0().child(child)
}

fn pair_line() -> Div {
    div().flex().items_center().gap(px(6.))
}

impl Panel {
    pub(super) fn foreign_pairs(&self, e: &Editor, key: EditId, fk: &ForeignKey, cx: &App) -> Div {
        let columns: Vec<_> = e
            .draft
            .columns()
            .iter()
            .filter_map(|entry| {
                entry
                    .current()
                    .map(|column| (column.name.clone(), entry.id()))
            })
            .collect();
        let mut pairs = stack()
            .gap(px(6.))
            .child(field_caption("COLUMN MAPPING", cx))
            .child(
                pair_line()
                    .child(pair_track(field_caption("SOURCE", cx)))
                    .child(div().w(px(20.)).flex_shrink_0())
                    .child(pair_track(field_caption("TARGET", cx)))
                    .child(div().w(px(28.)).flex_shrink_0()),
            );
        let sources = fk.key.columns.len();
        let targets = fk_target_column_count(&fk.target);
        let count = sources.max(targets);
        let pending = e.pending_fk_pairs.get(&key).copied();
        let (available, selected) = match &fk.target {
            ForeignTarget::ThisTable(ids) => (
                columns
                    .iter()
                    .map(|(name, id)| (name.clone(), Target::Here(*id)))
                    .collect::<Vec<_>>(),
                ids.iter().copied().map(Target::Here).collect::<Vec<_>>(),
            ),
            ForeignTarget::Relation {
                schema,
                table,
                columns,
            } => {
                let target = |column: &String| Target::Relation {
                    schema: schema.clone(),
                    table: table.clone(),
                    column: column.clone(),
                };
                let available = match e.targets.get(&key) {
                    Some(LoadState::Ready(entry)) => entry
                        .columns
                        .iter()
                        .map(|(name, _)| (name.clone(), target(name)))
                        .collect(),
                    Some(LoadState::Failed(message)) => {
                        pairs =
                            pairs.child(Label::new(message.clone()).text_color(cx.theme().danger));
                        Vec::new()
                    }
                    _ => {
                        if !table.is_empty() {
                            pairs = pairs.child("Loading target columns…");
                        }
                        Vec::new()
                    }
                };
                (available, columns.iter().map(target).collect())
            }
        };
        for position in 0..count.max(pending.map_or(0, |position| position + 1)) {
            let source = self.foreign_select(
                (format!("fk-source-{key:?}-{position}"), "Source column"),
                fk.key.columns.get(position),
                &columns,
                position > sources,
                move |v, cid, cx| {
                    v.edit_foreign_pair(key, position, cx, |fk| {
                        assign(&mut fk.key.columns, position, cid);
                    });
                },
                cx,
            );
            let target = self.foreign_select(
                (format!("fk-target-{key:?}-{position}"), "Target column"),
                selected.get(position),
                &available,
                position > targets,
                move |v, choice, cx| {
                    v.edit_foreign_pair(key, position, cx, |fk| match (&mut fk.target, choice) {
                        (ForeignTarget::ThisTable(ids), Target::Here(id)) => {
                            assign(ids, position, id)
                        }
                        (
                            ForeignTarget::Relation {
                                schema,
                                table,
                                columns,
                            },
                            Target::Relation {
                                schema: expected_schema,
                                table: expected_table,
                                column,
                            },
                        ) if *schema == expected_schema && *table == expected_table => {
                            assign(columns, position, column);
                        }
                        _ => {}
                    });
                },
                cx,
            );
            let remove_pair = self
                .press(
                    format!("fk-remove-pair-{key:?}-{position}"),
                    move |v, _, cx| {
                        v.edit_structure_state(cx, |editor| {
                            let Some(fk) =
                                foreign(std::sync::Arc::make_mut(&mut editor.draft), key)
                            else {
                                return;
                            };
                            remove(&mut fk.key.columns, position);
                            match &mut fk.target {
                                ForeignTarget::ThisTable(ids) => remove(ids, position),
                                ForeignTarget::Relation { columns, .. } => {
                                    remove(columns, position)
                                }
                            }
                            if let std::collections::btree_map::Entry::Occupied(mut pending) =
                                editor.pending_fk_pairs.entry(key)
                            {
                                match position.cmp(pending.get()) {
                                    std::cmp::Ordering::Equal => {
                                        pending.remove();
                                    }
                                    std::cmp::Ordering::Less => *pending.get_mut() -= 1,
                                    std::cmp::Ordering::Greater => {}
                                }
                            }
                        });
                    },
                )
                .accessibility_label("Remove column pair")
                .w(px(28.))
                .h(px(28.))
                .flex_shrink_0()
                .rounded(px(4.))
                .child(icon(IconName::Trash, 12., cx.theme().danger));
            pairs = pairs.child(
                pair_line()
                    .id(format!("fk-pair-{key:?}-{position}"))
                    .test_support()
                    .child(pair_track(source))
                    .child(
                        div()
                            .flex()
                            .justify_center()
                            .w(px(20.))
                            .flex_shrink_0()
                            .child(icon(
                                IconName::ArrowRight,
                                12.,
                                cx.theme().muted_foreground.opacity(0.4),
                            )),
                    )
                    .child(pair_track(target))
                    .child(remove_pair),
            );
        }
        let add = self
            .button(format!("fk-add-pair-{key:?}"), move |v, _, cx| {
                v.edit_structure_state(cx, |editor| {
                    if let Some((_, NewObject::ForeignKey(fk))) = editor
                        .draft
                        .added_objects()
                        .iter()
                        .find(|(id, _)| *id == key)
                        && fk.key.columns.len() == fk_target_column_count(&fk.target)
                    {
                        editor
                            .pending_fk_pairs
                            .entry(key)
                            .or_insert(fk.key.columns.len());
                    }
                });
            })
            .label("Add column pair")
            .icon(IconName::Plus)
            .ghost()
            .disabled(pending.is_some() || sources != targets);
        pairs.child(div().flex().child(add)).child(
            pair_line()
                .gap(px(8.))
                .child(self.foreign_action(key, true, fk.on_update, cx))
                .child(self.foreign_action(key, false, fk.on_delete, cx)),
        )
    }

    fn foreign_action(&self, key: EditId, update: bool, selected: FkAction, cx: &App) -> Div {
        let field = if update { "update" } else { "delete" };
        let caption = if update { "ON UPDATE" } else { "ON DELETE" };
        let options: Vec<_> = FkAction::ALL
            .into_iter()
            .map(|action| (action.sql().to_owned(), action))
            .collect();
        let select = self.foreign_select(
            (format!("fk-{field}-{key:?}"), caption),
            Some(&selected),
            &options,
            false,
            move |v, action, cx| {
                v.edit_draft(cx, |draft| {
                    if let Some(fk) = foreign(draft, key) {
                        if update {
                            fk.on_update = action;
                        } else {
                            fk.on_delete = action;
                        }
                    }
                });
            },
            cx,
        );
        pair_track(stack().child(field_caption(caption, cx)).child(select))
    }

    fn foreign_select<T, F>(
        &self,
        (id, label): (String, &'static str),
        selected: Option<&T>,
        options: &[(String, T)],
        disabled: bool,
        choose: F,
        cx: &App,
    ) -> impl IntoElement + use<T, F>
    where
        T: Clone + PartialEq + 'static,
        F: Fn(&mut Workspace, T, &mut Context<Workspace>) + 'static,
    {
        let current = options
            .iter()
            .find(|(_, value)| Some(value) == selected)
            .map(|(name, _)| name.as_str())
            .unwrap_or(if selected.is_some() {
                "Unavailable column"
            } else {
                "Select…"
            });
        let owner = self.owner.clone();
        let modal = self.id;
        let options = options
            .iter()
            .map(|(name, value)| (name.clone(), value.clone(), Some(value) == selected))
            .collect::<Vec<_>>();
        let choose = std::rc::Rc::new(choose);
        compact_button(Button::new(id))
            .label(current.to_owned())
            .accessibility_label(format!("{label}: {current}"))
            .w_full()
            .outline()
            .rounded(px(6.))
            .bg(cx.theme().background)
            .border_color(cx.theme().border.opacity(0.3))
            .disabled(disabled || options.is_empty())
            .dropdown_menu(move |mut menu, _, _| {
                menu = menu.scrollable(true).max_h(px(220.));
                for (name, value, checked) in &options {
                    let choose = choose.clone();
                    let owner = owner.clone();
                    let value = value.clone();
                    menu = menu.item(PopupMenuItem::new(name.clone()).checked(*checked).on_click(
                        move |_, _, cx| {
                            let _ = owner.update(cx, |v, cx| {
                                if v.property_current(modal)
                                    && v.properties.as_ref().is_some_and(|p| !p.busy)
                                {
                                    choose(v, value.clone(), cx);
                                }
                            });
                        },
                    ));
                }
                menu
            })
    }
}

impl Workspace {
    fn edit_foreign_pair(
        &mut self,
        key: EditId,
        position: usize,
        cx: &mut Context<Self>,
        change: impl FnOnce(&mut ForeignKey),
    ) {
        self.edit_structure_state(cx, |editor| {
            let Some(fk) = foreign(std::sync::Arc::make_mut(&mut editor.draft), key) else {
                return;
            };
            change(fk);
            if editor.pending_fk_pairs.get(&key) == Some(&position)
                && position < fk.key.columns.len()
                && position < fk_target_column_count(&fk.target)
            {
                editor.pending_fk_pairs.remove(&key);
            }
        });
    }
}
