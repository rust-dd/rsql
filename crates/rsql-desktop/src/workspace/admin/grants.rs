use super::*;
use gpui_kit::component::label::Label;
use view::heading;

fn row(id: impl Into<ElementId>, cx: &App) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .w_full()
        .items_center()
        .gap_2()
        .px_3()
        .py_1()
        .border_b_1()
        .border_color(cx.theme().border.opacity(0.2))
}

fn cell(content: impl IntoElement) -> Div {
    div().flex_1().min_w_0().child(content)
}

fn columns(names: &[&'static str], cx: &App) -> Stateful<Div> {
    row("columns", cx)
        .flex_shrink_0()
        .bg(cx.theme().muted.opacity(0.4))
        .text_color(cx.theme().muted_foreground)
        .children(
            names
                .iter()
                .map(|&n| cell(Label::new(n).id(n).text_color(cx.theme().muted_foreground))),
        )
}

impl Panel {
    pub(super) fn grant_sections(&self, cx: &Context<Self>) -> Div {
        let box_style = || {
            div()
                .border_1()
                .rounded_xl()
                .overflow_hidden()
                .border_color(cx.theme().border.opacity(0.3))
        };
        let mut database = box_style().id("role-database-grants");
        if !self.grants.databases.is_empty() {
            database = database
                .child(columns(&["Database", "Privilege"], cx))
                .children(self.grants.databases.iter().enumerate().map(|(i, g)| {
                    row(i, cx)
                        .child(cell(
                            div()
                                .flex()
                                .items_center()
                                .gap_1()
                                .child(
                                    Icon::new(IconName::Database)
                                        .size(px(12.))
                                        .text_color(cx.theme().muted_foreground),
                                )
                                .child(Label::new(g.database.clone())),
                        ))
                        .child(cell(Label::new(g.privilege.clone())))
                }));
        } else if self.grants.database.ready() {
            database = database.child(Label::new("No privileges found"));
        }
        let mut tables = box_style()
            .id("role-table-grants")
            .test_support()
            .max_h(px(300.))
            .flex()
            .flex_col();
        if !self.grants.tables.is_empty() {
            tables = tables
                .child(columns(&["Schema", "Table", "Privileges"], cx))
                .child(
                    div()
                        .id("privilege-rows")
                        .min_h_0()
                        .overflow_y_scroll()
                        .children(self.grants.tables.iter().enumerate().map(|(i, g)| {
                            row(i, cx)
                                .child(cell(
                                    Label::new(g.schema.clone())
                                        .text_color(cx.theme().muted_foreground),
                                ))
                                .child(cell(Label::new(g.table.clone())))
                                .child(cell(div().flex().gap_1().flex_wrap().children(
                                    g.privileges.iter().map(|v| {
                                        div()
                                            .rounded_sm()
                                            .px_1()
                                            .py(px(2.))
                                            .bg(cx.theme().primary.opacity(0.05))
                                            .child(
                                                Label::new(v.clone())
                                                    .id(SharedString::from(v.clone()))
                                                    .text_size(px(10.)),
                                            )
                                    }),
                                )))
                        })),
                );
        } else if self.grants.table.ready() {
            tables = tables.child(Label::new("No privileges found"));
        }
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(heading(
                "database-grants-heading",
                "Database Privileges",
                cx,
            ))
            .children(self.status(Lane::Database, &self.grants.database, cx))
            .child(database)
            .child(
                heading(
                    "table-grants-heading",
                    &format!("Table Privileges ({})", self.grants.tables.len()),
                    cx,
                )
                .mt_3(),
            )
            .children(self.status(Lane::Table, &self.grants.table, cx))
            .child(tables)
    }
}
