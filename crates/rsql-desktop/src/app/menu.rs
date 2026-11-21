use super::*;
use crate::workspace::theme::controls::compact_button;
use gpui_kit::component::{WindowExt, button::Button, label::Label};

actions!(
    rsql,
    [
        About, HideApp, HideOthers, ShowAll, Fullscreen, Minimize, Maximize
    ]
);

fn active_window(cx: &mut App, f: impl FnOnce(&mut Window, &mut App) + 'static) {
    let Some(window) = cx.active_window() else {
        return;
    };
    cx.defer(move |cx| {
        let _ = window.update(cx, |_, w, cx| f(w, cx));
    });
}

pub(super) fn bind(cx: &mut App) {
    cx.on_action(|_: &About, cx| {
        active_window(cx, |w, cx| {
            w.open_dialog(cx, |dialog, _, _| {
                let mut panel = dialog
                    .title(Label::new(format!("About {}", profile::NAME)))
                    .w(px(360.))
                    .child(Label::new(format!("Version {}", env!("CARGO_PKG_VERSION"))));
                if let Some(revision) = option_env!("RSQL_REVISION") {
                    panel = panel.child(Label::new(format!("Build {}", &revision[..12])));
                }
                panel.child("Modern SQL client for PostgreSQL.").child(
                    compact_button(Button::new("about-website").label(profile::WEBSITE)).on_click(
                        |_, _, cx| crate::desktop::DesktopServices::open_url(cx, profile::WEBSITE),
                    ),
                )
            });
        })
    });
    if cfg!(target_os = "macos") {
        cx.on_action(|_: &HideApp, cx| cx.hide());
        cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
        cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
    }
    cx.on_action(|_: &Fullscreen, cx| active_window(cx, |w, _| w.toggle_fullscreen()));
    cx.on_action(|_: &Minimize, cx| active_window(cx, |w, _| w.minimize_window()));
    cx.on_action(|_: &Maximize, cx| active_window(cx, |w, _| w.zoom_window()));
}

pub(super) fn install(cx: &mut App) {
    cx.set_menus(menus());
    if !cfg!(target_os = "macos")
        && let Some(menus) = cx.get_menus()
    {
        gpui_kit::base::GlobalState::global_mut(cx).set_app_menus(menus);
    }
}

pub(super) fn menus() -> Vec<Menu> {
    let mut app = vec![MenuItem::action(format!("About {}", profile::NAME), About)];
    app.push(MenuItem::action(
        "Check for Updates",
        crate::workspace::CheckForUpdates,
    ));
    if cfg!(target_os = "macos") {
        app.extend([
            MenuItem::Separator,
            MenuItem::os_submenu("Services", SystemMenuType::Services),
            MenuItem::Separator,
            MenuItem::action(format!("Hide {}", profile::NAME), HideApp),
            MenuItem::action("Hide Others", HideOthers),
            MenuItem::action("Show All", ShowAll),
        ]);
    }
    app.extend([
        MenuItem::Separator,
        MenuItem::action(
            "Import and recover profile",
            super::migration::ReviewMigration,
        ),
        MenuItem::action(format!("Restart {}", profile::NAME), Restart),
        MenuItem::action(format!("Quit {}", profile::NAME), Quit),
    ]);
    vec![
        Menu::new(profile::NAME).items(app),
        Menu::new("Edit").items([
            MenuItem::os_action("Undo", Undo, OsAction::Undo),
            MenuItem::os_action("Redo", Redo, OsAction::Redo),
            MenuItem::Separator,
            MenuItem::os_action("Cut", Cut, OsAction::Cut),
            MenuItem::os_action("Copy", Copy, OsAction::Copy),
            MenuItem::os_action("Paste", Paste, OsAction::Paste),
            MenuItem::os_action("Select All", SelectAll, OsAction::SelectAll),
            MenuItem::Separator,
            MenuItem::action("Show Suggestions", ShowCompletions),
            MenuItem::action("Go to Definition", GoToDefinition),
            MenuItem::action("Find References", FindReferences),
        ]),
        Menu::new("Query").items([MenuItem::action(
            "Explain main statement",
            crate::workspace::ExplainMain,
        )]),
        Menu::new("View").items([
            MenuItem::action("Toggle Fullscreen", Fullscreen),
            MenuItem::action("Command Palette", TogglePalette),
            MenuItem::action("Focus SQL Editor", FocusEditor),
            MenuItem::action("New Query", NewQuery),
            MenuItem::action("Open Terminal", OpenTerminal),
            MenuItem::action("Next Tab", NextTab),
            MenuItem::action("Close Tab", CloseTab),
            MenuItem::action("Toggle Split", ToggleSplit),
            MenuItem::action("Toggle Theme", ToggleTheme),
        ]),
        Menu::new("Window").items([
            MenuItem::action("Minimize", Minimize),
            MenuItem::action("Maximize", Maximize),
            MenuItem::Separator,
            MenuItem::action("Close Window", super::lifecycle::CloseWindow),
        ]),
    ]
}
