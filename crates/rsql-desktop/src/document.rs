use crate::execution::{Outcome, QueryResult, Status};
use crate::results::{PageLane, ResultView, ViewChoice};
use rsql_core::services::ProjectTicket;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, ops::Range};
use tokio::sync::oneshot;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    Query,
    Terminal,
    Monitor,
    Erd(String),
    Notify,
    Roles,
    SchemaDiff,
    Extensions,
    Enums,
    PgSettings,
}
impl Kind {
    pub fn durable(&self) -> bool {
        !matches!(self, Self::Terminal | Self::Notify)
    }
    fn title(&self, number: usize) -> String {
        match self {
            Self::Query => return format!("Query {number}"),
            Self::Erd(schema) => return format!("ERD: {schema}"),
            Self::Terminal => "Terminal",
            Self::Monitor => "Monitor",
            Self::Notify => "LISTEN/NOTIFY",
            Self::Roles => "Roles",
            Self::SchemaDiff => "Schema Diff",
            Self::Extensions => "Extensions",
            Self::Enums => "Enum Types",
            Self::PgSettings => "PG Settings",
        }
        .into()
    }
}

#[derive(Default, Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Appearance {
    #[default]
    Light,
    Dark,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    #[default]
    Main,
    Right,
}

#[derive(Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Draft {
    pub text: String,
    #[serde(default)]
    pub selection: Range<usize>,
    #[serde(default)]
    pub reversed: bool,
}

/// Dropping the sender wakes the execution owner's cancellation receiver.
#[derive(Default)]
struct Work {
    lane: PageLane,
    view: Option<ResultView>,
    generation: u64,
    active: Option<oneshot::Sender<()>>,
    outcome: Option<Outcome>,
    committed: Option<(ProjectTicket, String)>,
    write_delivery: Option<(Uuid, Uuid, ProjectTicket)>,
    empty_choice: Option<ViewChoice>,
    explaining: bool,
    cancelled: bool,
    started: Option<std::time::Instant>,
    stopped_after: Option<std::time::Duration>,
}
impl Work {
    fn invalidate_execution(&mut self, explain: bool) {
        let notice = explain.then(|| self.committed.take()).flatten();
        let delivery = explain.then(|| self.write_delivery.take()).flatten();
        self.invalidate();
        self.committed = notice;
        self.write_delivery = delivery;
        self.explaining = explain;
    }
    fn invalidate(&mut self) {
        self.generation += 1;
        self.active = None;
        self.view = None;
        self.empty_choice = None;
        self.outcome = None;
        self.committed = None;
        self.write_delivery = None;
        self.explaining = false;
        self.cancelled = false;
        self.started = None;
        self.stopped_after = None;
    }
}

