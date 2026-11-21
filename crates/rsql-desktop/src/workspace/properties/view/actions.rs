use super::super::maintenance::ActionStatus;
use super::{
    header::icon,
    structure::{line, stack},
    *,
};
use gpui_kit::{
    assets::IconName,
    base::TestTextExt,
    component::{Disableable, Sizable, button::ButtonVariants, input::Input},
};
use rsql_core::services::object_action::ObjectAction;

pub(super) fn outcome(status: ActionStatus, message: String, cx: &App) -> impl IntoElement {
    let (id, color, glyph) = match status {
        ActionStatus::Success => ("action-success", cx.theme().success, IconName::Check),
        ActionStatus::Error => ("action-error", cx.theme().danger, IconName::TriangleAlert),
        ActionStatus::Warning => (
            "action-warning",
            cx.theme().warning,
            IconName::TriangleAlert,
        ),
        ActionStatus::Indeterminate => {
            ("action-indeterminate", cx.theme().warning, IconName::Clock3)
        }
    };
    div()
        .id(id)
        .test_support()
        .flex()
        .items_start()
        .gap(px(8.))
        .flex_shrink_0()
        .px(px(12.))
        .py(px(8.))
        .rounded(px(6.))
        .bg(color.opacity(0.1))
        .child(div().flex_shrink_0().child(icon(glyph, 14., color)))
        .child(
            Label::new(message)
                .text_color(color)
                .text_size(px(12.))
                .line_height(px(16.)),
        )
}

fn operation_title(label: &'static str, color: Hsla) -> impl IntoElement {
    let title = div()
        .id("operation-title")
        .test_support()
        .relative()
        .role(Role::Label)
        .aria_value(label)
        .font_weight(FontWeight::MEDIUM)
        .text_color(color)
        .child(label);
    title.test_text(label)
}

