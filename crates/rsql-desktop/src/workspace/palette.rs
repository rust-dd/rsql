use super::*;
use gpui_kit::component::input::{InputEvent, InputState};

use rsql_desktop::{
    connection::ConnectionProfile,
    results::{ResultAction, diff::SnapshotKey},
    saved::SavedQueryKey,
};

mod entries;
mod presentation;
mod selection;
mod view;
mod workspaces;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Page {
    Root,
    Connections,
    Schema,
    Saved,
    SaveWorkspace,
    SaveQuery,
    LoadWorkspace,
    DeleteWorkspace,
}
pub(super) struct Palette {
    page: Page,
    workspace_operation: workspaces::Operation,
    list_focus: FocusHandle,
    search: Entity<InputState>,
    selected_search: SharedString,
    selected: selection::Selection,
    pressed: Option<Rc<view::RenderedChoice>>,
    scrolled_to: Option<usize>,
    previous: Option<FocusHandle>,
    saved_query: Option<super::saved_query::SaveTarget>,
    project: Option<String>,
    profile: Option<ConnectionProfile>,
    tab: Option<String>,
    side: Side,
    schema_capture: Option<Rc<catalog::Capture>>,
    pin_target: Option<SnapshotKey>,
    clear_target: Option<uuid::Uuid>,
    ticket: Option<rsql_core::services::ProjectTicket>,
    _subscription: Subscription,
    scroll: ListState,
    row_layout: Vec<presentation::Row>,
}
#[derive(Clone, PartialEq, Eq)]
enum Command {
    Page(Page),
    Choose(String),
    Create,
    Connection(Rc<ConnectionProfile>),
    Disconnect(String),
    NewQuery,
    Terminal,
    Admin(Kind, Rc<catalog::Capture>),
    Monitor(Rc<catalog::Capture>),
    Compare(Rc<catalog::Capture>),
    Erd(Rc<catalog::Capture>, String),
    Draft(Rc<catalog::Capture>, catalog::CatalogDraft),
    Execute(execution::Action),
    Format,
    Save,
    OpenSaved(SavedQueryKey),
    Named(workspaces::Command),
    Cancel,
    Pin(SnapshotKey),
    ClearPin(uuid::Uuid),
    Split,
    Close,
    Updates,
    Theme,
}

fn execution_side(action: execution::Action, focused: Side) -> Side {
    match action {
        execution::Action::Explain | execution::Action::Analyze => Side::Main,
        execution::Action::Run | execution::Action::Script => focused,
    }
}

