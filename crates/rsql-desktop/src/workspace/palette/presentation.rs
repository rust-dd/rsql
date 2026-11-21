use super::*;
use gpui_kit::{
    assets::IconName,
    component::{ActiveTheme, Icon},
    prelude::FluentBuilder,
};
use rsql_desktop::metadata::ObjectKind;

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Group {
    Actions,
    Connections,
    Workspaces,
    Saved,
    Objects,
    Schema(String, String, &'static str),
}

impl Group {
    pub(super) fn label(&self) -> String {
        match self {
            Self::Actions => "Actions".into(),
            Self::Connections => "Connections".into(),
            Self::Workspaces => "Workspaces".into(),
            Self::Saved => "Saved Queries".into(),
            Self::Objects => "Database Objects".into(),
            Self::Schema(project, schema, kind) if schema.is_empty() => format!("{project} {kind}"),
            Self::Schema(project, schema, kind) => {
                format!("{project} / {schema} {kind}").trim_end().to_owned()
            }
        }
    }

    pub(super) fn id(&self) -> ElementId {
        match self {
            Self::Actions => "palette-actions-heading".into(),
            Self::Connections => "palette-connections-heading".into(),
            Self::Workspaces => "palette-workspaces-heading".into(),
            Self::Saved => "palette-saved-heading".into(),
            Self::Objects => "palette-objects-heading".into(),
            Self::Schema(..) => SharedString::from(self.label()).into(),
        }
    }
}

pub(super) fn group(command: &Command) -> Group {
    use catalog::DraftObject;
    match command {
        Command::Page(Page::Connections)
        | Command::Choose(_)
        | Command::Connection(_)
        | Command::Monitor(_)
        | Command::Disconnect(_) => Group::Connections,
        Command::Page(Page::SaveWorkspace | Page::LoadWorkspace | Page::DeleteWorkspace)
        | Command::Named(_) => Group::Workspaces,
        Command::Page(Page::Saved) | Command::OpenSaved(_) => Group::Saved,
        Command::Page(Page::Schema) | Command::Erd(..) => Group::Objects,
        Command::Draft(capture, object) => {
            let (schema, kind) = match &object.object {
                DraftObject::Schema(_) => ("", "Schemas"),
                DraftObject::Relation(object) => (
                    object.schema.as_str(),
                    match object.kind {
                        ObjectKind::View => "Views",
                        ObjectKind::MaterializedView => "Mat. Views",
                        _ => "",
                    },
                ),
                DraftObject::Function(object) => (object.schema.as_str(), "Functions"),
            };
            Group::Schema(capture.project.clone(), schema.to_owned(), kind)
        }
        _ => Group::Actions,
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(super) enum Row {
    Heading(Group),
    Choice(usize),
}

impl Row {
    pub(super) fn height(&self, page: Page) -> f32 {
        match self {
            Self::Heading(_) => 27.,
            Self::Choice(_) if page.workspace_list() => 52.,
            Self::Choice(_) => 36.,
        }
    }
}

pub(super) fn rows(page: Page, entries: &[(String, Command)]) -> Vec<Row> {
    let mut rows = Vec::new();
    let mut previous = None;
    for (index, (_, command)) in entries.iter().enumerate() {
        if page == Page::Root {
            let group = group(command);
            if previous.as_ref() != Some(&group) {
                previous = Some(group.clone());
                rows.push(Row::Heading(group));
            }
        }
        rows.push(Row::Choice(index));
    }
    rows
}

fn modifier() -> &'static str {
    if cfg!(target_os = "macos") {
        "⌘"
    } else {
        "Ctrl"
    }
}

pub(super) fn content(
    label: String,
    command: &Command,
    profile: Option<(String, bool)>,
    cx: &App,
) -> Div {
    let theme = cx.theme();
    let (icon, detail, badge) = match command {
        Command::Page(Page::Connections) | Command::Choose(_) | Command::Connection(_) => {
            (IconName::Database, None, Some("Connection"))
        }
        Command::Disconnect(_) => (IconName::Link2Off, None, Some("Disconnect")),
        Command::Monitor(_) => (IconName::Activity, None, Some("Performance")),
        Command::Create => (IconName::Plus, None, Some("Connection")),
        Command::NewQuery => (IconName::Plus, None, None),
        Command::Terminal => (
            IconName::Terminal,
            Some(terminal_tabs::SHORTCUT.into()),
            None,
        ),
        Command::Admin(kind, _) => (
            match kind {
                Kind::Monitor => IconName::Activity,
                Kind::Roles => IconName::Shield,
                Kind::PgSettings => IconName::Settings,
                Kind::Notify => IconName::Zap,
                Kind::Extensions => IconName::Package,
                _ => IconName::List,
            },
            None,
            None,
        ),
        Command::Compare(_) => (IconName::Columns3, None, None),
        Command::Erd(..) => (IconName::Network, None, Some("Diagram")),
        Command::Draft(_, draft) => {
            let (icon, badge) = match &draft.object {
                catalog::DraftObject::Schema(_) => (IconName::Database, "Schema"),
                catalog::DraftObject::Function(_) => (IconName::FileCode, "Function"),
                catalog::DraftObject::Relation(object) => match object.kind {
                    ObjectKind::View => (IconName::Eye, "View"),
                    ObjectKind::MaterializedView => (IconName::Layers, "Mat. View"),
                    _ => (IconName::Table, "Table"),
                },
            };
            (icon, draft.detail.clone(), Some(badge))
        }
        Command::Execute(action) => match action {
            execution::Action::Run => (IconName::Play, Some(format!("{}+Enter", modifier())), None),
            execution::Action::Script => (
                IconName::ListOrdered,
                Some(format!(
                    "{}+{}+Enter",
                    modifier(),
                    if cfg!(target_os = "macos") {
                        "⌥"
                    } else {
                        "Alt"
                    }
                )),
                None,
            ),
            execution::Action::Explain => (
                IconName::GitBranch,
                Some(format!("{}+Shift+Enter", modifier())),
                None,
            ),
            execution::Action::Analyze => (IconName::Gauge, Some("executes it".into()), None),
        },
        Command::Format => (IconName::TextAlignStart, None, None),
        Command::Save => (IconName::Save, None, None),
        Command::OpenSaved(_) | Command::Page(Page::Saved) => (IconName::FileText, None, None),
        Command::Cancel => (IconName::CircleX, Some(format!("{}+.", modifier())), None),
        Command::Pin(_) => (IconName::Pin, None, None),
        Command::ClearPin(_) => (IconName::PinOff, None, None),
        Command::Split => (IconName::Columns2, None, None),
        Command::Close => (IconName::X, Some(format!("{}+W", modifier())), None),
        Command::Theme => (
            if theme.is_dark() {
                IconName::Sun
            } else {
                IconName::Moon
            },
            None,
            None,
        ),
        Command::Named(_) => (IconName::RefreshCw, None, None),
        Command::Updates => (IconName::Download, None, Some("App")),
        Command::Page(_) => (IconName::FolderOpen, None, None),
    };
    let (detail, badge, badge_color) = match profile {
        Some((endpoint, true)) => (Some(endpoint), Some("Connected"), theme.success),
        Some((endpoint, false)) => (Some(endpoint), Some("Connect"), theme.muted_foreground),
        None => (detail, badge, theme.muted_foreground),
    };
    let row = div()
        .flex()
        .items_center()
        .gap_3()
        .size_full()
        .min_w_0()
        .child(
            Icon::new(icon)
                .size(px(16.))
                .text_color(theme.muted_foreground),
        )
        .child(div().min_w_0().truncate().child(label));
    let detail = detail.map(|detail| {
        div()
            .min_w_0()
            .truncate()
            .text_size(px(12.))
            .text_color(theme.muted_foreground)
            .child(detail)
    });
    let draft = matches!(command, Command::Draft(..));
    let badge = badge.map(|badge| {
        div()
            .flex_shrink_0()
            .when(!draft, |badge| badge.ml_auto())
            .rounded(px(4.))
            .px_1p5()
            .py_0p5()
            .text_size(px(10.))
            .line_height(px(15.))
            .text_color(badge_color)
            .bg(if badge_color == theme.success {
                theme.success.opacity(0.2)
            } else {
                theme.muted
            })
            .child(badge)
    });
    if draft {
        row.children(badge).children(detail)
    } else {
        row.children(detail).children(badge)
    }
}
