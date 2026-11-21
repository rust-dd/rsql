use super::*;

impl Workspace {
    pub(in crate::workspace) fn sync_monitors(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        let docs = self.session.documents.borrow();
        let active = docs.active();
        let mut monitors = std::mem::take(&mut self.monitors);
        monitors.retain(|id, entity| {
            let p = entity.read(cx);
            let keep = !p.retired
                && p.capture.current_route(self)
                && self.navigation.catalogs.contains_key(&p.capture.project)
                && docs.tabs.iter().any(|t| {
                    &t.id == id
                        && t.kind == Kind::Monitor
                        && t.project.as_ref() == Some(&p.capture.project)
                });
            if keep && !p.capture.current(self) {
                let project = p.capture.project.clone();
                if p.visible {
                    entity.update(cx, |p, cx| p.visibility(false, cx));
                }
                if entity.read(cx).pending.is_some() {
                    return true;
                }
                let Some(capture) = catalog::Capture::new(self, &project) else {
                    return false;
                };
                entity.update(cx, |p, _| p.capture = capture);
            }
            let p = entity.read(cx);
            let visible = keep && w.is_visible() && active.is_some_and(|t| &t.id == id);
            if visible != p.visible || (!keep && !p.retired) {
                entity.update(cx, |p, cx| {
                    if !keep {
                        p.retired = true;
                        p.snapshot = MonitorSnapshot::default();
                    }
                    p.visibility(visible, cx);
                });
            }
            keep || entity.read(cx).pending.is_some()
        });
        self.monitors = monitors;
        let Some(tab) =
            active.filter(|t| t.kind == Kind::Monitor && !self.monitors.contains_key(&t.id))
        else {
            return;
        };
        let Some(project) = &tab.project else { return };
        let Some(capture) = catalog::Capture::new(self, project).filter(|c| c.current(self)) else {
            return;
        };
        let owner = cx.weak_entity();
        let database = self
            .session
            .connections
            .borrow()
            .profiles
            .iter()
            .find(|p| &p.id == project)
            .map_or_else(|| project.clone(), |p| p.database.clone());
        let entity = cx.new(|cx| {
            let mut history = self.session.history_updates();
            let updates = cx.spawn(async move |panel, cx| {
                while history.changed().await.is_ok() {
                    if panel
                        .update(cx, |p: &mut Panel, cx| {
                            if p.visible && matches!(p.page, Page::Overview | Page::History) {
                                cx.notify();
                            }
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            });
            let mut p = Panel {
                session: self.session.clone(),
                owner,
                capture,
                tab: tab.id.clone(),
                database,
                focus: cx.focus_handle(),
                page: Page::Overview,
                snapshot: Default::default(),
                inspection: None,
                auto: false,
                visible: false,
                retired: false,
                revision: 0,
                pending: None,
                timer: None,
                _history: updates,
            };
            p.visibility(w.is_visible(), cx);
            if p.visible {
                p.focus.focus(w, cx);
            }
            p
        });
        self.monitors.insert(tab.id.clone(), entity);
    }
}
