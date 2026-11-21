mod feedback;
mod filter;
mod reads;
mod row;
mod toolbar;

use super::{theme::controls::compact_button, *};
use crate::runtime::history::HistoryLog;
use filter::Form;
use gpui_kit::component::{
    ActiveTheme,
    button::ButtonVariants,
    input::{InputEvent, InputState},
};
use row::Row;
use rsql_desktop::{document::Tab, results::ViewChoice};
use tokio::sync::watch;

pub(super) struct Panel {
    session: Rc<Session>,
    workspace: WeakEntity<Workspace>,
    tab: String,
    pane_generation: u64,
    updates: watch::Receiver<HistoryLog>,
    disk_revision: u64,
    active: bool,
    search: Entity<InputState>,
    _input_events: Subscription,
    debounce: Option<Task<()>>,
    form: Form,
    generation: u64,
    list: ListState,
    rows: Vec<Row>,
    databases: Vec<String>,
    has_more: bool,
    page_task: Option<reads::Read>,
    databases_task: Option<reads::Read>,
    page_error: Option<String>,
    databases_error: Option<String>,
    confirming: bool,
    toasts: toast::Toasts,
    seen: feedback::Seen,
}

impl Workspace {
    pub(super) fn sync_history(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        let docs = self.session.documents.borrow();
        let active = docs
            .active()
            .filter(|t| t.main.choice() == ViewChoice::History);
        for ((tab, side), live) in &mut self.editors {
            if *side != Side::Main {
                continue;
            }
            let pane = docs.tabs.iter().find(|t| &t.id == tab).map(|t| &t.main);
            if let Some(panel) = &live.history
                && pane.is_none_or(|p| {
                    p.choice() != ViewChoice::History
                        || p.generation() != panel.read(cx).pane_generation
                })
            {
                panel.update(cx, |p, cx| p.sync(false, cx));
                live.history = None;
            }
            let visible = active.filter(|t| &t.id == tab);
            if let Some(current) = visible
                && live.history.is_none()
            {
                let workspace = cx.weak_entity();
                let generation = current.main.generation();
                live.history = Some(cx.new(|cx| {
                    Panel::new(
                        self.session.clone(),
                        self.toasts.clone(),
                        workspace,
                        tab.clone(),
                        generation,
                        w,
                        cx,
                    )
                }));
            }
            if let Some(panel) = &live.history {
                panel.update(cx, |p, cx| p.sync(visible.is_some(), cx));
            }
        }
    }

    pub(super) fn empty_result_segments(&self, tab: &Tab, cx: &Context<Self>) -> impl IntoElement {
        use super::results::toolbar::{segment, segments};
        let mut choices = segments(cx);
        let plan = tab.main.outcome().is_some_and(|outcome| {
            matches!(outcome.status, rsql_desktop::execution::Status::Plan(_))
        });
        for (label, icon, requested) in [
            ("Grid", None, ViewChoice::Grid),
            ("Record", None, ViewChoice::Record),
            (
                "Explain",
                Some(gpui_kit::assets::IconName::GitBranch),
                ViewChoice::Explain,
            ),
            (
                "History",
                Some(gpui_kit::assets::IconName::RotateCcwClock),
                ViewChoice::History,
            ),
        ] {
            if requested == ViewChoice::Explain && !plan {
                continue;
            }
            let id = tab.id.clone();
            let generation = tab.main.generation();
            let button = segment(
                label,
                icon,
                tab.main.choice() == requested,
                requested == ViewChoice::Record,
                cx,
            )
            .on_click(cx.listener(move |v, _: &ClickEvent, _, cx| {
                let mut docs = v.session.documents.borrow_mut();
                if let Some(t) = docs
                    .selected
                    .map(|i| &mut docs.tabs[i])
                    .filter(|t| t.id == id && t.main.generation() == generation)
                {
                    t.main.choose_empty(requested);
                    cx.notify();
                }
            }));
            choices = choices.child(button);
        }
        choices
    }
}

