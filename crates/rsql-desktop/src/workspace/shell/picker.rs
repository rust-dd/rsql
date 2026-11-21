use super::*;
use gpui_kit::component::{
    Disableable,
    button::{Button, ButtonVariants},
    menu::{DropdownMenu, PopupMenuItem},
};
use rsql_core::services::ProjectTicket;
use rsql_desktop::connection::ConnectionProfile;

struct Choice {
    profile: ConnectionProfile,
    ticket: ProjectTicket,
    tab: String,
    previous: Option<String>,
    previous_ticket: Option<ProjectTicket>,
}

impl Choice {
    fn apply(&self, view: &mut Workspace, w: &mut Window, cx: &mut Context<Workspace>) {
        let same_tab = view.session.documents.borrow().active().is_some_and(|t| {
            t.kind == Kind::Query && t.id == self.tab && t.project == self.previous
        });
        let same_profile = view
            .session
            .connections
            .borrow()
            .profiles
            .iter()
            .any(|p| p.same_route(&self.profile));
        if same_tab
            && same_profile
            && self.ticket.is_current()
            && self
                .previous_ticket
                .as_ref()
                .is_none_or(ProjectTicket::is_current)
        {
            view.update_documents(w, cx, |docs| {
                if let Some(t) = docs.selected.and_then(|i| docs.tabs.get_mut(i)) {
                    t.set_project(Some(self.profile.id.clone()));
                }
            });
        }
    }
}

impl Workspace {
    fn picker_label(&self, p: &ConnectionProfile) -> String {
        let suffix = if self
            .navigation
            .status
            .get(&p.id)
            .is_some_and(|s| s == "Connected")
        {
            ""
        } else {
            " • disconnected"
        };
        format!("{} ({}){suffix}", p.display_name(), p.database)
    }

    pub(super) fn connection_picker(
        &self,
        project: Option<&ConnectionProfile>,
        cx: &Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let title = project.map_or_else(
            || "Select connection...".to_owned(),
            |p| self.picker_label(p),
        );
        let target = self
            .session
            .documents
            .borrow()
            .active()
            .filter(|t| t.kind == Kind::Query)
            .map(|t| (t.id.clone(), t.project.clone()));
        let trigger = Button::new("connection-picker")
            .ghost()
            .rounded(px(8.))
            .h(px(26.))
            .px(px(8.))
            .py(px(4.))
            .min_w_0()
            .flex_shrink(1.)
            .border_1()
            .border_color(theme.border.opacity(0.5))
            .bg(theme.input)
            .accessibility_label(format!("Current connection: {title}"))
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_size(px(12.))
                    .text_color(theme.foreground)
                    .child(title),
            )
            .child(Icon::new(IconName::ChevronDown).size(px(12.)));
        let Some((tab, original_project)) = target else {
            return trigger.disabled(true).into_any_element();
        };
        let owner = cx.weak_entity();
        trigger
            .dropdown_menu(move |mut menu, _, cx| {
                let Some(workspace) = owner.upgrade() else {
                    return menu;
                };
                let view = workspace.read(cx);
                let capture = |id: &str| view.session.services.capture_project_ticket(id).ok();
                let previous_ticket = original_project.as_deref().and_then(capture);
                for p in &view.session.connections.borrow().profiles {
                    let Some(ticket) = capture(&p.id) else {
                        continue;
                    };
                    let choice = Choice {
                        profile: p.clone(),
                        ticket,
                        tab: tab.clone(),
                        previous: original_project.clone(),
                        previous_ticket: previous_ticket.clone(),
                    };
                    let owner = owner.clone();
                    menu = menu.item(
                        PopupMenuItem::new(view.picker_label(p))
                            .checked(original_project.as_ref() == Some(&p.id))
                            .on_click(move |_, w, cx| {
                                let _ = owner.update(cx, |v, cx| choice.apply(v, w, cx));
                            }),
                    );
                }
                menu
            })
            .into_any_element()
    }
}
