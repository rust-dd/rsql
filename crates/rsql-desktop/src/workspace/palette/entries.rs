use super::*;
use gpui_kit::component::ActiveTheme;

fn matches_search(filter: &str, fields: &[&str]) -> bool {
    let fields = fields
        .iter()
        .map(|field| field.to_lowercase())
        .collect::<Vec<_>>();
    filter
        .split_whitespace()
        .all(|word| fields.iter().any(|field| field.contains(word)))
}

impl Workspace {
    pub(super) fn entries(&self, p: &Palette, cx: &App) -> Vec<(String, Command)> {
        use Command::*;
        let mut rows = Vec::new();
        let filter = p.search.read(cx).value().trim().to_lowercase();
        let root = p.page == super::Page::Root;
        if root {
            if !self.navigation.busy {
                rows.push(("New Connection".into(), Create));
            }
            rows.extend([
                ("New Query Tab".into(), NewQuery),
                ("Open Terminal".into(), Terminal),
            ]);
            let docs = self.session.documents.borrow();
            if let Some(tab) = docs
                .tabs
                .iter()
                .find(|t| Some(&t.id) == p.tab.as_ref() && t.kind == Kind::Query)
            {
                let current = tab.project == p.project
                    && self.profile_for(&tab.id) == p.profile
                    && p.ticket.as_ref().is_none_or(|t| t.is_current());
                let mut can_cancel = tab.main.is_busy() || tab.right.is_busy();
                if current && p.profile.is_some() {
                    for (label, action) in [
                        ("Run Selection or Statement", execution::Action::Run),
                        ("Run Script", execution::Action::Script),
                        ("Explain Query", execution::Action::Explain),
                        ("Explain Analyze Query", execution::Action::Analyze),
                    ] {
                        let side = execution_side(action, p.side);
                        if (side == Side::Main || tab.split)
                            && !rsql_desktop::sql::is_blank(&tab.pane(side).draft.text)
                        {
                            rows.push((label.into(), Execute(action)));
                            can_cancel = true;
                        }
                    }
                }
                if can_cancel {
                    rows.push(("Cancel Query".into(), Cancel));
                }
                if current && !rsql_desktop::sql::is_blank(&tab.main.draft.text) {
                    rows.push(("Format SQL".into(), Format));
                    if p.profile.is_some() {
                        rows.push(("Save Query".into(), Save));
                    }
                }
                if let Some(key) = p.pin_target
                    && tab
                        .main
                        .view()
                        .is_some_and(|v| v.key() == key && v.allows(ResultAction::Pin))
                {
                    rows.push(("Pin Current Result".into(), Pin(key)));
                }
            }
            rows.push((
                if cx.theme().is_dark() {
                    "Light Mode"
                } else {
                    "Dark Mode"
                }
                .into(),
                Theme,
            ));
            rows.push(("Check for Updates".into(), Updates));
            if let Some(id) = p.clear_target
                && self.can_clear_pin(id)
            {
                rows.push(("Clear Pinned Result".into(), ClearPin(id)));
            }
            if let Some(capture) = &p.schema_capture
                && capture.current(self)
            {
                rows.extend(
                    admin::tools()
                        .map(|(kind, label)| (label.into(), Admin(kind, capture.clone()))),
                );
                rows.push(("Schema Diff".into(), Compare(capture.clone())));
            }
            if p.tab.is_some() {
                rows.push(("Close tab".into(), Close));
            }
            if docs
                .tabs
                .iter()
                .any(|tab| Some(&tab.id) == p.tab.as_ref() && tab.kind == Kind::Query)
            {
                rows.push(("Toggle split".into(), Split));
            }
        }
        self.workspace_entries(p.page, &mut rows);
        if p.page == super::Page::Connections || root {
            if !root && !self.navigation.busy {
                rows.push(("New Connection".into(), Create));
            }
            let profiles = self.session.connections.borrow();
            for profile in &profiles.profiles {
                rows.push((
                    profile.display_name().to_owned(),
                    Connection(Rc::new(profile.clone())),
                ));
            }
            for profile in &profiles.profiles {
                if self
                    .navigation
                    .status
                    .get(&profile.id)
                    .is_some_and(|status| status == "Connected")
                    && let Some(capture) = catalog::Capture::new(self, &profile.id)
                {
                    rows.push((
                        format!("Monitor {}", profile.display_name()),
                        Monitor(Rc::new(capture)),
                    ));
                }
            }
            for profile in &profiles.profiles {
                let name = profile.display_name();
                rows.push((format!("Use {name}"), Choose(profile.id.clone())));
                if !self.navigation.busy
                    && self
                        .navigation
                        .status
                        .get(&profile.id)
                        .is_some_and(|s| s == "Connected")
                {
                    rows.push((format!("Disconnect {name}"), Disconnect(profile.id.clone())));
                }
            }
            if root {
                rows.push(("Connections…".into(), Page(super::Page::Connections)));
            }
        }
        if p.page == super::Page::Schema || root {
            for profile in &self.session.connections.borrow().profiles {
                if let Some(catalog) = self.navigation.catalogs.get(&profile.id)
                    && let Some(capture) = catalog::Capture::new(self, &profile.id)
                {
                    let capture = Rc::new(capture);
                    if let Ok(Some(schemas)) = catalog.schema_names() {
                        rows.extend(schemas.iter().map(|schema| {
                            (
                                format!("ERD {}/{schema}", profile.display_name()),
                                Erd(capture.clone(), schema.clone()),
                            )
                        }));
                    }
                    rows.extend(catalog.drafts(&profile.id).into_iter().map(|draft| {
                        (
                            draft.object.search_fields().0.to_owned(),
                            Draft(capture.clone(), draft),
                        )
                    }));
                }
            }
            if root {
                rows.push(("Schema objects…".into(), Page(super::Page::Schema)));
            }
        }
        if p.page == super::Page::Saved || root {
            let profiles = self.session.connections.borrow();
            rows.extend(self.session.saved_queries().rows.keys().map(|key| {
                let label = saved::connection_label(key, &profiles);
                (format!("{} · {label}", key.title), OpenSaved(key.clone()))
            }));
            if root {
                rows.push(("Saved Queries…".into(), Page(super::Page::Saved)));
            }
        }
        let profiles = self.session.connections.borrow();
        rows.retain(|(label, command)| {
            label.to_lowercase().contains(&filter)
                || match command {
                    Theme => matches_search(&filter, &[label, "toggle theme dark light"]),
                    Execute(execution::Action::Run) => {
                        matches_search(&filter, &[label, "execute query"])
                    }
                    Execute(execution::Action::Script) => {
                        matches_search(&filter, &[label, "execute all"])
                    }
                    Execute(execution::Action::Explain) => {
                        matches_search(&filter, &[label, "main statement"])
                    }
                    Execute(execution::Action::Analyze) => {
                        matches_search(&filter, &[label, query_toolbar::ANALYZE_HINT])
                    }
                    Connection(profile) => matches_search(
                        &filter,
                        &[
                            label,
                            &profile.database,
                            &profile.host,
                            &profile.id,
                            "connection connect",
                        ],
                    ),
                    Monitor(capture) => {
                        matches_search(&filter, &[label, &capture.project, "performance monitor"])
                    }
                    Erd(capture, schema) => {
                        matches_search(&filter, &[label, &capture.project, schema, "ERD diagram"])
                    }
                    Draft(capture, draft) => {
                        let (name, schema, kind) = draft.object.search_fields();
                        let profile = profiles.profiles.iter().find(|p| p.id == capture.project);
                        let signature_match = draft.signature.as_ref().is_some_and(|signature| {
                            signature.to_lowercase().contains(&filter)
                                || format!("{schema}.{signature}")
                                    .to_lowercase()
                                    .contains(&filter)
                        });
                        signature_match
                            || matches_search(
                                &filter,
                                &[
                                    name,
                                    schema,
                                    kind,
                                    &format!("{schema}.{name}"),
                                    &capture.project,
                                    profile.map_or("", ConnectionProfile::display_name),
                                    draft.detail.as_deref().unwrap_or_default(),
                                ],
                            )
                    }
                    OpenSaved(key) => {
                        matches_search(&filter, &[label, &key.project, &key.database, &key.driver])
                    }
                    _ => false,
                }
        });
        if root {
            rows.sort_by_cached_key(|(_, command)| presentation::group(command));
        }
        rows
    }
}