impl Panel {
    fn new(
        session: Rc<Session>,
        toasts: toast::Toasts,
        workspace: WeakEntity<Workspace>,
        tab: String,
        pane_generation: u64,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        session.refresh_history_status();
        let search = cx.new(|cx| InputState::new(w, cx).placeholder("Search SQL"));
        let events = cx.subscribe(&search, |p, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) && p.active {
                p.debounce.take();
                let text = input.read(cx).value().to_string();
                if p.form.text != text {
                    p.debounce = Some(cx.spawn(async move |panel, cx| {
                        cx.background_executor()
                            .timer(Duration::from_millis(200))
                            .await;
                        let _ = panel.update(cx, |p, cx| {
                            p.form.text = text;
                            p.reload(cx);
                        });
                    }));
                }
            }
        });
        let seen = feedback::Seen::of(&session.history());
        Self {
            updates: session.history_updates(),
            seen,
            session,
            workspace,
            tab,
            pane_generation,
            active: false,
            disk_revision: 0,
            search,
            _input_events: events,
            debounce: None,
            form: Default::default(),
            generation: 0,
            list: ListState::new(0, ListAlignment::Top, px(96.)),
            rows: vec![],
            databases: vec![],
            has_more: false,
            page_task: None,
            databases_task: None,
            page_error: None,
            databases_error: None,
            confirming: false,
            toasts,
        }
    }

    pub(super) fn suspend(&mut self) {
        if self.active {
            self.generation += 1;
            self.page_task = None;
            self.databases_task = None;
            self.debounce = None;
            self.confirming = false;
            self.active = false;
        }
    }

    fn sync(&mut self, visible: bool, cx: &mut Context<Self>) {
        if !visible {
            self.suspend();
            self.seen = feedback::Seen::of(&self.updates.borrow());
            return;
        }
        let project_removed = self.form.project.as_ref().is_some_and(|project| {
            !self
                .session
                .connections
                .borrow()
                .profiles
                .iter()
                .any(|p| &p.id == project)
        });
        if project_removed {
            self.form.project = None;
            self.form.database = None;
        }
        let changed = self.updates.has_changed().unwrap_or(false);
        let revision = self.updates.borrow_and_update().revision;
        self.report_log(cx);
        if !self.active || self.disk_revision != revision || project_removed {
            if !self.active {
                self.form.text = self.search.read(cx).value().to_string();
            }
            self.active = true;
            self.reload(cx);
        } else if changed {
            cx.notify();
        }
    }

    fn current(&self, generation: u64) -> bool {
        self.active
            && self.generation == generation
            && self.disk_revision == self.session.history().revision
            && self.owns_pane()
    }

    fn owns_pane(&self) -> bool {
        !*self.session.preflight_stop.borrow()
            && self.session.documents.borrow().active().is_some_and(|tab| {
                tab.id == self.tab
                    && tab.main.generation() == self.pane_generation
                    && tab.main.choice() == ViewChoice::History
            })
    }

    fn owns_idle_pane(&self) -> bool {
        self.owns_pane()
            && self
                .session
                .documents
                .borrow()
                .active()
                .is_some_and(|tab| !tab.main.is_busy())
    }
}

impl Render for Panel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        use gpui_kit::component::button::Button;
        let mut panel = div()
            .id("history-panel")
            .test_support()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .text_size(px(12.))
            .child(self.toolbar(cx));
        let count = self.rows.len();
        let view = cx.entity().downgrade();
        let generation = self.generation;
        if count > 0 {
            panel = panel.child(
                div()
                    .id("history-rows")
                    .w_full()
                    .min_w_0()
                    .flex_1()
                    .min_h_0()
                    .child(
                        list(self.list.clone(), move |index, _, cx| {
                            let Some(panel) = view.upgrade() else {
                                return div().into_any_element();
                            };
                            let model = panel.read(cx);
                            model.rows.get(index).map_or_else(
                                || div().into_any_element(),
                                |row| model.row(row, generation, &panel, cx),
                            )
                        })
                        .size_full(),
                    ),
            );
        }
        if self.page_task.is_some() {
            panel = panel.child("Loading history…");
        } else if let Some(error) = &self.page_error {
            panel = panel
                .child(
                    div()
                        .text_color(cx.theme().danger)
                        .child(format!("Could not load history: {error}")),
                )
                .child(compact_button(
                    Button::new("history-page-retry")
                        .label("Retry loading history")
                        .on_click(cx.listener(|p, _, _, cx| {
                            p.load_page(p.rows.last().map(|r| r.entry.id), cx)
                        })),
                ));
        } else if self.rows.is_empty() {
            panel = panel.child(div().p_4().text_color(cx.theme().muted_foreground).child(
                if self.form == Form::default() {
                    "No saved queries yet"
                } else {
                    "No queries match these filters"
                },
            ));
        } else if self.has_more {
            panel = panel.child(compact_button(
                Button::new("history-more")
                    .label("Load more")
                    .on_click(cx.listener(move |p, _, _, cx| {
                        if p.current(generation) && p.has_more && p.page_task.is_none() {
                            p.load_page(p.rows.last().map(|r| r.entry.id), cx);
                        }
                    })),
            ));
        }
        panel
    }
}
