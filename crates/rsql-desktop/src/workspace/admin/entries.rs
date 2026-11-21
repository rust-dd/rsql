use super::*;

pub(in crate::workspace) const TOOLS: [(Kind, &str); 4] = [
    (Kind::Roles, "Roles"),
    (Kind::Extensions, "Extensions"),
    (Kind::Enums, "Enum Types"),
    (Kind::PgSettings, "PostgreSQL Settings"),
];

pub(in crate::workspace) fn name(kind: &Kind) -> Option<&'static str> {
    TOOLS
        .into_iter()
        .find(|(k, _)| k == kind)
        .map(|(_, label)| label)
}

#[derive(Hash, PartialEq, Eq)]
pub(in crate::workspace) enum PanelKey {
    Document(String),
    Finishing(Uuid),
}

impl Workspace {
    pub(in crate::workspace) fn admin_panel(&self, id: &str) -> Option<&Entity<Panel>> {
        self.admin_panels.get(&PanelKey::Document(id.to_owned()))
    }

    pub(in crate::workspace) fn open_admin(
        &mut self,
        kind: Kind,
        capture: &catalog::Capture,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if capture.current(self) {
            self.update_documents(w, cx, |d| {
                d.open(kind, Some(capture.project.clone()));
            });
        }
    }

    pub(in crate::workspace) fn sync_admin_panels(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let docs = self.session.documents.borrow();
        let active = docs.active();
        let mut obsolete = Vec::new();
        for (key, panel) in &self.admin_panels {
            let PanelKey::Document(id) = key else {
                continue;
            };
            let p = panel.read(cx);
            let attached = docs.tabs.iter().any(|t| {
                &t.id == id && t.kind == p.kind && t.project.as_ref() == Some(&p.capture.project)
            });
            let current = p.capture.current(self);
            let renewed = (attached && !current && p.kind == Kind::Extensions)
                .then(|| catalog::Capture::new(self, &p.capture.project))
                .flatten()
                .filter(|c| c.current(self) && c.route.same_generation(&p.capture.route));
            let keep = attached && (current || renewed.is_some());
            let visible = keep && active.is_some_and(|t| &t.id == id);
            if (!visible || !current) && p.reviewing_extension() {
                panel.update(cx, |p, _| p.extensions.dismiss_review());
                window.close_dialog(cx);
            }
            if let Some(capture) = renewed {
                panel.update(cx, |p, cx| {
                    p.capture = capture;
                    p.extensions.cancel_write();
                    p.main = Section::default();
                    p.extensions.available_section = Section::default();
                    p.visibility(visible, cx);
                });
            } else if panel.read(cx).visible != visible {
                panel.update(cx, |p, cx| p.visibility(visible, cx));
            }
            if !keep {
                obsolete.push(id.clone());
            }
        }
        for id in obsolete {
            if let Some(panel) = self.admin_panels.remove(&PanelKey::Document(id)) {
                let running = panel.update(cx, |p, cx| {
                    let running = p
                        .extensions
                        .running
                        .take()
                        .filter(|_| p.capture.route.is_current());
                    p.main = Section::default();
                    p.rows = None;
                    p.extensions = Default::default();
                    p.filters = Filters::default();
                    if !p.search.read(cx).value().is_empty() {
                        p.search.update(cx, |s, cx| s.set_value("", window, cx));
                    }
                    p.index_settings();
                    p.grants = Grants::default();
                    let id = running.as_ref().map(|(id, _)| *id);
                    p.extensions.running = running;
                    id
                });
                if let Some(id) = running {
                    self.admin_panels.insert(PanelKey::Finishing(id), panel);
                }
            }
        }
        let Some(t) = active.filter(|t| name(&t.kind).is_some()) else {
            return;
        };
        if self.admin_panel(&t.id).is_some() {
            return;
        }
        let Some(project) = &t.project else { return };
        let Some(capture) = catalog::Capture::new(self, project).filter(|c| c.current(self)) else {
            return;
        };
        let finishing = self.admin_panels.iter().find_map(|(key, panel)| match key {
            PanelKey::Finishing(id)
                if t.kind == Kind::Extensions
                    && panel.read(cx).capture.route.same_generation(&capture.route) =>
            {
                Some(*id)
            }
            _ => None,
        });
        let previous = finishing
            .and_then(|id| self.admin_panels.remove(&PanelKey::Finishing(id)))
            .and_then(|p| p.update(cx, |p, _| p.extensions.running.take()));
        let database = self
            .session
            .connections
            .borrow()
            .profiles
            .iter()
            .find(|p| &p.id == project)
            .map_or_else(|| project.clone(), |p| p.database.clone());
        let owner = cx.weak_entity();
        let panel = cx.new(|cx| {
            let search = cx.new(|cx| {
                InputState::new(window, cx).placeholder(
                    if matches!(t.kind, Kind::Enums | Kind::Extensions) {
                        "Filter…"
                    } else {
                        "Search settings…"
                    },
                )
            });
            let subscription =
                cx.subscribe(&search, |p: &mut Panel, input, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) && p.current(cx) {
                        p.filters.text = input.read(cx).value().to_lowercase();
                        p.filter_changed(cx);
                    }
                });
            let settings_list = ListState::new(0, ListAlignment::Top, px(100.));
            let mut panel = Panel {
                tab: t.id.clone(),
                kind: t.kind.clone(),
                database,
                owner,
                capture,
                session: self.session.clone(),
                visible: false,
                main: Default::default(),
                rows: None,
                grants: Default::default(),
                extensions: Default::default(),
                filters: Default::default(),
                search,
                filter_revision: 0,
                settings_list,
                filtered_settings: Vec::new(),
                _search: subscription,
            };
            if let Some(running) = previous {
                panel.extensions.running = Some(running);
                panel.extensions.wait_for_delivery();
            }
            panel.visibility(true, cx);
            panel
        });
        self.admin_panels
            .insert(PanelKey::Document(t.id.clone()), panel);
    }
}
