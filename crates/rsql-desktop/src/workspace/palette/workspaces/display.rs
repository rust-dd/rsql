use super::*;
use gpui_kit::component::{ActiveTheme, Icon};

impl Workspace {
    pub(in crate::workspace::palette) fn workspace_header(
        &self,
        page: Page,
        cx: &Context<Self>,
    ) -> Option<impl IntoElement> {
        let (title, icon) = page.workspace_title()?;
        if page == Page::SaveWorkspace {
            return None;
        }
        Some(
            div()
                .id("workspace-page-title")
                .test_support()
                .flex()
                .items_center()
                .gap_2()
                .px_4()
                .py_3()
                .line_height(px(20.))
                .border_b_1()
                .border_color(cx.theme().border)
                .child(workspace_icon(icon, false, cx))
                .child(div().flex_1().child(title))
                .child(self.palette_back(cx)),
        )
    }

    pub(in crate::workspace::palette) fn workspace_status(
        &self,
        palette: &Palette,
        cx: &App,
    ) -> impl IntoElement {
        let library = self.session.named_workspaces();
        let mut status = div();
        if palette.page == Page::SaveWorkspace {
            status = status.child(workspace_message(
                "workspace-save-hint",
                "Press Enter to save current query tabs as a workspace",
                false,
                cx,
            ));
        }
        let operation = &palette.workspace_operation;
        let failed = if palette.page == Page::SaveWorkspace {
            Some((
                WriteKind::Save,
                palette.search.read(cx).value().trim().to_owned(),
            ))
        } else {
            palette
                .selected_retry()
                .map(|(kind, name)| (kind, name.to_owned()))
        };
        let error = operation.error.clone().or_else(|| {
            let (kind, name) = failed?;
            library
                .write_error(kind, &name)
                .map(|error| format!("Could not {} workspace “{name}”: {error}", kind.verb()))
        });
        if let Some((target, _)) = &operation.write {
            let label = match target.kind() {
                WriteKind::Save => "Saving workspace…",
                WriteKind::Delete => "Deleting workspace…",
            };
            status = status.child(workspace_message("workspace-writing", label, false, cx));
        } else if let Some(error) = error {
            status = status.child(workspace_message("workspace-error", error, true, cx));
        } else if let Some(name) = &operation.saved_name {
            status = status.child(workspace_message(
                "workspace-saved",
                format!("Saved workspace “{name}”"),
                false,
                cx,
            ));
        }
        if (palette.page == Page::Root || palette.page.workspace_list())
            && let Some(error) = &library.load_error
        {
            status = status.child(workspace_message(
                "workspace-load-error",
                format!("Could not load workspaces: {error}"),
                true,
                cx,
            ));
        } else if palette.page.workspace_list() && library.loading && library.rows.is_empty() {
            status = status.child(workspace_message(
                "workspace-loading",
                "Loading workspaces…",
                false,
                cx,
            ));
        }
        status
    }
}

pub(in crate::workspace::palette) fn row_content(
    label: String,
    command: &PaletteCommand,
    library: &crate::runtime::workspaces::Library,
    profile: Option<(String, bool)>,
    cx: &App,
) -> (String, Div) {
    let content = div().size_full();
    match command {
        PaletteCommand::Page(page) if page.workspace_title().is_some() => {
            let (_, icon) = page.workspace_title().unwrap();
            (
                label.clone(),
                content
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(workspace_icon(icon, false, cx))
                    .child(label),
            )
        }
        PaletteCommand::Named(Command::Load(target) | Command::Delete { target, .. }) => {
            let delete = matches!(command, PaletteCommand::Named(Command::Delete { .. }));
            let count = library.entry(target).map_or("", |entry| entry.tab_label());
            let error = delete
                .then(|| library.delete_failures.get(&target.name))
                .flatten();
            let (accessibility, details, color) = match error {
                Some(error) => (
                    format!("{label}, {count}, {error}"),
                    format!("{count} · {error}"),
                    cx.theme().danger,
                ),
                None => (
                    format!("{label}, {count}"),
                    count.to_owned(),
                    cx.theme().muted_foreground,
                ),
            };
            let content = content
                .flex()
                .items_center()
                .gap_3()
                .child(workspace_icon(IconName::FolderOpen, false, cx))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .font_weight(FontWeight::MEDIUM)
                                .truncate()
                                .child(label),
                        )
                        .child(
                            div()
                                .text_size(px(12.))
                                .truncate()
                                .text_color(color)
                                .child(details),
                        ),
                )
                .children(delete.then(|| workspace_icon(IconName::Trash, true, cx)));
            (accessibility, content)
        }
        PaletteCommand::Named(Command::RetryWrite(target)) => {
            let detail = library
                .write_error(target.kind(), target.name())
                .map_or_else(|| label.clone(), |error| format!("{label} · {error}"));
            (
                detail.clone(),
                content
                    .truncate()
                    .text_color(cx.theme().danger)
                    .child(detail),
            )
        }
        _ => (
            label.clone(),
            super::super::presentation::content(label, command, profile, cx),
        ),
    }
}

pub(in crate::workspace::palette) fn workspace_icon(
    name: IconName,
    destructive: bool,
    cx: &App,
) -> Icon {
    Icon::new(name).size(px(16.)).text_color(if destructive {
        cx.theme().danger
    } else {
        cx.theme().muted_foreground
    })
}

fn workspace_message(
    id: &'static str,
    text: impl Into<SharedString>,
    destructive: bool,
    cx: &App,
) -> impl IntoElement {
    div()
        .id(id)
        .test_support()
        .px_4()
        .py_3()
        .text_size(px(12.))
        .text_color(if destructive {
            cx.theme().danger
        } else {
            cx.theme().muted_foreground
        })
        .child(text.into())
}