fn caption(label: &'static str, cx: &App) -> Div {
    div()
        .px(px(4.))
        .text_size(px(10.))
        .line_height(px(15.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(cx.theme().muted_foreground)
        .child(
            Label::new(label.to_uppercase())
                .id(format!("heading-{label}"))
                .line_height(px(15.))
                .text_color(cx.theme().muted_foreground),
        )
}

impl Panel {
    /// Source order: table quick queries, the latest outcome, then the operation cards.
    pub(super) fn action_cards(
        &self,
        p: &Properties,
        outcome: Option<impl IntoElement>,
        cx: &App,
    ) -> Div {
        let mut out = stack().gap(px(12.));
        if p.object.kind.is_table() {
            let mut quick = line().gap(px(8.));
            for a in [Action::Select, Action::Count, Action::Latest] {
                let object = p.object.clone();
                let capture = p.capture.clone();
                quick = quick.child(
                    self.button(a.label(), move |v, w, cx| {
                        v.dismiss_properties(w, cx);
                        v.object_action(&capture, &object, a, w, cx)
                    })
                    .when(a == Action::Select, |b| b.label("SELECT TOP 100"))
                    .outline()
                    .icon(IconName::Play)
                    .disabled(p.preserve()),
                );
            }
            out = out.child(
                stack()
                    .gap(px(8.))
                    .child(caption("Quick Queries", cx))
                    .child(quick),
            );
        }
        out = out.children(outcome);
        if p.action.retired {
            return out;
        }
        if p.preserve() {
            out = out.child("Apply or discard the structure draft before running an action.");
        }
        let mut cards = stack().gap(px(8.));
        for &action in ObjectAction::available(p.object.kind) {
            cards = cards.child(self.maintenance_card(p, action, cx));
        }
        out.child(caption("Maintenance & Operations", cx))
            .child(cards)
    }

    fn maintenance_card(&self, p: &Properties, action: ObjectAction, cx: &App) -> impl IntoElement {
        let label = action.label(p.object.kind);
        use rsql_desktop::metadata::ObjectKind;
        // Source wording, except where it would misstate PostgreSQL behavior.
        let text = match (action, p.object.kind) {
            (ObjectAction::Analyze, _) => "Update table statistics for the query planner.",
            (ObjectAction::Vacuum, _) => "Reclaim storage occupied by dead tuples.",
            (ObjectAction::VacuumFull, _) => {
                "Rewrite table to reclaim max space. Locks table exclusively."
            }
            (ObjectAction::Reindex, _) => "Rebuild all indexes on this table.",
            (ObjectAction::Truncate, _) => {
                "Remove all rows. This action commits immediately on success."
            }
            (ObjectAction::Drop, ObjectKind::View) => "Permanently delete this view.",
            (ObjectAction::Drop, ObjectKind::MaterializedView) => {
                "Permanently delete this materialized view."
            }
            (ObjectAction::Drop, ObjectKind::ForeignTable) => {
                "Permanently delete this foreign table."
            }
            (ObjectAction::Drop, ObjectKind::Procedure) => "Permanently delete this procedure.",
            (ObjectAction::Drop, kind) if kind.is_table() => {
                "Permanently delete this table and all its data."
            }
            (ObjectAction::Drop, _) => "Permanently delete this function.",
            (ObjectAction::DropCascade, ObjectKind::View) => "Drop view and all dependent objects.",
            (ObjectAction::DropCascade, ObjectKind::Procedure) => {
                "Drop procedure and all dependent objects."
            }
            (ObjectAction::DropCascade, _) => {
                "Drop function and all dependent objects (triggers, etc.)."
            }
            (ObjectAction::Refresh, _) => "Refresh data by re-executing the query.",
            (ObjectAction::RefreshConcurrently, _) => {
                "Refresh without locking reads. Requires a unique index on a populated view."
            }
        };
        let destructive = action.drops() || action == ObjectAction::Truncate;
        let glyph = if destructive {
            IconName::Trash
        } else if action == ObjectAction::Reindex {
            IconName::Key
        } else {
            IconName::RefreshCw
        };
        let border = cx.theme().border;
        let muted = cx.theme().muted;
        let color = if destructive {
            cx.theme().danger
        } else {
            cx.theme().muted_foreground
        };
        let input = p
            .action
            .input
            .as_ref()
            .filter(|_| p.action.selection == Some(action));
        let selected = input.is_some();
        let card = stack()
            .gap_0()
            .id(format!("operation-card-{label}"))
            .test_support()
            .border_1()
            .border_color(border.opacity(if selected { 0.4 } else { 0.25 }))
            .bg(muted.opacity(if selected { 0.2 } else { 0.1 }))
            .rounded(px(12.))
            .hover(move |d| d.bg(muted.opacity(0.2)).border_color(border.opacity(0.4)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .px(px(14.))
                    .py(px(12.))
                    .child(icon(glyph, 16., color))
                    .child(
                        stack()
                            .flex_1()
                            .min_w_0()
                            .gap_0()
                            .child(operation_title(
                                label,
                                if destructive {
                                    color
                                } else {
                                    cx.theme().foreground
                                },
                            ))
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .line_height(px(16.5))
                                    .text_color(cx.theme().muted_foreground)
                                    .child(text),
                            ),
                    )
                    .when(!selected, |d| {
                        d.child(
                            self.button(label, move |v, w, cx| v.choose_action(action, w, cx))
                                .label("Run")
                                .accessibility_label(format!("Run {label}"))
                                .px(px(12.))
                                .when(!destructive, |button| button.outline())
                                .when(destructive, |button| button.ghost().text_color(color))
                                .disabled(p.preserve()),
                        )
                    }),
            );
        card.when_some(input, |d, input| {
            let exact = input.read(cx).value().as_ref() == p.object.name;
            d.child(stack().gap(px(8.)).px(px(14.)).pb(px(12.))
                .when(action.requires_top_level(p.object.kind), |d| {
                    d.child(Label::new("Runs outside a transaction. Keep the table and schema names unchanged until it finishes."))
                })
                .when(action.requires_top_level(p.object.kind)
                    && p.object.kind == rsql_desktop::metadata::ObjectKind::PartitionedTable, |d| {
                    d.child(Label::new("Earlier partitions may already have changed if a later partition fails or the action is cancelled."))
                })
                .when_some(p.action.review.as_ref(), |d, review| {
                    d.child(div().text_size(px(11.)).child(Label::new(review.sql().to_owned())))
                })
                .child(line().gap(px(8.))
                    .child(div().flex().gap(px(4.)).items_center().text_size(px(10.))
                        .text_color(cx.theme().muted_foreground)
                        .child("Type")
                        .child(div().font_weight(FontWeight::SEMIBOLD)
                            .text_color(cx.theme().foreground).child(p.object.name.clone()))
                        .child("to confirm"))
                    .child(div().flex_1().min_w(px(100.)).child(
                        Input::new(input).id("action-name").with_size(px(28.)).h(px(28.))
                            .rounded(px(6.)).bg(cx.theme().background)
                            .border_color(cx.theme().border.opacity(0.4)),
                    ))
                    .child(self.button("Confirm", |v, w, cx| v.confirm_action(w, cx))
                        .ghost().text_size(px(11.))
                        .when(destructive, |button| button.text_color(color))
                        .disabled(!exact || p.action.review.is_none()))
                    .child(self.button("Cancel action", |v, _, cx| {
                        let p = v.properties.as_mut().unwrap();
                        p.action = Default::default();
                        p.status.clear();
                        cx.notify();
                    }).label("Cancel").ghost().text_size(px(11.)).text_color(cx.theme().muted_foreground))))
        })
    }
}
