use super::{Command as PaletteCommand, Page, Palette, Workspace, *};
use crate::runtime::workspaces::{WorkspaceTarget, WriteKind, WriteStatus, WriteTarget, WriteTask};
use gpui_kit::assets::IconName;

mod display;
pub(super) use display::{row_content, workspace_icon};

#[derive(Clone, PartialEq, Eq)]
pub(super) enum Command {
    Load(WorkspaceTarget),
    Delete {
        target: WorkspaceTarget,
        retry: bool,
    },
    RetryWrite(WriteTarget),
    RetryRead,
}

impl Command {
    pub(super) fn retry(&self) -> Option<(WriteKind, &str)> {
        match self {
            Self::Delete {
                target,
                retry: true,
            } => Some((WriteKind::Delete, &target.name)),
            Self::RetryWrite(target) => Some((target.kind(), target.name())),
            _ => None,
        }
    }
}

#[derive(Default)]
pub(super) struct Operation {
    write: Option<(WriteTarget, Task<()>)>,
    error: Option<String>,
    saved_name: Option<String>,
}

impl Operation {
    pub(super) fn writing(&self) -> bool {
        self.write.is_some()
    }

    pub(super) fn deleting(&self, target: &WorkspaceTarget) -> bool {
        matches!(&self.write, Some((WriteTarget::Delete(queued), _)) if queued == target)
    }

    pub(super) fn name_changed(&mut self) {
        if self.write.is_none() {
            self.error = None;
            self.saved_name = None;
        }
    }
}

impl Page {
    pub(super) fn workspace_list(self) -> bool {
        matches!(self, Self::LoadWorkspace | Self::DeleteWorkspace)
    }

    pub(super) fn placeholder(self) -> &'static str {
        match self {
            Self::SaveWorkspace => "Workspace name...",
            Self::SaveQuery => "Query name...",
            Self::Connections => "Search connections...",
            Self::Schema => "Search tables, views, functions...",
            Self::Saved => "Search saved queries...",
            _ => "Search commands, tables, views...",
        }
    }

    fn workspace_title(self) -> Option<(&'static str, IconName)> {
        match self {
            Self::SaveWorkspace => Some(("Save Workspace", IconName::Save)),
            Self::LoadWorkspace => Some(("Load Workspace", IconName::FolderOpen)),
            Self::DeleteWorkspace => Some(("Delete Workspace", IconName::Trash)),
            _ => None,
        }
    }
}

impl Palette {
    pub(super) fn focus(&self, w: &mut Window, cx: &mut App) {
        if self.page.workspace_list() {
            self.list_focus.focus(w, cx);
        } else {
            self.search.focus_handle(cx).focus(w, cx);
        }
    }
}

impl Workspace {
    pub(super) fn workspace_entries(&self, page: Page, rows: &mut Vec<(String, PaletteCommand)>) {
        let library = self.session.named_workspaces();
        if page == Page::Root {
            for page in [
                Page::SaveWorkspace,
                Page::LoadWorkspace,
                Page::DeleteWorkspace,
            ] {
                if page == Page::SaveWorkspace || !library.rows.is_empty() {
                    let (label, _) = page.workspace_title().unwrap();
                    rows.push((label.into(), PaletteCommand::Page(page)));
                }
            }
            let saves = library.save_failures.keys().cloned().map(WriteTarget::Save);
            let deletes = library
                .delete_failures
                .keys()
                .filter_map(|name| Some(WriteTarget::Delete(library.rows.get(name)?.target(name))));
            rows.extend(saves.chain(deletes).map(|target| {
                let label = format!("Retry {} “{}”", target.kind().verb(), target.name());
                (label, PaletteCommand::Named(Command::RetryWrite(target)))
            }));
        } else if page.workspace_list() {
            rows.extend(library.rows.iter().map(|(name, entry)| {
                let target = entry.target(name);
                let command = if page == Page::LoadWorkspace {
                    Command::Load(target)
                } else {
                    Command::Delete {
                        target,
                        retry: library.delete_failures.contains_key(name),
                    }
                };
                (name.clone(), PaletteCommand::Named(command))
            }));
        }
        if (page == Page::Root || page.workspace_list()) && library.load_error.is_some() {
            rows.push((
                "Retry loading workspaces".into(),
                PaletteCommand::Named(Command::RetryRead),
            ));
        }
    }

