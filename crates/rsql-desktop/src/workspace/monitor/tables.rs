use super::*;
use gpui_kit::{
    component::{ActiveTheme, label::Label},
    prelude::FluentBuilder,
};
use view::color;

fn width(header: &str) -> f32 {
    match header {
        "Query" => 300.,
        "Bloat %" => 145.,
        "Definition" => 400.,
        "Schema" | "Table" | "Index" | "Last Vacuum" | "Last Analyze" | "Mode" | "Backend"
        | "State" => 155.,
        "PID" | "Size" => 70.,
        _ => 110.,
    }
}

impl Panel {
    pub(super) fn table(&self, rows: &Rows, cx: &Context<Self>) -> Div {
        let (count, empty) = rows.count_and_empty();
        let mut panel = div().flex().flex_col().flex_1().min_h_0().gap_2();
        if let Some((note, tone)) = rows.note() {
            panel = panel.child(
                div()
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .text_color(color(tone, cx))
                    .bg(color(tone, cx).opacity(0.08))
                    .child(Label::new(note.clone()).text_color(color(tone, cx))),
            );
        }
        if let Some(details) = self.inspection.and_then(|i| rows.details(i)) {
            panel = panel.child(
                div()
                    .id("monitor-inspection")
                    .max_h(px(220.))
                    .overflow_y_scroll()
                    .border_1()
                    .border_color(cx.theme().border)
                    .rounded_md()
                    .p_2()
                    .test_support()
                    .children(
                        details
                            .into_iter()
                            .enumerate()
                            .map(|(index, (field, text))| {
                                div()
                                    .id(("inspection-field", index))
                                    .flex()
                                    .items_start()
                                    .gap_2()
                                    .child(
                                        Label::new(field)
                                            .w(px(150.))
                                            .flex_shrink_0()
                                            .text_color(cx.theme().muted_foreground),
                                    )
                                    .child(
                                        div()
                                            .min_w_0()
                                            .flex_1()
                                            .whitespace_normal()
                                            .child(Label::new(text.to_owned())),
                                    )
                            }),
                    ),
            );
        }
        let headers = rows.headers();
        let heading = div()
            .flex()
            .bg(cx.theme().muted.opacity(0.5))
            .text_size(px(10.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(cx.theme().muted_foreground)
            .children(headers.iter().enumerate().map(|(index, h)| {
                div()
                    .id(("monitor-heading", index))
                    .w(px(width(h)))
                    .px_2()
                    .py_1p5()
                    .child(Label::new(*h).text_color(cx.theme().muted_foreground))
            }));
        if count == 0 {
            return panel
                .child(heading)
                .child(div().py_6().child(Label::new(empty)));
        }
        let lane = self.page.lane().unwrap();
        let owner = cx.entity();
        let list = uniform_list(
            ("monitor-rows", lane as usize),
            count,
            move |range, _, cx| {
                let p = owner.read(cx);
                let section = &p.snapshot.sections[lane as usize];
                let revision = section.data_revision;
                let Some(rows) = section.rows.as_ref() else {
                    return Vec::new();
                };
                range
                    .filter_map(|i| {
                        let cells = rows.cells(i)?;
                        let owner = owner.downgrade();
                        let waiting = matches!(rows,Rows::Locks(r) if r.rows[i].status=="waiting");
                        Some(
                            div()
                                .id(("monitor-row", i))
                                .test_support()
                                .role(Role::Button)
                                .aria_label(format!("Inspect row {}", i + 1))
                                .tab_index(0)
                                .h(px(26.))
                                .flex()
                                .items_center()
                                .cursor_pointer()
                                .text_size(px(11.))
                                .border_b_1()
                                .border_color(cx.theme().border)
                                .when(waiting, |d| d.bg(cx.theme().warning.opacity(0.08)))
                                .hover(|d| d.bg(cx.theme().muted.opacity(0.3)))
                                .children(
                                    cells
                                        .into_iter()
                                        .zip(headers)
                                        .enumerate()
                                        .map(|(index, (c, h))| c.element(index, width(h), cx)),
                                )
                                .on_click(move |_, _, cx| {
                                    let _ = owner.update(cx, |p, cx| {
                                        if p.current()
                                            && p.page.lane() == Some(lane)
                                            && p.snapshot.sections[lane as usize].data_revision
                                                == revision
                                        {
                                            p.inspection = (p.inspection != Some(i)).then_some(i);
                                            cx.notify();
                                        }
                                    });
                                }),
                        )
                    })
                    .collect()
            },
        );
        panel.child(
            div()
                .id("monitor-table")
                .overflow_x_scroll()
                .min_h_0()
                .flex_1()
                .border_1()
                .border_color(cx.theme().border)
                .rounded_md()
                .child(
                    div()
                        .h_full()
                        .min_w_full()
                        .w(px(headers.iter().map(|h| width(h)).sum()))
                        .flex()
                        .flex_col()
                        .child(heading)
                        .child(list.flex_1().min_h_0()),
                ),
        )
    }
}
