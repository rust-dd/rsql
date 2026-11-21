use super::*;
use gpui_kit::component::input::{InputEvent, InputState};
use rsql_core::{ApplicationServices, services::ProjectTicket};
use std::{collections::BTreeMap, sync::Arc};
use tokio::sync::watch;
use uuid::Uuid;

mod actions;
mod listen;
mod log;
mod view;

struct Channel {
    id: Uuid,
    active: bool,
    stop: watch::Sender<bool>,
}

pub(super) struct Panel {
    tab: String,
    project: String,
    route: ProjectTicket,
    session: Rc<Session>,
    retired: watch::Sender<bool>,
    channel: Entity<InputState>,
    target: Entity<InputState>,
    payload: Entity<InputState>,
    _inputs: Vec<Subscription>,
    subscribe_revision: u64,
    send_revision: u64,
    sending: Option<Uuid>,
    discovering: Option<Uuid>,
    known: Vec<String>,
    channels: BTreeMap<String, Channel>,
    discovery_error: Option<String>,
    subscribe_error: Option<String>,
    send_error: Option<String>,
    log: log::Log,
    list: ListState,
}

impl Workspace {
    pub(super) fn sync_notify_panels(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let docs = self.session.documents.borrow();
        self.notify_panels.retain(|id, panel| {
            let p = panel.read(cx);
            let attached = docs.tabs.iter().any(|t| {
                &t.id == id && t.kind == Kind::Notify && t.project.as_ref() == Some(&p.project)
            });
            let keep = attached
                && p.route.is_current()
                && !*self.session.preflight_stop.borrow()
                && self
                    .navigation
                    .connected_routes
                    .get(&p.project)
                    .is_some_and(|r| r.same_generation(&p.route));
            if !keep {
                panel.update(cx, |p, _| {
                    p.retired.send_replace(true);
                    p.channels.clear();
                    p.log.clear();
                    p.list.reset(0);
                });
            }
            keep
        });
        let Some(tab) = docs
            .active()
            .filter(|t| t.kind == Kind::Notify && !self.notify_panels.contains_key(&t.id))
        else {
            return;
        };
        let Some(project) = tab.project.as_ref() else {
            return;
        };
        let Some(route) = self
            .navigation
            .connected_routes
            .get(project)
            .filter(|r| r.is_current() && !*self.session.preflight_stop.borrow())
        else {
            return;
        };
        self.notify_panels.insert(
            tab.id.clone(),
            cx.new(|cx| {
                Panel::new(
                    tab.id.clone(),
                    project.clone(),
                    route.clone(),
                    self.session.clone(),
                    window,
                    cx,
                )
            }),
        );
    }
}

impl Panel {
    fn new(
        tab: String,
        project: String,
        route: ProjectTicket,
        session: Rc<Session>,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let channel = cx.new(|cx| InputState::new(w, cx).placeholder("Channel name..."));
        let target = cx.new(|cx| InputState::new(w, cx).placeholder("Channel"));
        let payload = cx.new(|cx| InputState::new(w, cx).placeholder("Payload..."));
        let inputs = vec![
            cx.subscribe_in(&channel, w, |p: &mut Self, _, e, w, cx| match e {
                InputEvent::Change => p.subscribe_revision += 1,
                InputEvent::PressEnter { .. } => p.subscribe_manual(w, cx),
                _ => (),
            }),
            cx.subscribe(&target, |p, _, e, _| {
                if matches!(e, InputEvent::Change) {
                    p.send_revision += 1;
                }
            }),
            cx.subscribe_in(&payload, w, |p, _, e, w, cx| match e {
                InputEvent::Change => p.send_revision += 1,
                InputEvent::PressEnter { .. } => p.send(w, cx),
                _ => (),
            }),
        ];
        let mut panel = Self {
            tab,
            project,
            route,
            session,
            retired: watch::channel(false).0,
            channel,
            target,
            payload,
            _inputs: inputs,
            subscribe_revision: 0,
            send_revision: 0,
            sending: None,
            discovering: None,
            known: Vec::new(),
            channels: Default::default(),
            discovery_error: None,
            subscribe_error: None,
            send_error: None,
            log: Default::default(),
            list: ListState::new(0, ListAlignment::Top, px(128.)),
        };
        panel.discover(cx);
        panel
    }

    fn attached(&self) -> bool {
        !*self.retired.borrow()
            && !*self.session.preflight_stop.borrow()
            && self.route.is_current()
            && self.session.documents.borrow().tabs.iter().any(|t| {
                t.id == self.tab
                    && t.kind == Kind::Notify
                    && t.project.as_ref() == Some(&self.project)
            })
    }

    fn selected(&self) -> bool {
        self.attached()
            && self
                .session
                .documents
                .borrow()
                .active()
                .is_some_and(|t| t.id == self.tab)
    }

    fn lifetime(&self) -> Lifetime {
        Lifetime {
            route: self.route.clone(),
            owner: self.retired.subscribe(),
            shutdown: self.session.preflight_stop.subscribe(),
        }
    }
}

struct Lifetime {
    route: ProjectTicket,
    owner: watch::Receiver<bool>,
    shutdown: watch::Receiver<bool>,
}

impl Lifetime {
    async fn ended(&mut self) {
        tokio::select! {
            _ = self.owner.wait_for(|v| *v) => (),
            _ = self.shutdown.wait_for(|v| *v) => (),
            _ = self.route.retired() => (),
        }
    }
}