#[derive(Default, Serialize, Deserialize)]
pub struct Pane {
    #[serde(flatten)]
    pub draft: Draft,
    #[serde(skip)]
    work: Work,
}
impl Pane {
    pub fn register_write(&mut self, result: Uuid, route: ProjectTicket) -> Uuid {
        let operation = Uuid::new_v4();
        self.work.write_delivery = Some((result, operation, route));
        operation
    }
    pub fn take_write_delivery(
        &mut self,
        result: Uuid,
        operation: Uuid,
        route: &ProjectTicket,
    ) -> bool {
        let matches = self
            .work
            .write_delivery
            .as_ref()
            .is_some_and(|(source, id, ticket)| {
                *source == result
                    && *id == operation
                    && route.is_current()
                    && ticket.same_generation(route)
            });
        if matches {
            self.work.write_delivery = None;
        }
        matches
    }
    pub fn record_commit(&mut self, route: rsql_core::services::ProjectTicket, message: String) {
        if let Some(view) = self.work.view.as_mut()
            && view.write_uncertain
            && view.route.same_generation(&route)
        {
            view.edit_error =
                Some("Run the query again to refresh the committed rows before editing.".into());
        }
        self.work.committed = Some((route, message));
    }
    pub fn commit_notice(&self) -> Option<&str> {
        self.work
            .committed
            .as_ref()
            .filter(|(route, _)| route.is_current())
            .map(|(_, message)| message.as_str())
    }
    pub fn is_busy(&self) -> bool {
        self.work.active.is_some()
    }
    pub fn elapsed(&self) -> Option<std::time::Duration> {
        self.work
            .started
            .map(|start| start.elapsed())
            .or(self.work.stopped_after)
            .or_else(|| self.outcome().map(|o| o.elapsed))
    }
    pub fn was_cancelled(&self) -> bool {
        self.work.cancelled
    }
    pub fn generation(&self) -> u64 {
        self.work.generation
    }
    pub fn outcome_generation(&self) -> Option<u64> {
        self.outcome().map(|_| self.work.generation)
    }
    pub fn outcome(&self) -> Option<&Outcome> {
        self.work
            .outcome
            .as_ref()
            .filter(|o| o.route_ticket.as_ref().is_some_and(|t| t.is_current()))
    }
    pub fn view(&self) -> Option<&ResultView> {
        self.outcome()?;
        self.retained_view()
    }
    pub fn choice(&self) -> ViewChoice {
        self.view().map_or_else(
            || match self.work.empty_choice.unwrap_or_default() {
                ViewChoice::Explain if self.outcome().is_none() => ViewChoice::Grid,
                choice => choice,
            },
            ResultView::choice,
        )
    }
    pub fn choose_empty(&mut self, choice: ViewChoice) {
        let plan = self
            .outcome()
            .is_some_and(|outcome| matches!(outcome.status, Status::Plan(_)));
        if self.view().is_none()
            && (matches!(choice, ViewChoice::Grid | ViewChoice::History)
                || (plan && choice == ViewChoice::Explain))
        {
            self.work.empty_choice = Some(choice);
        }
    }
    pub fn retained_view(&self) -> Option<&ResultView> {
        self.work.view.as_ref().filter(|v| v.route.is_current())
    }
    pub fn view_mut(&mut self) -> Option<&mut ResultView> {
        self.outcome()?;
        self.retained_view_mut()
    }
    pub fn retained_view_mut(&mut self) -> Option<&mut ResultView> {
        self.work.view.as_mut().filter(|v| v.route.is_current())
    }
    pub fn result(&self) -> Option<&QueryResult> {
        Some(&self.view()?.data)
    }
}

#[derive(Clone)]
pub struct Request {
    requested: ViewChoice,
    project: Option<String>,
    tab: String,
    side: Side,
    generation: u64,
}

impl Request {
    pub fn owner(&self) -> (&str, Side) {
        (&self.tab, self.side)
    }
}

#[derive(Serialize, Deserialize)]
pub struct Tab {
    pub id: String,
    pub kind: Kind,
    pub title: String,
    pub project: Option<String>,
    pub timeout: Option<u64>,
    /// Legacy schema metadata for non-ERD tabs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    #[serde(default)]
    pub split: bool,
    #[serde(skip)]
    pub focused_side: Side,
    #[serde(default)]
    pub main: Pane,
    #[serde(default)]
    pub right: Pane,
}
impl Tab {
    pub fn pane(&self, side: Side) -> &Pane {
        match side {
            Side::Main => &self.main,
            Side::Right => &self.right,
        }
    }
    pub fn pane_mut(&mut self, side: Side) -> &mut Pane {
        match side {
            Side::Main => &mut self.main,
            Side::Right => &mut self.right,
        }
    }

