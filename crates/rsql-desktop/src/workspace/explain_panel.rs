use super::*;
use gpui_kit::{
    component::{ActiveTheme, label::Label},
    prelude::FluentBuilder,
};
use rsql_desktop::explain::{ExplainPlan, PlanNode};
use std::collections::HashSet;

#[derive(Default)]
pub(super) struct Expansion {
    collapsed: HashSet<String>,
}

struct Tree<'a> {
    plan: &'a ExplainPlan,
    collapsed: &'a HashSet<String>,
    owner: Owner,
    generation: u64,
}

fn icon(path: &'static str, color: Hsla) -> Svg {
    svg()
        .path(path)
        .size(px(14.))
        .flex_shrink_0()
        .text_color(color)
}

fn duration(value: Option<f64>) -> String {
    value.map_or_else(|| "N/A".into(), |v| format!("{v:.2}ms"))
}

fn cost(value: Option<f64>, precision: usize) -> String {
    value.map_or_else(|| "N/A".into(), |v| format!("{v:.precision$}"))
}

fn count(value: Option<impl ToString>) -> String {
    let Some(value) = value else {
        return "N/A".into();
    };
    let digits = value.to_string();
    if !digits.bytes().all(|b| b.is_ascii_digit()) {
        return digits;
    }
    let mut text = String::new();
    for (i, c) in digits.char_indices() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            text.push(',');
        }
        text.push(c);
    }
    text
}

fn stat(label: &'static str, value: String, warning: bool, cx: &App) -> Div {
    let tint = if warning {
        cx.theme().danger
    } else {
        cx.theme().group_box_foreground
    };
    div()
        .flex()
        .items_center()
        .child(
            Label::new(format!("{label}: "))
                .id(SharedString::from(format!("stat-label-{label}")))
                .text_color(cx.theme().muted_foreground)
                .line_height(px(14.)),
        )
        .child(
            Label::new(value)
                .id(SharedString::from(format!("stat-value-{label}")))
                .font_weight(FontWeight::MEDIUM)
                .text_color(tint)
                .line_height(px(14.)),
        )
}

fn metric(path: &'static str, label: &'static str, value: String, cx: &App) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(6.))
        .child(icon(path, cx.theme().primary))
        .child(
            Label::new(label)
                .id(SharedString::from(format!("metric-label-{label}")))
                .text_color(cx.theme().muted_foreground),
        )
        .child(
            Label::new(value)
                .id(SharedString::from(format!("metric-value-{label}")))
                .font_weight(FontWeight::MEDIUM),
        )
}

impl Workspace {
    pub(super) fn explain_panel(
        &self,
        owner: Owner,
        generation: u64,
        plan: &ExplainPlan,
        cx: &Context<Self>,
    ) -> Div {
        let mut summary = div()
            .flex_shrink_0()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(16.))
            .px(px(16.))
            .py(px(8.))
            .border_b_1()
            .border_color(cx.theme().border);
        if !plan.analyzed() {
            summary = summary.child(
                div()
                    .border_1()
                    .border_color(cx.theme().border)
                    .rounded(px(4.))
                    .px(px(6.))
                    .py(px(2.))
                    .text_size(px(10.))
                    .text_color(cx.theme().muted_foreground)
                    .child(
                        Label::new("Estimated plan · not executed")
                            .text_color(cx.theme().muted_foreground)
                            .line_height(px(14.)),
                    ),
            );
        }
        if let Some(t) = plan.planning_time {
            summary = summary.child(metric(
                "icons/timer.svg",
                "Planning:",
                duration(Some(t)),
                cx,
            ));
        }
        if plan.analyzed() {
            summary = summary.child(metric(
                "icons/clock.svg",
                "Execution:",
                duration(plan.execution_time),
                cx,
            ));
        }
        summary = summary.child(metric(
            "icons/layers.svg",
            "Total Cost:",
            cost(plan.root.total_cost, 2),
            cx,
        ));
        let tree = Tree {
            plan,
            collapsed: &self.editors[&owner].expansion.collapsed,
            owner,
            generation,
        };
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .bg(cx.theme().group_box)
            .text_color(cx.theme().group_box_foreground)
            .text_size(px(12.))
            .child(summary)
            .child(
                div()
                    .id("plan-tree")
                    .flex_1()
                    .min_h_0()
                    .overflow_scroll()
                    .p(px(16.))
                    .children(tree.rows(cx)),
            )
    }
}

