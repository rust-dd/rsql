use super::*;
use crate::desktop::{Relaunch, geometry::Geometry};
use gpui_kit::component::ActiveTheme;

actions!(rsql, [Quit, Restart, CloseWindow]);

struct RestartStatus {
    message: String,
    recoverable: bool,
}
impl Render for RestartStatus {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .p_4()
            .child(gpui_kit::component::label::Label::new(self.message.clone()))
            .children(self.recoverable.then(super::recovery_button))
    }
}

pub(super) fn bind(
    workspace: &Entity<Workspace>,
    owner: Rc<RefCell<Option<Rc<Session>>>>,
    restart: Result<Relaunch, String>,
    window: &mut Window,
    cx: &mut App,
) {
    let migration = {
        let session = owner.borrow();
        let session = session.as_ref().unwrap();
        migration::Migration::new(
            session.runtime.clone(),
            session.profile_directory().to_owned(),
            restart.clone(),
            None,
        )
    };
    bind_control(workspace, owner, restart, migration, window, cx);
}

pub(super) fn bind_control(
    workspace: &Entity<Workspace>,
    owner: Rc<RefCell<Option<Rc<Session>>>>,
    restart: Result<Relaunch, String>,
    migration: Rc<migration::Migration>,
    window: &mut Window,
    cx: &mut App,
) {
    if let Some(lease) = owner
        .borrow()
        .as_ref()
        .and_then(|s| s.services.local_db.profile_lease())
    {
        migration.attach_lease(&lease);
    }
    migration.bind(window, cx);
    let handle = window.window_handle();
    let weak = workspace.downgrade();
    workspace.update(cx, |_, cx| {
        let session = Rc::downgrade(owner.borrow().as_ref().unwrap());
        cx.observe_window_bounds(window, move |_, w, cx| {
            if let Some(s) = session.upgrade()
                && let Some(g) = Geometry::capture(w, cx)
            {
                s.window_geometry.replace(Some(g));
                s.window_geometry_dirty.set(true);
            }
        })
        .detach();
    });
    let shutdown = {
        let owner = owner.clone();
        move |mut window: Option<&mut Window>, cx: &mut App| {
            let Some(session) = owner.borrow().clone() else {
                return;
            };
            let _ = weak.update(cx, |view, cx| {
                view.capture_edits(cx);
                view.release_maps(window.as_deref_mut(), cx);
                view.release_erd_images(window, cx);
            });
            session.shutdown();
        }
    };
    let gate = migration.clone();
    let quit = shutdown.clone();
    cx.on_action(move |_: &Quit, cx| {
        let gate = gate.clone();
        let quit = quit.clone();
        // The window may still be borrowed by action dispatch.
        cx.defer(move |cx| {
            if gate.defer_close() {
                return;
            }
            quit(None, cx);
            cx.quit();
        });
    });
    let gate = migration.clone();
    let close = shutdown.clone();
    cx.on_action(move |_: &CloseWindow, cx| {
        let gate = gate.clone();
        let close = close.clone();
        cx.defer(move |cx| {
            if gate.defer_close() {
                return;
            }
            let _ = handle.update(cx, |_, w, cx| {
                close(Some(w), cx);
                w.remove_window();
            });
        });
    });
    let gate = Rc::downgrade(&migration);
    let stop = shutdown.clone();
    let transition_owner = owner.clone();
    migration
        .transition
        .replace(Some(Rc::new(move |id, work, install, w, cx| {
            let Some(migration) = gate.upgrade() else {
                return;
            };
            stop(Some(w), cx);
            let Some(session) = transition_owner.borrow().clone() else {
                migration.finish(id, cx);
                return;
            };
            if migration.retain_failed_save(id, &session, w, cx) {
                session.runtime.spawn_blocking(move || drop(install));
                return;
            }
            transition_owner.take();
            let importing = work.is_some();
            let installing = install.is_some();
            let delivery = migration.retiring(work, &session);
            migration.detached();
            let rt = session.runtime.clone();
            let command = restart.clone();
            let screen = cx.new(|_| RestartStatus {
                message: if importing {
                    "Saving and releasing the current profile…"
                } else if installing {
                    "Installing the update…"
                } else {
                    "Restarting RSQL…"
                }
                .into(),
                recoverable: false,
            });
            w.replace_root(cx, |w, cx| gpui_kit::base::Root::new(screen.clone(), w, cx));
            cx.spawn(async move |cx| {
                let result = match migration::receive(delivery).await {
                    Ok(_) if migration.closing() => Ok(()),
                    Ok(_) => rt
                        .spawn_blocking(move || {
                            command.and_then(|c| match install {
                                Some(update) => update.apply(&c),
                                None => c.start(),
                            })
                        })
                        .await
                        .map_err(|e| e.to_string())
                        .and_then(|r| r),
                    Err(error) => Err(error),
                };
                cx.update(|cx| {
                    if !migration.finish(id, cx) {
                        return;
                    }
                    match result {
                        Ok(()) => cx.quit(),
                        Err(error) => screen.update(cx, |s, cx| {
                            s.recoverable = !installing;
                            s.message = if installing {
                                format!("Unable to install the update. {error}")
                            } else {
                                format!("Unable to restart {}. {error}", profile::NAME)
                            };
                            eprintln!("{}", s.message);
                            cx.notify();
                        }),
                    }
                });
            })
            .detach();
        })));
    super::update::bind(owner.clone(), migration.clone(), handle, cx);
    let gate = migration.clone();
    cx.on_action(move |_: &Restart, cx| {
        if owner.borrow().is_none() {
            return;
        }
        let Some(id) = gate.reserve() else { return };
        let transition = gate
            .transition
            .borrow()
            .clone()
            .expect("attached lifecycle");
        let gate = gate.clone();
        cx.defer(move |cx| {
            if handle
                .update(cx, |_, w, cx| transition(id, None, None, w, cx))
                .is_err()
            {
                gate.finish(id, cx);
            }
        });
    });
    let close = shutdown.clone();
    let gate = migration.clone();
    window.on_window_should_close(cx, move |w, cx| {
        if gate.defer_close() {
            return false;
        }
        close(Some(w), cx);
        true
    });
    cx.on_app_quit(move |cx| {
        // GPUI limits the returned future to 200 ms.
        shutdown(None, cx);
        let migration = migration.clone();
        // GPUI disposes its windows and flushes entity drops before polling the quit futures.
        async move {
            migration.join_admitted();
        }
    })
    .detach();
}
