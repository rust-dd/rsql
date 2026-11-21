use super::{Command, Page, Palette, presentation, workspaces};
use crate::runtime::workspaces::{WriteKind, WriteTarget};
use gpui_kit::{App, ListOffset};

#[derive(Default)]
pub(super) enum Selection {
    #[default]
    Initial,
    Unselected,
    Chosen(Box<Command>),
}

impl Palette {
    pub(super) fn sync_search_selection(&mut self, cx: &App) {
        let search = self.search.read(cx).value();
        if self.selected_search != search {
            self.selected_search = search;
            if self.page == Page::SaveWorkspace {
                self.workspace_operation.name_changed();
            }
            self.reset_selection();
        }
    }

    pub(super) fn selected_index(&self, entries: &[(String, Command)]) -> Option<usize> {
        match &self.selected {
            Selection::Initial => entries.first().map(|_| 0),
            Selection::Unselected => None,
            Selection::Chosen(command) => entries.iter().position(|(_, c)| c == command.as_ref()),
        }
    }

    pub(super) fn reconcile_selection(&mut self, entries: &[(String, Command)]) -> Option<usize> {
        let index = self.selected_index(entries);
        if let Some(i) = index {
            if matches!(self.selected, Selection::Initial) {
                self.selected = Selection::Chosen(Box::new(entries[i].1.clone()));
            }
            self.scroll_to_choice(i, entries);
        } else if let Selection::Chosen(command) = &self.selected {
            // The failure callback promotes only the submitted delete to its retry.
            let pending_retry = matches!(command.as_ref(), Command::Named(workspaces::Command::Delete { target, retry: false })
                if self.workspace_operation.deleting(target));
            if !pending_retry {
                self.selected = Selection::Unselected;
                self.scrolled_to = None;
            }
        }
        index
    }

    pub(super) fn reset_selection(&mut self) {
        self.selected = Selection::Initial;
        self.pressed = None;
        self.scrolled_to = None;
        self.scroll.scroll_to(ListOffset::default());
    }

    pub(super) fn choose_index(&mut self, index: usize, entries: &[(String, Command)]) {
        self.selected = Selection::Chosen(Box::new(entries[index].1.clone()));
        self.scroll_to_choice(index, entries);
    }

    fn scroll_to_choice(&mut self, index: usize, entries: &[(String, Command)]) {
        self.update_row_layout(entries);
        let Some(row) = self
            .row_layout
            .iter()
            .position(|row| *row == presentation::Row::Choice(index))
        else {
            return;
        };
        if self.scrolled_to != Some(row) {
            self.scroll.scroll_to_reveal_item(row);
            self.scrolled_to = Some(row);
        }
    }

    pub(super) fn update_row_layout(&mut self, entries: &[(String, Command)]) {
        let layout = presentation::rows(self.page, entries);
        if self.row_layout != layout {
            self.scroll.reset(layout.len());
            self.row_layout = layout;
            self.scrolled_to = None;
        }
    }

    pub(super) fn select_failed_write(&mut self, failed: &WriteTarget) {
        if let Selection::Chosen(command) = &mut self.selected
            && let Command::Named(workspaces::Command::Delete { target, retry }) = command.as_mut()
            && matches!(failed, WriteTarget::Delete(t) if t == target)
        {
            *retry = true;
        }
    }

    pub(super) fn selected_retry(&self) -> Option<(WriteKind, &str)> {
        if let Selection::Chosen(command) = &self.selected
            && let Command::Named(named) = command.as_ref()
        {
            return named.retry();
        }
        None
    }
}
