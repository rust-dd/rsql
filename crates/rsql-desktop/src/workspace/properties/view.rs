use super::*;
use gpui_kit::component::{ActiveTheme, button::Button, label::Label};
use gpui_kit::prelude::FluentBuilder;
pub(super) mod actions;
mod columns;
mod foreign;
mod foreign_pairs;
pub(super) mod header;
pub(super) mod layout;
mod structure;
mod structure_columns;
mod structure_keys;
mod structure_objects;
use properties::qualified;
use theme::controls::compact_button;

pub(super) struct Panel {
    owner: WeakEntity<Workspace>,
    id: Uuid,
    _observe: Subscription,
}

impl Panel {
    pub(super) fn watch(owner: &Entity<Workspace>, id: Uuid, cx: &mut App) -> Entity<Self> {
        cx.new(|cx| Self {
            id,
            owner: owner.downgrade(),
            _observe: cx.observe(owner, |_, _, cx| cx.notify()),
        })
    }

    pub(super) fn button(
        &self,
        label: impl Into<SharedString>,
        callback: impl Fn(&mut Workspace, &mut Window, &mut Context<Workspace>) + 'static,
    ) -> Button {
        self.control(label, true, callback)
    }

    fn control(
        &self,
        label: impl Into<SharedString>,
        require_current: bool,
        callback: impl Fn(&mut Workspace, &mut Window, &mut Context<Workspace>) + 'static,
    ) -> Button {
        let label = label.into();
        compact_button(Button::new(label.clone()))
            .label(label)
            .on_click(self.listener(require_current, callback))
    }

    pub(super) fn press(
        &self,
        id: impl Into<ElementId>,
        callback: impl Fn(&mut Workspace, &mut Window, &mut Context<Workspace>) + 'static,
    ) -> gpui_kit::base::Button {
        gpui_kit::base::Button::new(id).on_click(self.listener(true, callback))
    }

