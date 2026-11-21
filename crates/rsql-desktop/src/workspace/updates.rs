use super::{CheckForUpdates, Workspace, theme};
use gpui_kit::{App, Context, Div, Window};
use rsql_desktop::update::Origin;
impl Workspace {
    pub(super) fn check_for_updates(&self) {
        self.session.updates.check(Origin::Manual);
    }
    pub(super) fn update_panel(&self, cx: &App) -> Option<Div> {
        rsql_desktop::update::panel(&self.session.updates, theme::controls::compact_button, cx)
    }
    pub(super) fn check_updates_action(
        &self,
        cx: &Context<Self>,
    ) -> impl Fn(&CheckForUpdates, &mut Window, &mut App) + 'static {
        Self::shell_action(cx, |view, _, _| view.check_for_updates())
    }
}
