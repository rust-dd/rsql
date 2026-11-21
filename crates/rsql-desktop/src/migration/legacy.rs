use crate::{
    connection::Connections,
    document::{Appearance, Documents, Kind, Pane, Side, Tab},
};
use rsql_core::migration::LegacyUi;
pub use rsql_core::migration::legacy::MigrationError;
use rsql_core::migration::legacy::{LegacyKind, LegacyTab, decode_legacy_tabs};

pub struct ImportedDocuments {
    pub documents: Documents,
    /// Count of malformed entries. Terminal and notification tabs are excluded by design.
    pub discarded_tabs: usize,
}

fn native_kind(kind: LegacyKind, schema: &mut Option<String>) -> Option<Kind> {
    Some(match kind {
        LegacyKind::Query => Kind::Query,
        LegacyKind::Terminal => Kind::Terminal,
        LegacyKind::Monitor => Kind::Monitor,
        LegacyKind::Erd => Kind::Erd(schema.take()?),
        LegacyKind::Notify => Kind::Notify,
        LegacyKind::Roles => Kind::Roles,
        LegacyKind::SchemaDiff => Kind::SchemaDiff,
        LegacyKind::Extensions => Kind::Extensions,
        LegacyKind::Enums => Kind::Enums,
        LegacyKind::PgSettings => Kind::PgSettings,
    })
}

fn native_tab(
    tab: LegacyTab,
    kind: Kind,
    schema: Option<String>,
    projects: &Connections,
) -> Result<Tab, MigrationError> {
    let fields = tab.fields;
    let project = fields
        .project_id
        .as_deref()
        .map(|id| {
            projects
                .resolve_id(id)
                .map(|profile| profile.id.clone())
                .ok_or(MigrationError::UnresolvedProject {
                    tab_number: tab.number,
                })
        })
        .transpose()?;
    let mut main = Pane::default();
    main.draft.text = fields.editor_value;
    let mut right = Pane::default();
    right.draft.text = fields.split_editor_value;
    Ok(Tab {
        id: fields.id,
        kind,
        title: fields.title,
        project,
        schema,
        timeout: fields.query_timeout,
        split: fields.is_split,
        focused_side: Side::default(),
        main,
        right,
    })
}

pub fn convert_legacy(
    ui: &LegacyUi,
    projects: &Connections,
) -> Result<ImportedDocuments, MigrationError> {
    let theme = if ui.theme.as_deref() == Some("dark") {
        Appearance::Dark
    } else {
        Appearance::Light
    };
    let Some(raw) = ui.tabs.as_deref() else {
        return Ok(ImportedDocuments {
            documents: Documents {
                theme,
                ..Documents::default()
            },
            discarded_tabs: 0,
        });
    };
    let state = decode_legacy_tabs(raw)?;
    let mut tabs = Vec::new();
    let mut selected = None;
    for (index, mut tab) in state.tabs.into_iter().enumerate() {
        let mut schema = tab.fields.schema.take();
        let kind =
            native_kind(tab.fields.kind, &mut schema).ok_or(MigrationError::InvalidLegacyTabs)?;
        if !kind.durable() {
            continue;
        }
        if index <= state.selected || selected.is_none() {
            selected = Some(tabs.len());
        }
        tabs.push(native_tab(tab, kind, schema, projects)?);
    }
    Ok(ImportedDocuments {
        documents: Documents {
            tabs,
            selected,
            theme,
        },
        discarded_tabs: state.discarded,
    })
}