    fn listener<F, E: 'static>(
        &self,
        require_current: bool,
        callback: F,
    ) -> impl Fn(&E, &mut Window, &mut App) + 'static + use<F, E>
    where
        F: Fn(&mut Workspace, &mut Window, &mut Context<Workspace>) + 'static,
    {
        let owner = self.owner.clone();
        let id = self.id;
        move |_, w, cx| {
            let _ = owner.update(cx, |v, cx| {
                if v.properties.as_ref().is_some_and(|p| p.id == id && !p.busy)
                    && (!require_current || v.property_current(id))
                {
                    callback(v, w, cx);
                }
            });
        }
    }

    fn body(&self, p: &Properties, cx: &App) -> Div {
        let mut panel = div().flex().flex_col().gap(px(16.));
        let actions_tab = p.tab == Tab::Actions;
        let status = if actions_tab {
            p.status.as_str()
        } else {
            p.status_text()
        };
        let mut outcome = p
            .action
            .notice
            .as_ref()
            .filter(|_| actions_tab)
            .map(|(kind, message)| actions::outcome(*kind, message.clone(), cx));
        if let Some(effects) = p.action.effects {
            panel = panel.child(
                div()
                    .id("maintenance-effects")
                    .test_support()
                    .flex_shrink_0()
                    .text_color(cx.theme().warning)
                    .child(Label::new(effects).text_color(cx.theme().warning)),
            );
        }
        if p.busy {
            panel = panel.child(Label::new(p.status.clone()));
            if let Some((execution, _)) = p.action.running.as_ref() {
                let execution = *execution;
                let id = p.id;
                let owner = self.owner.clone();
                panel = panel.child(
                    compact_button(Button::new("Cancel running action"))
                        .label("Cancel running action")
                        .on_click(move |_, _, cx| {
                            let _ = owner.update(cx, |v, cx| {
                                if let Some(p) =
                                    v.properties.as_mut().filter(|p| p.id == id && p.busy)
                                    && p.action
                                        .running
                                        .as_ref()
                                        .is_some_and(|(current, _)| *current == execution)
                                    && let Some((_, cancel)) = p.action.running.take()
                                {
                                    let _ = cancel.send(());
                                    p.status = "Cancelling action…".into();
                                    cx.notify();
                                }
                            });
                        }),
                );
            }
            return panel;
        }
        if !p.capture.current(self.owner.upgrade().unwrap().read(cx))
            || p.editor.as_ref().is_some_and(|e| e.recover)
        {
            panel = panel.children(outcome.take());
            if !status.is_empty() {
                panel = panel.child(Label::new(status.to_owned()).id("properties-diagnostic"));
            }
            let workspace = self.owner.upgrade().unwrap();
            let same_route = workspace
                .read(cx)
                .navigation
                .connected_routes
                .get(&p.object.project)
                .is_some_and(|r| r.is_current() && r.same_generation(&p.capture.route));
            if !same_route || p.editor.is_none() {
                return panel.child(
                    "The connection or catalog changed. Reopen properties for the current object.",
                );
            }
            let recovery = p.editor.as_ref().is_some_and(|e| e.recover);
            let (message, label) = if recovery {
                (
                    "Reload the table metadata before editing again.",
                    "Reload metadata",
                )
            } else {
                (
                    "Catalog changed. Your draft is preserved.",
                    "Discard draft and reload",
                )
            };
            if !recovery && let Some(e) = &p.editor {
                panel = panel.child(format!("{} draft changes", e.draft.change_count()));
                if let Some((_, sql)) = &e.review {
                    panel = panel.child(Label::new(SharedString::from(&sql.preview)));
                }
                panel = panel.child(self.control("Continue draft", false, |v, _, cx| {
                    v.revalidate_structure(cx)
                }));
            }
            return panel
                .child(message)
                .child(self.control(label, false, |v, w, cx| v.reload_structure(w, cx)));
        }
        if p.tab == Tab::Structure
            && let Some(e) = &p.editor
        {
            if !status.is_empty() {
                panel = panel.child(
                    div()
                        .id("structure-status")
                        .test_support()
                        .max_h(px(72.))
                        .flex_shrink_0()
                        .overflow_y_scroll()
                        .child(Label::new(status.to_owned())),
                );
            }
            return panel.h_full().min_h_0().child(self.edit_structure(e, cx));
        }
        if !status.is_empty() {
            panel = panel.child(Label::new(status.to_owned()).id("properties-diagnostic"));
        }
        if actions_tab {
            return panel.child(self.action_cards(p, outcome, cx));
        }
        panel.children(
            p.tab
                .sections(p.object.kind.is_table())
                .iter()
                .filter_map(|&section| self.section_content(p, section, cx)),
        )
    }

    fn section_content(&self, p: &Properties, section: Section, cx: &App) -> Option<AnyElement> {
        use gpui_kit::assets::IconName;

        let element = match p.sections.get(&section) {
            None | Some(LoadState::Loading { .. }) => {
                if matches!(section, Section::Columns) && p.tab == Tab::Structure
                    || matches!(
                        section,
                        Section::Constraints
                            | Section::Triggers
                            | Section::Rules
                            | Section::Policies
                    )
                {
                    return None;
                }
                overview::loading(cx).into_any_element()
            }
            Some(LoadState::Failed(message)) => {
                let color = cx.theme().danger;
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(12.))
                    .py(px(48.))
                    .child(
                        div()
                            .size(px(40.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .bg(color.opacity(0.1))
                            .child(header::icon(IconName::TriangleAlert, 20., color)),
                    )
                    .child(
                        div()
                            .text_size(px(14.))
                            .text_color(color)
                            .child(Label::new(message.clone())),
                    )
                    .child(
                        self.button(
                            format!("Retry {}", section_label(section)),
                            move |v, w, cx| v.load_property(section, w, cx),
                        )
                        .label("Retry")
                        .icon(IconName::RefreshCw),
                    )
                    .into_any_element()
            }
            Some(LoadState::Ready(Content::Overview(rows))) if p.object.kind.is_table() => {
                overview::statistics(rows, cx).into_any_element()
            }
            Some(LoadState::Ready(Content::Overview(rows))) => {
                self.object_overview(p, rows, cx).into_any_element()
            }
            Some(LoadState::Ready(Content::Details(rows))) => {
                if rows.is_empty() {
                    return None;
                }
                self.overview_details(section, rows, cx).into_any_element()
            }
            Some(LoadState::Ready(Content::Columns(rows))) => {
                self.columns(rows, cx).into_any_element()
            }
            Some(LoadState::Ready(Content::Indexes(rows))) => {
                self.indexes(rows, cx).into_any_element()
            }
            Some(LoadState::Ready(Content::Structure(_))) => div()
                .text_color(cx.theme().muted_foreground)
                .child(Label::new(
                    "Read-only structure. Editing requires table ownership.",
                ))
                .into_any_element(),
            Some(LoadState::Ready(Content::Keys(keys))) => {
                let incoming = section == Section::Incoming;
                if keys.is_empty() {
                    if incoming
                        && matches!(p.sections.get(&Section::Outgoing), Some(LoadState::Ready(Content::Keys(outgoing))) if outgoing.is_empty())
                    {
                        return Some(
                            div()
                                .text_center()
                                .py(px(32.))
                                .text_size(px(14.))
                                .text_color(cx.theme().muted_foreground)
                                .child("No foreign key relationships found.")
                                .into_any_element(),
                        );
                    }
                    return None;
                }
                let (id, title, glyph) = if incoming {
                    ("Incoming", "Incoming (referenced by)", IconName::ArrowLeft)
                } else {
                    (
                        "Outgoing",
                        "Outgoing (this table references)",
                        IconName::ArrowRight,
                    )
                };
                div()
                    .id(id)
                    .test_support()
                    .child(overview::section(
                        title,
                        glyph,
                        self.related_table(p, keys, incoming, cx),
                        cx,
                    ))
                    .into_any_element()
            }
            Some(LoadState::Ready(Content::Ddl(text))) => self.ddl(p, text, cx).into_any_element(),
        };
        Some(element)
    }
}