impl Tree<'_> {
    fn rows(&self, cx: &Context<Workspace>) -> Vec<Div> {
        let mut rows = Vec::new();
        let mut pending = vec![(&self.plan.root, String::from("plan-0"), 0)];
        while let Some((node, path, depth)) = pending.pop() {
            if !self.collapsed.contains(&path) {
                pending.extend(
                    node.children
                        .iter()
                        .enumerate()
                        .rev()
                        .map(|(i, child)| (child, format!("{path}-{i}"), depth + 1)),
                );
            }
            let row = self.node(node, path, cx);
            rows.push(
                div()
                    .relative()
                    .pl(px(depth as f32 * 20.))
                    .pb(px(4.))
                    .when(depth > 0, |s| {
                        s.child(ancestor_guides(depth, cx.theme().border.opacity(0.5)))
                    })
                    .child(row),
            );
        }
        rows
    }

    fn node(&self, node: &PlanNode, path: String, cx: &Context<Workspace>) -> Div {
        let expanded = !self.collapsed.contains(&path);
        let leaf = node.children.is_empty();
        let color = cx.theme().muted_foreground;
        let arrow = match (leaf, expanded) {
            (true, _) => icon("icons/arrow-right.svg", color.opacity(0.3)),
            (_, true) => icon("icons/chevron-down.svg", color),
            _ => icon("icons/chevron-right.svg", color),
        };
        let header = div().flex().items_center().gap(px(8.)).child(arrow).child(
            div()
                .font_weight(FontWeight::SEMIBOLD)
                .child(Label::new(node.label())),
        );
        let pct = self.plan.percent(node);
        let bar = div()
            .id("plan-weight")
            .test_support()
            .ml(px(22.))
            .mt(px(6.))
            .h(px(6.))
            .rounded_full()
            .overflow_hidden()
            .bg(cx.theme().muted)
            .children(pct.map(|p| {
                let tint = if p > 80. {
                    cx.theme().danger
                } else if p > 40. {
                    cx.theme().warning
                } else {
                    cx.theme().success
                };
                div()
                    .id("plan-weight-fill")
                    .test_support()
                    .h_full()
                    .rounded_full()
                    .w(relative((p.clamp(1., 100.) / 100.) as f32))
                    .bg(tint)
            }));
        let mut stats = div()
            .ml(px(22.))
            .mt(px(4.))
            .flex()
            .flex_wrap()
            .gap_x(px(16.))
            .gap_y(px(2.))
            .text_size(px(10.));
        if self.plan.analyzed() {
            stats = stats
                .child(stat(
                    "Time",
                    duration(node.elapsed()),
                    pct.is_some_and(|p| p > 60.),
                    cx,
                ))
                .child(stat("Rows", count(node.actual_rows.as_ref()), false, cx))
                .child(stat(
                    "Est.",
                    count(node.plan_rows.as_ref()),
                    node.row_warning(),
                    cx,
                ));
            if node.has_zero_estimate() {
                stats = stats.child(stat("Ratio", "N/A".into(), false, cx));
            }
            if node.actual_loops.is_some_and(|l| l > 1) {
                stats = stats.child(stat("Loops", count(node.actual_loops), false, cx));
            }
        } else {
            stats = stats.child(stat("Est. rows", count(node.plan_rows.as_ref()), false, cx));
        }
        stats = stats.child(stat(
            "Cost",
            format!(
                "{}..{}",
                cost(node.startup_cost, 1),
                cost(node.total_cost, 1)
            ),
            false,
            cx,
        ));
        if node.hit_blocks.is_some() {
            stats = stats.child(stat("Buffers Hit", count(node.hit_blocks), false, cx));
        }
        if node.read_blocks.is_some_and(|n| n > 0) {
            stats = stats.child(stat("Buffers Read", count(node.read_blocks), true, cx));
        }
        let mut row = div()
            .id(SharedString::from(path.clone()))
            .test_support()
            .rounded(px(6.))
            .border_1()
            .border_color(cx.theme().border)
            .p(px(8.))
            .child(header)
            .child(bar)
            .child(stats)
            .children(node.details().into_iter().enumerate().map(|(i, text)| {
                div()
                    .mt(px(2.))
                    .ml(px(22.))
                    .text_size(px(10.))
                    .text_color(color)
                    .child(
                        Label::new(text)
                            .id(("plan-detail", i))
                            .line_height(px(14.))
                            .text_color(color),
                    )
            }));
        if !leaf {
            let owner = self.owner.clone();
            let key = path.clone();
            let generation = self.generation;
            row = row
                .role(Role::Button)
                .aria_label(node.label())
                .aria_expanded(expanded)
                .cursor_pointer()
                .hover(|s| s.bg(cx.theme().accent.opacity(0.3)))
                .on_click(cx.listener(move |v, _, _, cx| {
                    let docs = v.session.documents.borrow();
                    if !docs.active().is_some_and(|t| {
                        t.id == owner.0 && t.pane(owner.1).outcome_generation() == Some(generation)
                    }) {
                        return;
                    }
                    if let Some(editor) = v.editors.get_mut(&owner) {
                        let collapsed = &mut editor.expansion.collapsed;
                        if !collapsed.remove(&key) {
                            collapsed.insert(key.clone());
                        }
                        cx.notify();
                    }
                }));
        }
        div().child(row)
    }
}

fn ancestor_guides(depth: usize, color: Hsla) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, w, _| {
            let visible = bounds.intersect(&w.content_mask().bounds);
            if visible.is_empty() {
                return;
            }
            let first_x = bounds.left() + px(10.);
            let first = ((visible.left() - first_x) / px(20.)).floor().max(0.) as usize;
            for level in first..depth {
                let x = first_x + px(level as f32 * 20.);
                if x >= visible.right() {
                    break;
                }
                w.paint_quad(fill(
                    Bounds::new(point(x, bounds.top()), size(px(1.), bounds.size.height)),
                    color,
                ));
            }
        },
    )
    .absolute()
    .inset_0()
    .size_full()
}