    pub(super) fn run_workspace_command(
        &mut self,
        command: Command,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(palette) = self.palette.as_ref() else {
            return;
        };
        if palette.workspace_operation.write.is_some() {
            return;
        }
        match command {
            Command::Load(target) if palette.page == Page::LoadWorkspace => {
                let tabs = self
                    .session
                    .named_workspaces()
                    .entry(&target)
                    .map(|entry| entry.tabs.clone());
                match tabs {
                    Some(Ok(tabs)) => {
                        self.dismiss_palette(w, cx);
                        self.append_named_workspace(&tabs, w, cx);
                    }
                    Some(Err(error)) => self.workspace_error(error, cx),
                    None => {}
                }
            }
            Command::Delete { target, retry } if palette.page == Page::DeleteWorkspace => {
                {
                    let library = self.session.named_workspaces();
                    if library.entry(&target).is_none()
                        || library.delete_failures.contains_key(&target.name) != retry
                    {
                        return;
                    }
                }
                if retry {
                    self.retry_workspace_write(WriteTarget::Delete(target), w, cx);
                } else {
                    let write = self.session.delete_named_workspace(target.clone());
                    self.wait_for_workspace_write(write, WriteTarget::Delete(target), w, cx);
                }
            }
            Command::RetryWrite(target) if palette.page == Page::Root => {
                self.retry_workspace_write(target, w, cx);
            }
            Command::RetryRead if palette.page == Page::Root || palette.page.workspace_list() => {
                self.session.refresh_named_workspaces();
            }
            _ => {}
        }
    }

    pub(super) fn submit_workspace_save(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        let Some(palette) = self.palette.as_mut() else {
            return;
        };
        if palette.page != Page::SaveWorkspace || palette.workspace_operation.write.is_some() {
            return;
        }
        let name = palette.search.read(cx).value();
        let name = name.trim().to_owned();
        if name.is_empty() {
            self.workspace_error("Enter a workspace name.".into(), cx);
            return;
        }
        let retry = self
            .session
            .named_workspaces()
            .save_failures
            .contains_key(&name);
        if retry {
            self.retry_workspace_write(WriteTarget::Save(name), w, cx);
        } else {
            let snapshot = self.capture_named_workspace(cx);
            let write = self.session.save_named_workspace(name.clone(), snapshot);
            self.wait_for_workspace_write(write, WriteTarget::Save(name), w, cx);
        }
    }

    fn retry_workspace_write(
        &mut self,
        target: WriteTarget,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        {
            let library = self.session.named_workspaces();
            if library.write_error(target.kind(), target.name()).is_none()
                || matches!(&target, WriteTarget::Delete(t) if library.entry(t).is_none())
            {
                return;
            }
        }
        let task = self.session.retry_named_workspace(target.clone());
        self.wait_for_workspace_write(task, target, w, cx);
    }

    fn wait_for_workspace_write(
        &mut self,
        write: WriteTask,
        target: WriteTarget,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let kind = target.kind();
        let palette = self.palette.as_mut().unwrap();
        let operation = &mut palette.workspace_operation;
        operation.error = None;
        operation.saved_name = None;
        let source = palette.search.entity_id();
        let page = palette.page;
        let submitted_text = matches!(page, Page::Root | Page::SaveWorkspace)
            .then(|| palette.search.read(cx).value());
        let submitted = target.clone();
        let task = cx.spawn_in(w, async move |view, cx| {
            let result = write.await;
            let _ = view.update_in(cx, |workspace, w, cx| {
                let Some(palette) = &mut workspace.palette else {
                    return;
                };
                if palette.search.entity_id() != source || palette.page != page {
                    return;
                }
                palette.workspace_operation.write.take();
                let name_edited = submitted_text
                    .is_some_and(|submitted| submitted != palette.search.read(cx).value());
                match result {
                    Ok(Ok(WriteStatus::Committed)) if kind == WriteKind::Save => {
                        if name_edited {
                            palette.workspace_operation.saved_name = Some(target.name().into());
                            cx.notify();
                        } else {
                            workspace.dismiss_palette(w, cx);
                        }
                    }
                    Ok(Ok(WriteStatus::Committed))
                        if page == Page::DeleteWorkspace
                            && workspace.session.named_workspaces().rows.is_empty() =>
                    {
                        workspace.page(Page::Root, w, cx);
                    }
                    Ok(Ok(_)) => cx.notify(),
                    Ok(Err(_)) => {
                        if page == Page::DeleteWorkspace || (page == Page::Root && !name_edited) {
                            palette.select_failed_write(&target);
                        }
                        cx.notify();
                    }
                    Err(error) => {
                        workspace.workspace_error(
                            format!(
                                "Could not {} workspace “{}”: {error}",
                                kind.verb(),
                                target.name()
                            ),
                            cx,
                        );
                    }
                }
            });
        });
        self.palette.as_mut().unwrap().workspace_operation.write = Some((submitted, task));
        cx.notify();
    }

    fn workspace_error(&mut self, error: String, cx: &mut Context<Self>) {
        if let Some(palette) = &mut self.palette {
            palette.workspace_operation.error = Some(error);
            cx.notify();
        }
    }
}