impl Workspace {
    pub(super) fn toggle_palette(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        if self.palette.is_some() {
            self.dismiss_palette(w, cx);
        } else if self.properties.is_none()
            && !self.navigation.modal_open
            && !self.importing()
            && !self.saving_query
            && !w.has_active_dialog(cx)
        {
            self.open_palette(Page::Root, w, cx);
        }
    }
    pub(super) fn open_palette(&mut self, page: Page, w: &mut Window, cx: &mut Context<Self>) {
        let previous = w.focused(cx);
        self.session.ensure_named_workspaces();
        let search = cx.new(|cx| InputState::new(w, cx).placeholder(page.placeholder()));
        let subscription = cx.subscribe(&search, |v, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change)
                && let Some(p) = &mut v.palette
            {
                p.sync_search_selection(cx);
                cx.notify();
            }
        });
        let docs = self.session.documents.borrow();
        let project = docs.active().and_then(|t| t.project.clone());
        let tab = docs.active().map(|t| t.id.clone());
        let side = self
            .focused_owner(w, cx)
            .map_or(Side::Main, |(_, side)| side);
        let ticket = project
            .as_ref()
            .and_then(|id| self.session.services.capture_project_ticket(id).ok());
        let profile = tab.as_deref().and_then(|id| self.profile_for(id));
        let main = docs.active().and_then(|t| t.main.view());
        self.palette = Some(Palette {
            schema_capture: project
                .as_deref()
                .and_then(|id| catalog::Capture::new(self, id))
                .map(Rc::new),
            pin_target: main
                .filter(|v| v.allows(ResultAction::Pin))
                .map(|v| v.key()),
            clear_target: self.session.pin.borrow().as_ref().map(|p| p.id),
            profile,
            side,
            ticket,
            page,
            workspace_operation: Default::default(),
            list_focus: cx.focus_handle(),
            search,
            selected_search: Default::default(),
            selected: Default::default(),
            pressed: None,
            scrolled_to: None,
            previous,
            saved_query: None,
            project,
            tab,
            _subscription: subscription,
            scroll: ListState::new(0, ListAlignment::Top, px(72.)),
            row_layout: Vec::new(),
        });
        self.palette.as_ref().unwrap().focus(w, cx);
        cx.notify();
    }
    fn dismiss_palette(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        if let Some(p) = self.palette.take()
            && let Some(f) = p.previous
        {
            f.focus(w, cx);
        }
        cx.notify();
    }
    fn page(&mut self, page: Page, w: &mut Window, cx: &mut Context<Self>) {
        let Some(p) = &mut self.palette else { return };
        p.page = page;
        if page != Page::SaveQuery {
            p.saved_query = None;
        }
        p.workspace_operation = Default::default();
        p.reset_selection();
        p.search.update(cx, |s, cx| {
            s.set_value("", w, cx);
            s.set_placeholder(page.placeholder(), w, cx);
        });
        p.sync_search_selection(cx);
        p.focus(w, cx);
        cx.notify();
    }
    fn submit_palette_query(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        if let Some(p) = &self.palette
            && let Some(target) = &p.saved_query
            && self.submit_saved_query(target, &p.search.read(cx).value())
        {
            self.dismiss_palette(w, cx);
        }
    }

    fn run_command(&mut self, cmd: Command, w: &mut Window, cx: &mut Context<Self>) {
        use Command::*;
        if let Page(page) = cmd {
            self.page(page, w, cx);
            return;
        }
        if let Named(command) = cmd {
            self.run_workspace_command(command, w, cx);
            return;
        }
        let Some(p) = &self.palette else { return };
        let tab = p.tab.clone();
        let project = p.project.clone();
        let side = p.side;
        let current = p.ticket.as_ref().is_none_or(|t| t.is_current())
            && self
                .session
                .documents
                .borrow()
                .tabs
                .iter()
                .any(|t| Some(&t.id) == tab.as_ref() && t.project == project);
        let current_tools =
            current && tab.as_deref().and_then(|id| self.profile_for(id)) == p.profile;
        if matches!(cmd, Save) && current_tools {
            let owner = (tab.unwrap(), Side::Main);
            if let Some(target) = self.capture_saved_query(&owner, cx) {
                self.palette.as_mut().unwrap().saved_query = Some(target);
                self.page(self::Page::SaveQuery, w, cx);
            }
            return;
        }
        self.dismiss_palette(w, cx);
        match cmd {
            Page(_) | Named(_) => {}
            Execute(action) if current => {
                self.run_query((tab.unwrap(), execution_side(action, side)), action, w, cx);
            }
            Pin(key) if current_tools => self.pin_result(&tab.unwrap(), key, cx),
            ClearPin(id) => self.clear_pin(id, cx),
            Pin(_) => {}
            Cancel if current => self.cancel_tab_query(&tab.unwrap(), cx),
            Format if current_tools => self.format_query((tab.unwrap(), Side::Main), w, cx),
            Execute(_) | Cancel | Format | Save => {}
            OpenSaved(key) => self.open_saved_query(&key, w, cx),
            Terminal => self.dispatch_open_terminal(w, cx),
            NewQuery => self.update_documents(w, cx, |d| {
                d.open(Kind::Query, project);
            }),
            Compare(capture) if current_tools => self.open_schema_diff(&capture, w, cx),
            Admin(kind, capture) if current_tools => self.open_admin(kind, &capture, w, cx),
            Monitor(capture) => self.open_admin(Kind::Monitor, &capture, w, cx),
            Admin(..) | Compare(_) => {}
            Erd(capture, schema) => self.open_erd(&capture, &schema, w, cx),
            Draft(capture, sql) if capture.current(self) => {
                catalog::open_draft(self, &capture.project, sql.object.sql(), w, cx)
            }
            Draft(..) => {}
            Choose(id) => self.update_documents(w, cx, |d| {
                if let Some(t) = d
                    .tabs
                    .iter_mut()
                    .find(|t| Some(&t.id) == tab.as_ref() && t.kind == Kind::Query)
                {
                    t.set_project(Some(id));
                } else if tab.is_none() {
                    d.open(Kind::Query, Some(id));
                }
            }),
            Create => self.open_form(ConnectionProfile::default(), w, cx),
            Connection(profile) => {
                let current = self
                    .session
                    .connections
                    .borrow()
                    .profiles
                    .iter()
                    .any(|p| p == profile.as_ref());
                if current
                    && !matches!(
                        self.navigation.status.get(&profile.id).map(String::as_str),
                        Some("Connected" | "Connecting")
                    )
                {
                    self.connect_sidebar((*profile).clone(), w, cx);
                }
            }
            Disconnect(id) => self.disconnect(id, cx),
            Split => self.update_documents(w, cx, |d| {
                if let Some(t) = d.tabs.iter_mut().find(|t| Some(&t.id) == tab.as_ref()) {
                    t.toggle_split();
                }
            }),
            Close => self.update_documents(w, cx, |d| {
                if let Some(id) = tab {
                    d.close(&id);
                }
            }),
            Updates => self.check_for_updates(),
            Theme => self.toggle_theme(w, cx),
        }
    }
}
