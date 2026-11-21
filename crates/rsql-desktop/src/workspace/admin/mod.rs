use super::*;
use gpui_kit::{
    assets::IconName,
    component::{
        ActiveTheme, Disableable, Icon, Sizable,
        button::{Button, ButtonVariants},
        input::{Input, InputEvent, InputState},
    },
    prelude::FluentBuilder,
};
use model::{Filters, Section, Setting};
use read::{Query, Rows};
use rsql_core::postgres::{
    extensions::EnumType,
    roles_schema_objects::{DbGrant, PgRole, TableGrant},
};
use theme::controls::compact_button;
use uuid::Uuid;

mod entries;
pub(super) use entries::{PanelKey, name};
mod enums;
mod extensions;
mod grants;
mod model;
mod read;
mod roles;
mod settings;
mod view;

pub(super) fn tools() -> impl Iterator<Item = (Kind, &'static str)> {
    entries::TOOLS.into_iter().chain([
        (Kind::Monitor, "Performance Monitor"),
        (Kind::Notify, "LISTEN/NOTIFY"),
    ])
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Lane {
    Main,
    Available,
    Database,
    Table,
}

#[derive(Default)]
struct Grants {
    selected: Option<String>,
    database: Section,
    table: Section,
    databases: Vec<DbGrant>,
    tables: Vec<TableGrant>,
}

pub(super) struct Panel {
    tab: String,
    kind: Kind,
    database: String,
    owner: WeakEntity<Workspace>,
    session: Rc<Session>,
    capture: catalog::Capture,
    visible: bool,
    main: Section,
    rows: Option<Rows>,
    grants: Grants,
    extensions: extensions::Extensions,
    filters: Filters,
    filter_revision: u64,
    search: Entity<InputState>,
    settings_list: ListState,
    filtered_settings: Vec<usize>,
    _search: Subscription,
}

impl Panel {
    fn filter_changed(&mut self, cx: &mut Context<Self>) {
        self.filter_revision += 1;
        self.index_settings();
        cx.notify();
    }

    fn current(&self, cx: &App) -> bool {
        self.visible
            && self
                .owner
                .read_with(cx, |v, _| {
                    self.capture.current(v)
                        && v.session.documents.borrow().active().is_some_and(|t| {
                            t.id == self.tab
                                && t.kind == self.kind
                                && t.project.as_ref() == Some(&self.capture.project)
                        })
                })
                .unwrap_or(false)
    }

    fn section(&mut self, lane: Lane) -> &mut Section {
        match lane {
            Lane::Main => &mut self.main,
            Lane::Available => &mut self.extensions.available_section,
            Lane::Database => &mut self.grants.database,
            Lane::Table => &mut self.grants.table,
        }
    }

    fn main_query(&self) -> Query {
        match self.kind {
            Kind::Roles => Query::Roles,
            Kind::Enums => Query::Enums,
            Kind::Extensions => Query::Installed,
            Kind::PgSettings => Query::Settings,
            _ => unreachable!(),
        }
    }

    fn request(&mut self, lane: Lane, cx: &mut Context<Self>) {
        let query = match lane {
            Lane::Main => self.main_query(),
            Lane::Available => Query::Available,
            Lane::Database | Lane::Table => {
                let Some(name) = self.grants.selected.clone() else {
                    return;
                };
                if lane == Lane::Database {
                    Query::Database(name)
                } else {
                    Query::Table(name)
                }
            }
        };
        let (id, cancel) = self.section(lane).begin();
        let job = self.session.runtime.spawn(read::load(
            self.session.services.clone(),
            self.capture.clone(),
            query,
            cancel,
            self.session.preflight_stop.subscribe(),
        ));
        cx.spawn(async move |panel, cx| {
            let result = job.await.map_err(|e| e.to_string()).and_then(|r| r);
            let _ = panel.update(cx, |p, cx| p.publish(lane, id, result, cx));
        })
        .detach();
        cx.notify();
    }

    fn publish(
        &mut self,
        lane: Lane,
        id: Uuid,
        result: Result<Rows, String>,
        cx: &mut Context<Self>,
    ) {
        if !self.current(cx) || !self.section(lane).finish(id) {
            return;
        }
        match result {
            Err(e) => self.section(lane).error = Some(e),
            Ok(Rows::Installed(rows)) => self.extensions.installed = rows,
            Ok(Rows::Available(rows)) => self.extensions.available = rows,
            Ok(Rows::Database(rows)) => self.grants.databases = rows,
            Ok(Rows::Table(rows)) => self.grants.tables = rows,
            Ok(rows) => {
                if let Rows::Roles(roles) = &rows
                    && self
                        .grants
                        .selected
                        .as_ref()
                        .is_some_and(|name| !roles.iter().any(|r| &r.name == name))
                {
                    self.grants = Grants::default();
                }
                self.rows = Some(rows);
                self.index_settings();
            }
        }
        cx.notify();
    }

    fn lanes(&self) -> &'static [Lane] {
        match self.kind {
            Kind::Roles => &[Lane::Main, Lane::Database, Lane::Table],
            Kind::Extensions => &[Lane::Main, Lane::Available],
            _ => &[Lane::Main],
        }
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.current(cx) {
            for &lane in self.lanes() {
                self.request(lane, cx);
            }
        }
    }

    fn select_role(&mut self, name: String, cx: &mut Context<Self>) {
        if self.current(cx)
            && matches!(&self.rows, Some(Rows::Roles(roles)) if roles.iter().any(|r| r.name == name))
        {
            self.grants = Grants {
                selected: Some(name),
                ..Default::default()
            };
            self.request(Lane::Database, cx);
            self.request(Lane::Table, cx);
        }
    }

    fn visibility(&mut self, visible: bool, cx: &mut Context<Self>) {
        self.visible = visible;
        if !visible {
            self.extensions.cancel_write();
        }
        self.filter_revision += 1;
        for &lane in self.lanes() {
            let section = self.section(lane);
            if !visible {
                section.suspend();
            } else if !section.loaded && section.pending.is_none() {
                self.request(lane, cx);
            }
        }
    }
}
