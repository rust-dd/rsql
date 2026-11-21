mod cells;
mod entries;
mod model;
mod pages;
mod read;
mod tables;
mod view;

use super::*;
use model::*;
use tokio::sync::watch;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Page {
    #[default]
    Overview,
    Activity,
    Tables,
    History,
    Locks,
    Indexes,
    Bloat,
}

impl Page {
    fn lane(self) -> Option<Lane> {
        Some(match self {
            Self::Overview => Lane::Stats,
            Self::Activity => Lane::Activity,
            Self::Tables => Lane::Tables,
            Self::History => return None,
            Self::Locks => Lane::Locks,
            Self::Indexes => Lane::Indexes,
            Self::Bloat => Lane::Bloat,
        })
    }
}

struct Cycle {
    cancel: watch::Sender<bool>,
    remaining: usize,
}

pub(super) struct Panel {
    session: Rc<Session>,
    owner: WeakEntity<Workspace>,
    capture: catalog::Capture,
    tab: String,
    database: String,
    pub(super) focus: FocusHandle,
    page: Page,
    snapshot: MonitorSnapshot,
    inspection: Option<usize>,
    auto: bool,
    visible: bool,
    pub(super) retired: bool,
    revision: u64,
    pending: Option<Cycle>,
    timer: Option<Task<()>>,
    _history: Task<()>,
}

impl Panel {
    fn current(&self) -> bool {
        self.visible
            && !self.retired
            && self.capture.route.is_current()
            && matches!(self.capture.generation.has_changed(), Ok(false))
            && !*self.session.preflight_stop.borrow()
            && self.session.documents.borrow().active().is_some_and(|t| {
                t.id == self.tab
                    && t.kind == Kind::Monitor
                    && t.project.as_ref() == Some(&self.capture.project)
            })
    }

    fn visibility(&mut self, visible: bool, cx: &mut Context<Self>) {
        if visible == self.visible {
            return;
        }
        self.visible = visible;
        self.timer = None;
        if visible {
            self.refresh(cx);
        } else if let Some(cycle) = &self.pending {
            cycle.cancel.send_replace(true);
        }
        cx.notify();
    }

    fn arm_timer(&mut self, cx: &mut Context<Self>) {
        self.timer = None;
        if self.auto && self.current() && self.pending.is_none() {
            self.timer = Some(cx.spawn(async move |panel, cx| {
                cx.background_executor().timer(Duration::from_secs(5)).await;
                let _ = panel.update(cx, |p, cx| p.refresh(cx));
            }));
        }
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.request(&Lane::ALL, cx);
    }

    fn request(&mut self, lanes: &[Lane], cx: &mut Context<Self>) {
        if !self.current() || self.pending.is_some() {
            return;
        }
        self.timer = None;
        self.revision += 1;
        let revision = self.revision;
        let (cancel, rx) = watch::channel(false);
        self.pending = Some(Cycle {
            cancel,
            remaining: lanes.len(),
        });
        for &lane in lanes {
            self.snapshot.sections[lane as usize].loading = true;
            let capture = self.capture.clone();
            let services = self.session.services.clone();
            let rx = rx.clone();
            let stop = self.session.preflight_stop.subscribe();
            let job = self
                .session
                .runtime
                .spawn(async move { read::load(&services, &capture, lane, rx, stop).await });
            cx.spawn(async move |panel, cx| {
                let result = job.await.map_err(|e| e.to_string()).and_then(|r| r);
                let _ = panel.update(cx, |p, cx| p.settle(revision, lane, result, cx));
            })
            .detach();
        }
        cx.notify();
    }

    fn settle(
        &mut self,
        revision: u64,
        lane: Lane,
        result: Result<Rows, String>,
        cx: &mut Context<Self>,
    ) {
        if revision != self.revision {
            return;
        }
        let current = self.current();
        let Some(cycle) = &mut self.pending else {
            return;
        };
        let cancelled = *cycle.cancel.borrow();
        let section = &mut self.snapshot.sections[lane as usize];
        if current && !cancelled {
            if result.is_ok() && self.page.lane() == Some(lane) {
                self.inspection = None;
            }
            section.settle(result);
        } else {
            section.loading = false;
        }
        cycle.remaining -= 1;
        if cycle.remaining == 0 {
            self.pending = None;
            if self.retired || !matches!(self.capture.generation.has_changed(), Ok(false)) {
                let _ = self.owner.update(cx, |_, cx| cx.notify());
            } else if cancelled && current {
                self.refresh(cx);
            } else {
                self.arm_timer(cx);
            }
        }
        cx.notify();
    }
}
