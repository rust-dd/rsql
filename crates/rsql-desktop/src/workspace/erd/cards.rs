use super::*;
use gpui_kit::component::{ActiveTheme, label::Label};
use gpui_kit::prelude::FluentBuilder;
use rsql_desktop::erd::{COLUMN_HEIGHT, Column, HEADER_HEIGHT, Table, TableBox};

pub(super) fn card(
    p: &Panel,
    b: TableBox,
    table: &Table,
    connected: bool,
    cx: &App,
) -> impl IntoElement + use<> {
    let camera = p.camera;
    let len = |n| camera.length(n);
    let bounds = camera.bounds(b);
    let hovered = p.hover == Some(b.table);
    let dimmed = p.hover.is_some() && !hovered && !connected;
    let colors = cx.theme();
    let mut card = div()
        .id(("erd-table", b.table.0 as usize))
        .test_support()
        .role(Role::Group)
        .aria_label(table.identity.name.clone())
        .absolute()
        .left(bounds.left())
        .top(bounds.top())
        .w(bounds.size.width)
        .h(bounds.size.height)
        .rounded(len(6.))
        .bg(colors.group_box)
        .font_family(colors.mono_font_family.clone())
        .shadow(vec![
            BoxShadow::new(px(0.), len(2.), black().opacity(0.1)).blur_radius(len(3.)),
        ])
        .when(dimmed, |el| el.opacity(0.25));
    if bounds.top() + len(HEADER_HEIGHT) >= px(0.) && bounds.top() < p.viewport.size.height {
        card = card.child(
            div()
                .absolute()
                .top_0()
                .w_full()
                .h(len(HEADER_HEIGHT))
                .rounded_t(len(6.))
                .bg(colors.primary)
                .border_b(len(1.))
                .border_color(colors.border)
                .flex()
                .items_center()
                .px(len(10.))
                .overflow_hidden()
                .child(
                    Label::new(rsql_desktop::erd::table_title(&table.identity.name))
                        .id("name")
                        .truncate()
                        .text_size(len(12.))
                        .line_height(len(16.))
                        .font_weight(FontWeight::BOLD)
                        .text_color(colors.primary_foreground),
                ),
        );
    }
    let row_start = bounds.top() + len(HEADER_HEIGHT);
    let row_height = len(COLUMN_HEIGHT);
    let first = ((-row_start / row_height).floor().max(0.) as usize).min(table.columns.len());
    let end = (((p.viewport.size.height - row_start) / row_height)
        .ceil()
        .max(0.) as usize)
        .min(table.columns.len());
    for i in first..end {
        card = card.child(row(camera, &table.columns[i], i, table.columns.len(), cx));
    }
    card.child(
        div()
            .absolute()
            .inset_0()
            .rounded(len(6.))
            .border(len(if hovered { 2. } else { 1. }))
            .border_color(if hovered || connected {
                colors.primary
            } else {
                colors.border
            }),
    )
}

fn row(camera: Camera, column: &Column, i: usize, count: usize, cx: &App) -> Stateful<Div> {
    let len = |n| camera.length(n);
    let colors = cx.theme();
    let key = column.key_label();
    let ty = if column.nullable {
        format!("{}?", column.ty)
    } else {
        column.ty.clone()
    };
    div()
        .id(("column", column.id.number as usize))
        .absolute()
        .left(len(1.))
        .right(len(1.))
        .top(len(HEADER_HEIGHT + i as f64 * COLUMN_HEIGHT))
        .h(len(COLUMN_HEIGHT))
        .pl(len(if key.is_some() { 27. } else { 9. }))
        .pr(len(7.))
        .gap(len(2.))
        .flex()
        .items_center()
        .overflow_hidden()
        .when(i % 2 == 1, |r| {
            r.bg(colors.muted.opacity(0.3))
                .when(i + 1 == count, |r| r.rounded(len(4.)))
        })
        .children(key.map(|marker| {
            Label::new(marker)
                .id("key")
                .absolute()
                .left(len(7.))
                .text_size(len(10.))
                .line_height(len(COLUMN_HEIGHT))
                .font_weight(FontWeight::BOLD)
                .text_color(
                    colors
                        .primary
                        .opacity(if column.primary { 1. } else { 0.7 }),
                )
        }))
        .child(
            Label::new(column.name.clone())
                .id("name")
                .min_w_0()
                .flex_1()
                .truncate()
                .text_size(len(11.))
                .line_height(len(COLUMN_HEIGHT))
                .font_weight(if column.primary {
                    FontWeight::SEMIBOLD
                } else {
                    FontWeight::NORMAL
                })
                .text_color(if column.primary {
                    colors.foreground
                } else {
                    colors.muted_foreground
                }),
        )
        .child(
            Label::new(ty)
                .id("type")
                .min_w_0()
                .max_w_full()
                .truncate()
                .text_right()
                .text_size(len(10.))
                .line_height(len(COLUMN_HEIGHT))
                .text_color(colors.muted_foreground.opacity(0.6)),
        )
}
