use super::super::{
    editor::{Editor, Field, Page, change_key},
    transfer::stored_definition,
};
use super::{layout::toolbar_button, structure::stack, structure_keys::default_name, *};
use gpui_kit::assets::IconName;
use gpui_kit::component::{Sizable, checkbox::Checkbox};
use rsql_desktop::structure::{Action as FkAction, *};

fn shown_on(object: &NewObject, page: Page) -> bool {
    matches!(
        (object, page),
        (NewObject::Unique(_), Page::Unique)
            | (NewObject::Index { .. }, Page::Index)
            | (NewObject::ForeignKey(_), Page::Foreign)
    )
}

fn stored_on(stored: &StoredObject, page: Page) -> bool {
    matches!(
        (stored, page),
        (StoredObject::ForeignKey(_), Page::Foreign)
            | (StoredObject::Unique(_), Page::Unique)
            | (StoredObject::Index(_), Page::Index)
    )
}

fn card(cx: &App) -> Div {
    stack()
        .gap(px(8.))
        .px(px(14.))
        .py(px(12.))
        .rounded(px(12.))
        .border_1()
        .border_color(cx.theme().border.opacity(0.25))
        .bg(cx.theme().muted.opacity(0.1))
}

fn caption(text: &'static str, cx: &App) -> Div {
    div()
        .text_size(px(9.))
        .line_height(px(13.5))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(cx.theme().muted_foreground.opacity(0.6))
        .child(text)
}

