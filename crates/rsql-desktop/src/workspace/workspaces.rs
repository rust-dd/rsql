use super::*;
use crate::runtime::workspaces::{Capture, CapturedSql};
use rsql_desktop::named_workspace::{self, NamedTab};

impl Workspace {
    pub(super) fn capture_named_workspace(&self, cx: &App) -> Capture {
        named_workspace::capture(&self.session.documents.borrow(), |tab| {
            match self.editors.get(&(tab.id.clone(), Side::Main)) {
                Some(live) => CapturedSql::Retained(live.state.read(cx).text().clone()),
                None => CapturedSql::Draft(tab.main.draft.text.clone()),
            }
        })
        .into()
    }

    pub(super) fn append_named_workspace(
        &mut self,
        tabs: &[NamedTab],
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if tabs.is_empty() {
            return;
        }
        self.session.apply_removals();
        let session = self.session.clone();
        self.update_documents(w, cx, |docs| {
            named_workspace::append(docs, &session.connections.borrow(), tabs);
        });
    }
}
