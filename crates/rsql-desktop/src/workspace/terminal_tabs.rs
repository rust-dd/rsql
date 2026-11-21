use super::*;
use rsql_desktop::{
    terminal::{TerminalTheme, session::Viewport},
    terminal_view::TerminalView,
};

pub(super) const SHORTCUT: &str = if cfg!(target_os = "macos") {
    "⌘+`"
} else {
    "Ctrl+`"
};
pub(super) type Panel = Result<Entity<TerminalView>, String>;

impl Workspace {
    pub(super) fn dispatch_open_terminal(&self, w: &mut Window, cx: &mut App) {
        let focus = self.focus.clone();
        w.defer(cx, move |w, cx| focus.dispatch_action(&OpenTerminal, w, cx));
    }

    pub(super) fn open_terminal(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        self.update_documents(w, cx, |docs| {
            docs.open(Kind::Terminal, None);
        });
    }

    pub(super) fn sync_terminals(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        let docs = self.session.documents.borrow();
        let active = docs.active();
        let theme = match docs.theme {
            Appearance::Light => TerminalTheme::Light,
            Appearance::Dark => TerminalTheme::Dark,
        };
        self.terminal_panels.retain(|id, panel| {
            if docs
                .tabs
                .iter()
                .any(|tab| tab.kind == Kind::Terminal && tab.id == *id)
            {
                return true;
            }
            if let Ok(view) = panel {
                let done = view.read(cx).session().close();
                self.session.runtime.spawn(async move {
                    if let Err(error) = done.await {
                        eprintln!("{}: {error}", crate::profile::NAME);
                    }
                });
            }
            false
        });
        if let Some(tab) = active.filter(|t| t.kind == Kind::Terminal) {
            self.terminal_panels
                .entry(tab.id.clone())
                .or_insert_with(|| {
                    let size = Viewport::new(80, 24).expect("valid initial terminal dimensions");
                    self.session
                        .terminals
                        .borrow_mut()
                        .start_with_theme(&tab.id, size, theme)
                        .map(|session| cx.new(|cx| TerminalView::new(session, theme, true, w, cx)))
                });
        }
        for (id, panel) in &self.terminal_panels {
            if let Ok(view) = panel {
                view.update(cx, |v, cx| {
                    v.set_visible(active.is_some_and(|tab| tab.id == *id), cx);
                    v.set_theme(theme, cx);
                });
            }
        }
    }
}