    pub fn toggle_split(&mut self) {
        if self.kind == Kind::Query {
            self.split = !self.split;
            if !self.split {
                self.focused_side = Side::Main;
                self.right.work.invalidate();
            }
        }
    }
    pub fn set_project(&mut self, project: Option<String>) {
        if self.project != project {
            self.project = project;
            self.main.work.invalidate();
            self.right.work.invalidate();
        }
    }
    pub fn cancel(&mut self, side: Side) -> bool {
        let work = &mut self.pane_mut(side).work;
        if work.active.is_none() {
            return false;
        }
        let elapsed = work.started.map(|start| start.elapsed());
        work.invalidate_execution(work.explaining);
        work.stopped_after = elapsed;
        work.cancelled = true;
        true
    }
    pub fn begin(&mut self, side: Side) -> Option<(Request, oneshot::Receiver<()>)> {
        self.begin_mode(side, crate::execution::Mode::Query)
    }
    pub fn begin_mode(
        &mut self,
        side: Side,
        mode: crate::execution::Mode,
    ) -> Option<(Request, oneshot::Receiver<()>)> {
        if self.kind != Kind::Query || (side == Side::Right && !self.split) {
            return None;
        }
        let id = self.id.clone();
        let project = self.project.clone();
        let work = &mut self.pane_mut(side).work;
        let requested = work
            .view
            .as_ref()
            .map_or(work.empty_choice.unwrap_or_default(), ResultView::choice);
        let retain = side == Side::Main && mode != crate::execution::Mode::Query;
        let retained = retain
            .then(|| work.view.take())
            .flatten()
            .filter(|v| v.data.is_rowset && v.route.is_current());
        work.invalidate_execution(retain);
        work.view = retained.map(|mut v| {
            v.prepare_explain();
            v
        });
        let (sender, receiver) = oneshot::channel();
        work.active = Some(sender);
        work.started = Some(std::time::Instant::now());
        Some((
            Request {
                requested,
                project,
                tab: id,
                side,
                generation: work.generation,
            },
            receiver,
        ))
    }
}

pub struct Documents {
    pub tabs: Vec<Tab>,
    pub selected: Option<usize>,
    pub theme: Appearance,
}

impl Default for Documents {
    fn default() -> Self {
        let mut docs = Self {
            tabs: vec![],
            selected: None,
            theme: Appearance::Light,
        };
        docs.open(Kind::Query, None);
        docs
    }
}
impl Documents {
    pub fn open(&mut self, kind: Kind, project: Option<String>) -> &mut Tab {
        let existing = if matches!(kind, Kind::Query | Kind::Terminal) {
            None
        } else {
            self.tabs
                .iter()
                .position(|tab| tab.kind == kind && tab.project == project)
        };
        let index = existing.unwrap_or_else(|| {
            let index = self.tabs.len();
            self.tabs.push(Tab {
                id: Uuid::new_v4().to_string(),
                title: kind.title(index + 1),
                kind,
                project,
                timeout: None,
                schema: None,
                split: false,
                focused_side: Side::Main,
                main: Pane::default(),
                right: Pane::default(),
            });
            index
        });
        self.select_index(index);
        &mut self.tabs[index]
    }
    pub fn active(&self) -> Option<&Tab> {
        self.tabs.get(self.selected?)
    }
    pub fn select(&mut self, id: &str) {
        if let Some(index) = self.tabs.iter().position(|t| t.id == id) {
            self.select_index(index);
        }
    }
    pub fn select_index(&mut self, index: usize) {
        if index < self.tabs.len() {
            self.selected = Some(index);
            self.sync_views();
        }
    }

    pub fn sync_views(&self) {
        for (i, tab) in self.tabs.iter().enumerate() {
            for side in [Side::Main, Side::Right] {
                if let Some(view) = tab.pane(side).view() {
                    if Some(i) == self.selected && (side == Side::Main || tab.split) {
                        match view.choice() {
                            ViewChoice::Grid => view.loader.resume(),
                            ViewChoice::Map if side == Side::Main => view.loader.suspend_grid(),
                            _ => view.loader.suspend(),
                        }
                    } else {
                        view.loader.suspend();
                    }
                }
            }
        }
    }