impl Render for Panel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(owner) = self.owner.upgrade() else {
            return div();
        };
        let Some(p) = owner
            .read(cx)
            .properties
            .as_ref()
            .filter(|p| p.id == self.id)
        else {
            return div();
        };
        let connection = owner
            .read(cx)
            .session
            .connections
            .borrow()
            .profiles
            .iter()
            .find(|profile| profile.id == p.object.project)
            .map(|profile| profile.display_name().to_owned())
            .unwrap_or_else(|| p.object.project.clone());
        let nested = (p.tab == Tab::Structure && p.editor.as_ref().is_some_and(|e| !e.recover)
            || p.tab == Tab::Ddl
                && matches!(
                    p.sections.get(&Section::Ddl),
                    Some(LoadState::Ready(Content::Ddl(_)))
                ))
            && p.capture.current(owner.read(cx))
            && !p.busy;
        let body = div()
            .id(("property-page", p.tab as usize))
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .when(!nested, |d| d.overflow_y_scroll())
            .pt(if p.tab == Tab::Structure {
                px(8.)
            } else {
                px(12.)
            })
            .when(
                matches!(p.tab, Tab::Overview | Tab::ForeignKeys | Tab::Actions),
                |d| d.pb(px(12.)),
            )
            .child(
                self.body(p, cx)
                    .when(nested, |body| body.flex_1().min_h_0()),
            );
        div().size_full().min_h_0().child(
            div()
                .id("properties-panel")
                .test_support()
                .size_full()
                .flex()
                .flex_col()
                .min_h_0()
                .text_color(cx.theme().foreground)
                .text_size(px(12.))
                .line_height(px(16.))
                .child(self.header(p, connection, cx))
                .child(
                    div()
                        .id("properties-content")
                        .test_support()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_h_0()
                        .px(px(20.))
                        .pb(px(20.))
                        .child(body),
                ),
        )
    }
}

fn section_label(s: Section) -> &'static str {
    match s {
        Section::Columns => "columns",
        Section::Indexes => "indexes",
        Section::Constraints => "constraints",
        Section::Triggers => "triggers",
        Section::Rules => "rules",
        Section::Policies => "policies",
        Section::Overview => "overview",
        Section::Structure => "structure",
        Section::Ddl => "DDL",
        Section::Incoming => "incoming keys",
        Section::Outgoing => "outgoing keys",
    }
}