impl Panel {
    /// Source order: active stored and added cards first, then the removed rows.
    pub(super) fn structure_objects(&self, e: &Editor, cx: &App) -> Div {
        let page = e.page;
        let snapshot = e.draft.snapshot();
        let stored = snapshot
            .stored_objects()
            .filter(|s| stored_on(s, page))
            .collect::<Vec<_>>();
        let mut out = stack().gap(px(12.)).py(px(4.));
        for object in stored.iter().filter(|s| !e.draft.is_dropped(s.id())) {
            let id = object.id();
            out = out.child(
                card(cx)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(12.))
                                    .child(object.name().to_owned()),
                            )
                            .child(self.remove_restore(e, id, cx)),
                    )
                    .child(self.stored_preview(id, stored_definition(object), cx)),
            );
        }
        for (id, object) in e.draft.added_objects().iter() {
            if shown_on(object, page) {
                out = out.child(self.added_card(e, *id, object, cx));
            }
        }
        for object in stored.iter().filter(|s| e.draft.is_dropped(s.id())) {
            let muted = cx.theme().muted_foreground;
            let foreground = cx.theme().foreground;
            out = out.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .px(px(12.))
                    .py(px(8.))
                    .rounded(px(8.))
                    .border_1()
                    .border_color(cx.theme().danger.opacity(0.2))
                    .bg(cx.theme().danger.opacity(0.05))
                    .opacity(0.6)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(12.))
                            .line_through()
                            .child(object.name().to_owned()),
                    )
                    .child(
                        self.drop_toggle(object.id(), true)
                            .accessibility_label("Restore")
                            .text_size(px(10.))
                            .text_color(muted)
                            .hover(move |s| s.text_color(foreground))
                            .child("Restore"),
                    ),
            );
        }
        out
    }

    fn added_card(&self, e: &Editor, id: EditId, object: &NewObject, cx: &App) -> Div {
        let mut header = div().flex().items_center().gap(px(8.)).child(self.field(
            e,
            id,
            Field::KeyName,
            &object.key().name,
            cx,
        ));
        if let NewObject::Index { unique, .. } = object {
            header = header.child(
                Checkbox::new(format!("unique-{id:?}"))
                    .with_size(px(12.))
                    .checked(*unique)
                    .label("Unique")
                    .text_size(px(10.))
                    .text_color(cx.theme().muted_foreground)
                    .on_change(self.listener(true, move |v, _, cx| {
                        v.edit_draft(cx, |d| {
                            if let Some(NewObject::Index { unique, .. }) = d.new_object_mut(id) {
                                *unique = !*unique
                            }
                        })
                    })),
            );
        }
        let card = card(cx)
            .border_color(cx.theme().success.opacity(0.2))
            .bg(cx.theme().success.opacity(0.05))
            .child(header.child(self.remove_restore(e, id, cx)));
        match object {
            NewObject::ForeignKey(fk) => card.child(self.foreign(e, id, fk, cx)),
            _ => card
                .child(caption("COLUMNS", cx))
                .child(self.key_chips(e, id, object.key(), cx)),
        }
    }

    fn key_chips(&self, e: &Editor, id: EditId, key: &KeyColumns, cx: &App) -> Div {
        let mut chips = div().flex().flex_wrap().gap(px(6.));
        for column in e.draft.columns() {
            let Some(c) = column.current() else { continue };
            let col_id = column.id();
            let selected = key.columns.contains(&col_id);
            let border = cx.theme().border;
            let primary = cx.theme().primary;
            chips = chips.child(
                self.press(format!("key-{id:?}-{col_id:?}"), move |v, _, cx| {
                    v.edit_draft(cx, |d| {
                        change_key(d, id, |k| {
                            if k.columns.contains(&col_id) {
                                k.columns.retain(|x| *x != col_id)
                            } else {
                                k.columns.push(col_id)
                            }
                        })
                    })
                })
                .accessibility_label(c.name.clone())
                .aria_selected(selected)
                .px(px(8.))
                .py(px(2.))
                .rounded(px(6.))
                .border_1()
                .text_size(px(10.))
                .line_height(px(15.))
                .map(|chip| {
                    if selected {
                        chip.bg(primary.opacity(0.1))
                            .border_color(primary.opacity(0.3))
                            .text_color(cx.theme().foreground)
                    } else {
                        chip.border_color(border.opacity(0.2))
                            .text_color(cx.theme().muted_foreground)
                            .hover(move |s| s.border_color(border.opacity(0.4)))
                    }
                })
                .child(c.name.clone()),
            );
        }
        chips
    }

    fn stored_preview(&self, id: EditId, definition: &str, cx: &App) -> Div {
        let end = definition.floor_char_boundary(definition.len().min(400));
        let excerpt = &definition[..end];
        let abbreviated = end < definition.len();
        let preview = if abbreviated {
            format!("{excerpt}…")
        } else {
            excerpt.to_owned()
        };
        div()
            .flex()
            .flex_col()
            .min_w_0()
            .child(
                Label::new(preview)
                    .id("stored-structure-preview")
                    .text_size(px(11.))
                    .text_color(cx.theme().muted_foreground),
            )
            .when(abbreviated, |d| {
                d.child(toolbar_button(
                    self.press(format!("copy-stored-{id:?}"), move |v, w, cx| {
                        v.transfer_stored_definition(id, w, cx);
                    }),
                    IconName::Copy,
                    "Copy full definition",
                    cx,
                ))
            })
    }

    pub(super) fn add_row(&self, e: &Editor, cx: &App) -> impl IntoElement {
        let page = e.page;
        let (id, label) = match page {
            Page::Columns => ("Add column", "Add Column"),
            Page::Foreign => ("Add foreign key", "Add Foreign Key"),
            Page::Unique => ("Add unique constraint", "Add Unique Constraint"),
            _ => ("Add index", "Add Index"),
        };
        let foreground = cx.theme().foreground;
        let hover = cx.theme().muted.opacity(0.3);
        self.press(id, move |v, _, cx| {
            v.edit_draft(cx, |d| add_entry(d, page));
            if page == Page::Foreign {
                let project = v.properties.as_ref().unwrap().object.project.clone();
                v.load_fk_schemas(&project, cx);
            }
        })
        .accessibility_label(label)
        .w_full()
        .flex()
        .items_center()
        .justify_start()
        .gap(px(6.))
        .px(px(8.))
        .py(px(6.))
        .rounded(px(8.))
        .text_size(px(12.))
        .line_height(px(16.))
        .text_color(cx.theme().muted_foreground)
        .hover(move |s| s.text_color(foreground).bg(hover))
        .child(gpui_kit::component::Icon::new(IconName::Plus).size(px(14.)))
        .child(label)
    }
}

/// New entries get the source's numbered, table-derived defaults; every name stays editable.
fn add_entry(d: &mut StructureDraft, page: Page) {
    let table = d.snapshot().object().name.clone();
    let count = |page| {
        d.snapshot()
            .stored_objects()
            .filter(|s| stored_on(s, page))
            .count()
            + d.added_objects()
                .iter()
                .filter(|(_, o)| shown_on(o, page))
                .count()
            + 1
    };
    let key = |suffix: String| KeyColumns {
        name: default_name(&table, &suffix),
        columns: Vec::new(),
    };
    match page {
        Page::Columns => {
            let name = format!("new_column_{}", d.columns().len() + 1);
            d.add_column(Column {
                name,
                data_type: "text".into(),
                nullable: true,
                default: None,
            });
        }
        Page::Unique => {
            let key = key(format!("unique_{}", count(page)));
            d.add_object(NewObject::Unique(key));
        }
        Page::Index => {
            let key = key(format!("idx_{}", count(page)));
            d.add_object(NewObject::Index { key, unique: false });
        }
        Page::Foreign => {
            let key = key(format!("fk_{}", count(page)));
            d.add_object(NewObject::ForeignKey(ForeignKey {
                key,
                target: ForeignTarget::ThisTable(vec![]),
                on_delete: FkAction::NoAction,
                on_update: FkAction::NoAction,
            }));
        }
        Page::Primary => (),
    }
}