    pub fn close(&mut self, id: &str) {
        let Some(index) = self.tabs.iter().position(|t| t.id == id) else {
            return;
        };
        self.tabs.remove(index);
        self.selected = self
            .selected
            .filter(|_| !self.tabs.is_empty())
            .map(|active| {
                if active >= index {
                    active.saturating_sub(1)
                } else {
                    active
                }
            });
        self.sync_views();
    }
    pub fn close_others(&mut self, id: &str) {
        if self.tabs.iter().any(|t| t.id == id) {
            self.tabs.retain(|t| t.id == id);
            self.select_index(0);
        }
    }
    pub fn release_obsolete_results(&mut self, project: &str) {
        for tab in self
            .tabs
            .iter_mut()
            .filter(|t| t.project.as_deref() == Some(project))
        {
            for pane in [&mut tab.main, &mut tab.right] {
                if pane
                    .work
                    .view
                    .as_ref()
                    .is_some_and(|v| !v.route.is_current())
                    || pane
                        .work
                        .outcome
                        .as_ref()
                        .and_then(|o| o.route_ticket.as_ref())
                        .is_some_and(|t| !t.is_current())
                {
                    pane.work.view = None;
                    pane.work.outcome = None;
                }
            }
        }
    }

    pub fn release_work(&mut self) {
        for tab in &mut self.tabs {
            tab.main.work.invalidate();
            tab.right.work.invalidate();
        }
    }

    pub fn close_all(&mut self) {
        self.tabs.clear();
        self.selected = None;
    }
    pub fn remap(&mut self, ids: &BTreeMap<String, String>) {
        for tab in &mut self.tabs {
            if let Some(id) = tab.project.as_ref().and_then(|id| ids.get(id)) {
                tab.set_project(Some(id.clone()));
            }
        }
    }
    fn take_request(&mut self, request: &Request) -> Option<&mut Tab> {
        let tab = self.tabs.iter_mut().find(|t| t.id == request.tab)?;
        let work = &mut tab.pane_mut(request.side).work;
        if work.generation != request.generation {
            return None;
        }
        work.active.take()?;
        work.started = None;
        Some(tab)
    }

    pub fn retire(&mut self, request: Request) -> bool {
        if let Some(tab) = self.take_request(&request) {
            tab.pane_mut(request.side).work.view = None;
            true
        } else {
            false
        }
    }

    pub fn finish(&mut self, request: Request, outcome: Outcome) -> bool {
        let visible = self.active().is_some_and(|t| t.id == request.tab);
        let Some(tab) = self.take_request(&request) else {
            return false;
        };
        let correct_project = tab.project == request.project
            && request.project.as_deref() == Some(&outcome.project_id);
        let work = &mut tab.pane_mut(request.side).work;
        if !correct_project
            || !outcome
                .route_ticket
                .as_ref()
                .is_some_and(|ticket| ticket.is_current())
        {
            work.view = None;
            return false;
        }
        work.cancelled = matches!(outcome.status, Status::Cancelled);
        if let Status::Rows(data) = &outcome.status {
            // The request start clears empty_choice, so it can carry a newer view selection.
            let requested = work.empty_choice.take().unwrap_or(request.requested);
            let mut view = ResultView::new(
                data.clone(),
                outcome.route_ticket.as_ref().unwrap().clone(),
                work.lane.clone(),
            );
            view.generated = outcome.generated.clone();
            if request.side == Side::Main && requested == ViewChoice::Map {
                view.choose_main_map(view.key());
            } else {
                view.choose_available(requested, false, true);
            }
            if visible && view.choice() == ViewChoice::Grid {
                view.loader.resume();
            } else {
                view.loader.suspend();
            }
            work.view = Some(view);
        } else if matches!(outcome.status, Status::Plan(_)) {
            if let Some(view) = &mut work.view {
                view.sync_target();
                view.choose_available(ViewChoice::Explain, true, false);
            } else {
                work.empty_choice = Some(ViewChoice::Explain);
            }
        } else {
            work.view = None;
        }
        work.outcome = Some(outcome);
        true
    }
}
