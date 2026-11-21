use super::*;
use rsql_desktop::update::{InstallUpdate, install};

/// Stage the ready update while the workspace stays usable, then restart through the saving quit path.
pub(super) fn bind(
    owner: Rc<RefCell<Option<Rc<Session>>>>,
    migration: Rc<migration::Migration>,
    handle: AnyWindowHandle,
    cx: &mut App,
) {
    cx.on_action(move |_: &InstallUpdate, cx| {
        // Holding the Session here would keep its profile lease alive through the restart.
        let Some((updates, runtime)) = owner
            .borrow()
            .as_ref()
            .map(|s| (s.updates.clone(), s.runtime.clone()))
        else {
            return;
        };
        let Some(id) = migration.reserve() else {
            return;
        };
        let Some(artifact) = updates.begin_install() else {
            migration.finish(id, cx);
            return;
        };
        let migration = migration.clone();
        cx.spawn(async move |cx| {
            let staged = runtime
                .spawn_blocking(move || install::stage(artifact))
                .await
                .map_err(|e| e.to_string())
                .and_then(|r| r);
            cx.update(|cx| {
                let transition = migration.transition.borrow().clone();
                match (staged, transition) {
                    (Ok(update), Some(transition)) => {
                        if handle
                            .update(cx, |_, w, cx| transition(id, None, Some(update), w, cx))
                            .is_err()
                        {
                            migration.finish(id, cx);
                        }
                    }
                    (staged, _) => {
                        updates.install_failed(staged.err().unwrap_or_else(|| {
                            format!("{} cannot restart from this window.", profile::NAME)
                        }));
                        migration.finish(id, cx);
                    }
                }
            });
        })
        .detach();
    });
}
