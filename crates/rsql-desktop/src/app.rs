use crate::workspace::theme::catalog::AppAssets;
use crate::{
    profile,
    runtime::Session,
    workspace::{
        CloseTab, FindReferences, FocusEditor, NewQuery, NextTab, OpenTerminal, TogglePalette,
        ToggleSplit, ToggleTheme, Workspace,
    },
};
use gpui_kit::{
    component::input::{Copy, Cut, GoToDefinition, Paste, Redo, SelectAll, ShowCompletions, Undo},
    *,
};
use std::{cell::RefCell, rc::Rc};

mod lifecycle;
mod menu;
mod migration;
mod update;
pub(crate) use lifecycle::{Quit, Restart};

struct StartupError {
    message: String,
    can_recover: bool,
}

fn recovery_button() -> gpui_kit::component::button::Button {
    crate::workspace::theme::controls::compact_button(
        gpui_kit::component::button::Button::new("Open recovery").label("Open recovery"),
    )
    .on_click(|_, w, cx| w.dispatch_action(Box::new(migration::ReviewMigration), cx))
}

impl StartupError {
    fn open(
        message: String,
        recovery: Option<Rc<migration::Migration>>,
        w: &mut Window,
        cx: &mut App,
    ) -> Entity<Self> {
        let can_recover = recovery.is_some();
        if let Some(migration) = recovery {
            migration.bind(w, cx);
            let gate = migration.clone();
            w.on_window_should_close(cx, move |_, _| !gate.defer_close());
            let gate = migration.clone();
            cx.on_action(move |_: &Quit, cx| {
                if !gate.defer_close() {
                    cx.quit();
                }
            });
            cx.on_app_quit(move |_| {
                migration.join_admitted();
                std::future::ready(())
            })
            .detach();
        } else {
            cx.on_action(|_: &Quit, cx| cx.quit());
        }
        cx.new(|_| StartupError {
            message,
            can_recover,
        })
    }
}

impl Render for StartupError {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .p_4()
            .gap_2()
            .child(format!("Unable to start {}", profile::NAME))
            .child(self.message.clone())
            .children(self.can_recover.then(recovery_button))
    }
}

pub fn run(
    startup: Result<Rc<Session>, String>,
    arguments: Vec<std::ffi::OsString>,
    recovery: Option<(tokio::runtime::Handle, std::path::PathBuf)>,
) -> Result<(), String> {
    let startup = startup.map(|s| Rc::new(RefCell::new(Some(s))));
    let fallback = startup.as_ref().ok().cloned();
    let startup_error = Rc::new(RefCell::new(None));
    let window_error = startup_error.clone();
    application().with_assets(AppAssets).run(move |cx| {
        init(cx);
        menu::bind(cx);
        cx.set_app_identity(profile::ID, profile::NAME);
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        bind_quit_key(cx, cfg!(target_os = "macos"));
        bind_shell_keys(cx);
        menu::install(cx);
        let geometry = startup
            .as_ref()
            .ok()
            .and_then(|s| s.borrow().as_ref()?.window_geometry.borrow().clone());
        let options = crate::desktop::geometry::options(geometry, cx);
        let window = match startup {
            Ok(owner) => {
                let session = owner.borrow().as_ref().unwrap().clone();
                let restart = crate::desktop::DesktopServices::relaunch(cx, arguments);
                let result = open_window(options, cx, |window, cx| {
                    let workspace = cx.new(|cx| Workspace::new(session.clone(), window, cx));
                    lifecycle::bind(&workspace, owner, restart, window, cx);
                    workspace
                })
                .map(|(window, _)| window);
                if result.is_err() {
                    session.shutdown();
                }
                result
            }
            Err(message) => {
                crate::workspace::theme::apply(gpui_kit::component::ThemeMode::Light, None, cx);

                *window_error.borrow_mut() = Some(message.clone());
                open_window(options, cx, |w, cx| {
                    let recovery = recovery.map(|(runtime, profile)| {
                        let restart = crate::desktop::DesktopServices::relaunch(cx, arguments);
                        migration::Migration::new(
                            runtime,
                            profile,
                            restart,
                            Some(window_error.clone()),
                        )
                    });
                    StartupError::open(message, recovery, w, cx)
                })
                .map(|(window, _)| window)
            }
        };
        if let Err(error) = window {
            let message = format!("Open window: {error}");
            eprintln!("{}: {message}", profile::NAME);
            *window_error.borrow_mut() = Some(message);
            cx.quit();
            return;
        }
        cx.activate(true);
    });
    if let Some(session) = fallback.and_then(|s| s.take()) {
        session.shutdown();
    }
    startup_error.take().map_or(Ok(()), Err)
}

pub(crate) fn bind_quit_key(cx: &mut App, macos: bool) {
    cx.bind_keys([KeyBinding::new(
        if macos { "cmd-q" } else { "ctrl-q" },
        Quit,
        None,
    )]);
}

pub(crate) fn bind_shell_keys(cx: &mut App) {
    crate::workspace::bind_grid_keys(cx);
    let [k, p, w] = if cfg!(target_os = "macos") {
        ["cmd-k", "cmd-p", "cmd-w"]
    } else {
        ["ctrl-k", "ctrl-p", "ctrl-w"]
    };
    use crate::workspace::{CancelQuery, ExplainStatement, RunScript, RunStatement};
    let keys = if cfg!(target_os = "macos") {
        ["cmd-enter", "cmd-alt-enter", "cmd-shift-enter", "cmd-."]
    } else {
        ["ctrl-enter", "ctrl-alt-enter", "ctrl-shift-enter", "ctrl-."]
    };
    let editor = Some("RsqlWorkspace > (Input && mode == editor)");
    cx.bind_keys([
        KeyBinding::new(keys[0], RunStatement, editor),
        KeyBinding::new(keys[1], RunScript, editor),
        KeyBinding::new(keys[2], ExplainStatement, editor),
        KeyBinding::new(keys[3], CancelQuery, Some("RsqlWorkspace && !RsqlPalette")),
        KeyBinding::new(keys[3], CancelQuery, editor),
        KeyBinding::new(
            "ctrl-space",
            ShowCompletions,
            Some("RsqlWorkspace > (Input && mode == editor)"),
        ),
        KeyBinding::new(
            "f12",
            GoToDefinition,
            Some("RsqlWorkspace > (Input && mode == editor)"),
        ),
        KeyBinding::new(
            "shift-f12",
            FindReferences,
            Some("RsqlWorkspace > (Input && mode == editor)"),
        ),
        KeyBinding::new("f6", FocusEditor, Some("RsqlWorkspace && !RsqlPalette")),
        KeyBinding::new(
            if cfg!(target_os = "macos") {
                "cmd-`"
            } else {
                "ctrl-`"
            },
            OpenTerminal,
            Some("RsqlWorkspace && !RsqlPalette"),
        ),
        KeyBinding::new(k, TogglePalette, Some("RsqlWorkspace")),
        KeyBinding::new(p, TogglePalette, Some("RsqlWorkspace")),
        KeyBinding::new(w, CloseTab, Some("RsqlWorkspace && !RsqlPalette")),
    ]);
}
