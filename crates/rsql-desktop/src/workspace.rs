mod admin;
mod catalog;
mod diagnostic;
use rsql_desktop::clipboard_order;
mod editor_overlay;
mod editor_presentation;
mod editor_provider;
mod editor_services;
mod editor_snippet;
mod erd;
mod execution;
mod explain_panel;
mod form;
mod history;
mod import;
mod map;
mod monitor;
mod notify;
pub(crate) mod numbers;
mod palette;
mod properties;
mod query_result;
mod query_toolbar;
mod results;
pub(crate) use results::bind_keys as bind_grid_keys;
mod saved;
mod saved_query;
mod schema_diff;
mod shell;
mod shortcuts;
mod sidebar;
pub(super) mod structure_apply;
mod terminal_tabs;
pub(crate) mod theme;
mod toast;
mod updates;
mod workspaces;
use crate::runtime::Session;
use gpui_kit::{
    component::{
        ThemeMode, WindowExt,
        input::{Editor, EditorState},
    },
    *,
};
use rsql_desktop::document::{Appearance, Documents, Draft, Kind, Side};
use std::{collections::HashMap, rc::Rc, time::Duration};

actions!(
    rsql,
    [
        CheckForUpdates,
        TogglePalette,
        FocusEditor,
        FindReferences,
        NewQuery,
        OpenTerminal,
        NextTab,
        CloseTab,
        ToggleSplit,
        ToggleTheme,
        RunStatement,
        RunScript,
        ExplainStatement,
        ExplainMain,
        CancelQuery
    ]
);

type Owner = (String, Side);
struct LiveEditor {
    history: Option<Entity<history::Panel>>,
    search: Option<results::SearchInput>,
    export_focus: FocusHandle,
    grid: Option<Entity<results::Grid>>,
    map: Option<Entity<map::ResultMap>>,
    grid_events: Vec<Subscription>,
    result_updates: Option<Task<()>>,
    expansion: explain_panel::Expansion,
    state: Entity<EditorState>,
    _observe: Subscription,
    controls: execution::Controls,
    language: editor_services::Services,
}

pub struct Workspace {
    admin_panels: HashMap<admin::PanelKey, Entity<admin::Panel>>,
    notify_panels: HashMap<String, Entity<notify::Panel>>,
    erd_panels: HashMap<String, Entity<erd::Panel>>,
    monitors: HashMap<String, Entity<monitor::Panel>>,
    menu_bar: Option<Entity<gpui_kit::component::menu::AppMenuBar>>,
    _visibility: Subscription,
    terminal_panels: HashMap<String, terminal_tabs::Panel>,
    schema_panels: HashMap<String, Entity<schema_diff::Panel>>,
    properties: Option<properties::Properties>,
    structure_review_slot: std::sync::Arc<tokio::sync::Semaphore>,
    import_dialog: Option<WeakEntity<import::Dialog>>,
    import_picker: import::Picker,
    export_menu: Option<results::ExportMenu>,
    field_cache: std::cell::RefCell<results::FieldCache>,
    session: Rc<Session>,
    navigation: sidebar::Navigation,
    editors: HashMap<Owner, LiveEditor>,
    save_timer: Option<Task<()>>,
    focus: FocusHandle,
    resize_origin: (Pixels, f32),
    sidebar_width: f32,
    editor_fraction: f32,
    palette: Option<palette::Palette>,
    toasts: toast::Toasts,
    elapsed_timer: Option<Task<()>>,
    saving_query: bool,
    _storage_status: Task<()>,
    _composition_shortcuts: Subscription,
}

struct PreparedEditorText {
    editor: SharedString,
    snapshot: std::sync::Arc<str>,
}

impl Workspace {
    pub fn new(session: Rc<Session>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut updates = session.updates.observe_state();
        session.updates.check(rsql_desktop::update::Origin::Startup);
        let mut resources = session.resources.updates();
        session.resources.set_visible(window.is_visible());
        let mut status = session.save_status();
        let mut history = session.history_updates();
        let mut saved = session.saved_updates();
        let mut workspaces = session.named_workspace_updates();
        let storage_status = cx.spawn(async move |view, cx| {
            loop {
                let change = tokio::select! {
                    change = updates.changed() => change,
                    change = resources.changed() => change,
                    change = history.changed() => change,
                    change = status.changed() => change,
                    change = saved.changed() => change,
                    change = workspaces.changed() => change,
                };
                if change.is_err() {
                    break;
                }
                if view.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        });
        let mut view = Self {
            erd_panels: Default::default(),
            monitors: HashMap::new(),
            menu_bar: (!cfg!(target_os = "macos"))
                .then(|| gpui_kit::component::menu::AppMenuBar::new(cx)),
            _visibility: cx.observe_window_visibility(window, |view, _, w, cx| {
                view.session.resources.set_visible(w.is_visible());
                view.sync_monitors(w, cx);
                cx.notify();
            }),
            terminal_panels: HashMap::new(),
            admin_panels: HashMap::new(),
            notify_panels: HashMap::new(),
            schema_panels: HashMap::new(),
            properties: None,
            structure_review_slot: std::sync::Arc::new(tokio::sync::Semaphore::new(1)),
            import_dialog: None,
            import_picker: Rc::new(crate::desktop::DesktopServices::pick_csv),
            export_menu: None,
            field_cache: Default::default(),
            _storage_status: storage_status,
            _composition_shortcuts: Self::protect_composition(window, cx),
            resize_origin: Default::default(),
            sidebar_width: 280.,
            editor_fraction: 0.5,
            palette: None,
            toasts: toast::Toasts::new(window),
            elapsed_timer: None,
            saving_query: false,
            session,
            navigation: Default::default(),
            editors: HashMap::new(),
            save_timer: None,
            focus: cx.focus_handle(),
        };
        view.apply_theme(window, cx);
        view.sync_terminals(window, cx);
        view.sync_editors(window, cx);
        view
    }

    fn new_query(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        self.update_documents(w, cx, |d| {
            d.open(Kind::Query, None);
        });
    }

    fn toggle_theme(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        {
            let mut docs = self.session.documents.borrow_mut();
            docs.theme = match docs.theme {
                Appearance::Light => Appearance::Dark,
                Appearance::Dark => Appearance::Light,
            };
        }
        self.apply_theme(w, cx);
        self.sync_terminals(w, cx);
        self.schedule_save(cx);
        cx.notify();
    }

    fn apply_theme(&self, window: &mut Window, cx: &mut App) {
        let mode = match self.session.documents.borrow().theme {
            Appearance::Light => ThemeMode::Light,
            Appearance::Dark => ThemeMode::Dark,
        };
        theme::apply(mode, Some(window), cx);
    }

    fn schedule_save(&mut self, cx: &mut Context<Self>) {
        self.save_timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(350))
                .await;
            let _ = this.update(cx, |view, _| view.session.save());
        }));
    }

    fn update_documents(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        change: impl FnOnce(&mut Documents),
    ) {
        self.update_documents_with_initial(window, cx, None, change);
    }

    fn update_documents_with_initial(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        initial: Option<PreparedEditorText>,
        change: impl FnOnce(&mut Documents),
    ) {
        for live in self.editors.values_mut() {
            live.invalidate_language(cx);
        }
        self.export_menu = None;
        if initial.is_none() {
            self.capture_edits(cx);
        }
        change(&mut self.session.documents.borrow_mut());
        self.sync_terminals(window, cx);
        self.sync_erd_panels(window, cx);
        self.sync_schema_panels(cx);
        self.sync_admin_panels(window, cx);
        self.sync_notify_panels(window, cx);
        self.sync_monitors(window, cx);
        self.sync_editors_with_initial(window, cx, initial);
        self.schedule_save(cx);
        cx.notify();
    }

    pub fn capture_edits(&self, cx: &App) {
        let mut docs = self.session.documents.borrow_mut();
        for ((id, side), editor) in &self.editors {
            if let Some(tab) = docs.tabs.iter_mut().find(|t| &t.id == id) {
                tab.pane_mut(*side).draft = Self::read_draft(editor.state.read(cx));
            }
        }
    }

    fn read_draft(state: &EditorState) -> Draft {
        Draft {
            text: state.text().to_string(),
            selection: state.selected_range(),
            reversed: state.is_selection_reversed(),
        }
    }

    fn sync_editors(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_editors_with_initial(window, cx, None);
    }

    fn sync_editors_with_initial(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        mut initial: Option<PreparedEditorText>,
    ) {
        let docs = self.session.documents.borrow();
        self.editors.retain(|(id, side), live| {
            let keep = docs
                .tabs
                .iter()
                .any(|tab| tab.id == *id && (side == &Side::Main || tab.split));
            if !keep && let Some(map) = &live.map {
                map.update(cx, |v, cx| v.suspend(window, cx));
            }
            if !keep && let Some(panel) = &live.history {
                panel.update(cx, |p, _| p.suspend());
            }
            keep
        });
        let Some(tab) = docs.active().filter(|t| t.kind == Kind::Query) else {
            if let Some(panel) = docs
                .active()
                .and_then(|t| self.monitors.get(&t.id))
                .filter(|p| !p.read(cx).retired)
            {
                panel.read(cx).focus.clone().focus(window, cx);
            } else if let Some(Ok(view)) = docs
                .active()
                .and_then(|tab| self.terminal_panels.get(&tab.id))
            {
                view.focus_handle(cx).focus(window, cx);
            } else {
                self.focus.focus(window, cx);
            }
            return;
        };
        for side in [Side::Main, Side::Right] {
            if side == Side::Right && !tab.split {
                continue;
            }
            let owner = (tab.id.clone(), side);
            if self.editors.contains_key(&owner) {
                continue;
            }
            let draft = &tab.pane(side).draft;
            let (value, snapshot) = match initial.take() {
                Some(prepared) => (prepared.editor, Some(prepared.snapshot)),
                None => (draft.text.clone().into(), None),
            };
            let state = cx.new(|cx| {
                let mut state = EditorState::new(window, cx)
                    .language("sql")
                    .searchable(true)
                    .line_number(true)
                    .default_value(value);
                state.set_selected_range_with_direction(
                    draft.selection.clone(),
                    draft.reversed,
                    cx,
                );
                state
            });
            let id = tab.id.clone();
            let observed_owner = owner.clone();
            let mut observed_text = state.read(cx).text().clone();
            let observe = cx.observe_in(&state, window, move |view, editor, window, cx| {
                let state = editor.read(cx);
                let selection = state.selected_range();
                let reversed = state.is_selection_reversed();
                let edited = !editor_services::same_text(&observed_text, state.text());
                if edited {
                    observed_text = state.text().clone();
                }
                {
                    let mut docs = view.session.documents.borrow_mut();
                    let Some(tab) = docs.tabs.iter_mut().find(|t| t.id == id) else {
                        return;
                    };
                    let original = &mut tab.pane_mut(side).draft;
                    let moved = original.selection != selection || original.reversed != reversed;
                    if edited {
                        original.text = observed_text.to_string();
                    }
                    if let Some(live) = view.editors.get_mut(&observed_owner) {
                        live.synchronize_snippet(cx);
                    }
                    if !edited && !moved {
                        return;
                    }
                    original.selection = selection;
                    original.reversed = reversed;
                }
                view.analysis_for(&observed_owner, window, cx);
                view.schedule_save(cx);
                cx.notify();
            });
            let language = Self::attach_services(owner.clone(), &state, snapshot, window, cx);
            self.editors.insert(
                owner,
                LiveEditor {
                    state,
                    _observe: observe,
                    expansion: Default::default(),
                    history: None,
                    search: None,
                    export_focus: cx.focus_handle(),
                    grid: None,
                    map: None,
                    result_updates: None,
                    grid_events: vec![],
                    controls: Default::default(),
                    language,
                },
            );
        }
        let owner = (tab.id.clone(), tab.focused_side);
        drop(docs);
        self.focus_editor(owner.1, window, cx);
        // The focus listener handles panes focused after the window mounts.
        let view = cx.weak_entity();
        window.defer(cx, move |w, cx| {
            let _ = view.update(cx, |v, cx| {
                v.analysis_for(&owner, w, cx);
            });
        });
    }

    fn editor(&self, side: Side) -> Option<&Entity<EditorState>> {
        let docs = self.session.documents.borrow();
        self.editors
            .get(&(docs.active()?.id.clone(), side))
            .map(|e| &e.state)
    }
    fn shell_action<A>(
        cx: &Context<Self>,
        action: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> impl Fn(&A, &mut Window, &mut App) + 'static {
        cx.listener(move |view, _, window, cx| {
            if view.palette.is_none()
                && !view.navigation.modal_open
                && view.properties.is_none()
                && !view.importing()
                && !view.saving_query
                && !window.has_active_dialog(cx)
            {
                action(view, window, cx);
            }
        })
    }
    fn focus_editor(&self, side: Side, window: &mut Window, cx: &mut App) {
        if let Some(state) = self.editor(side).or_else(|| self.editor(Side::Main)) {
            state.update(cx, |s, cx| s.focus(window, cx));
        }
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self
            .properties
            .as_ref()
            .is_some_and(|p| !p.capture.current(self) && !p.preserve())
        {
            self.dismiss_properties(window, cx);
        }
        if self.navigation.modal_open && self.navigation.form.is_none() {
            self.navigation.modal_open = false;
            window.defer(cx, |w, cx| w.close_dialog(cx));
        }
        use gpui_kit::component::ActiveTheme;
        self.sync_erd_panels(window, cx);
        self.sync_schema_panels(cx);
        self.sync_admin_panels(window, cx);
        self.sync_notify_panels(window, cx);
        self.sync_monitors(window, cx);
        self.sync_diff(cx);
        self.sync_grids(window, cx);
        self.sync_history(window, cx);
        self.sync_export_menu(window, cx);
        let shell = self.shell(window, cx);
        let palette = self.palette_view(cx);
        clipboard_order::capture(div())
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .relative()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .key_context("RsqlWorkspace")
            .on_action(self.check_updates_action(cx))
            .on_action(cx.listener(|this, _: &TogglePalette, w, cx| this.toggle_palette(w, cx)))
            .on_action(Self::shell_action::<FocusEditor>(cx, |this, w, cx| {
                this.focus_editor(Side::Main, w, cx)
            }))
            .on_action(Self::shell_action::<FindReferences>(
                cx,
                Self::find_references,
            ))
            .on_action(Self::shell_action::<NewQuery>(cx, |this, w, cx| {
                this.new_query(w, cx)
            }))
            .on_action(Self::shell_action::<OpenTerminal>(cx, Self::open_terminal))
            .on_action(Self::shell_action::<CloseTab>(cx, |this, w, cx| {
                this.update_documents(w, cx, |d| {
                    if let Some(t) = d.active() {
                        d.close(&t.id.clone());
                    }
                })
            }))
            .on_action(Self::shell_action::<NextTab>(cx, |this, w, cx| {
                this.update_documents(w, cx, |d| {
                    if let Some(i) = d.selected {
                        d.select_index((i + 1) % d.tabs.len());
                    }
                })
            }))
            .on_action(Self::shell_action::<ToggleSplit>(cx, |this, w, cx| {
                this.update_documents(w, cx, |d| {
                    if let Some(i) = d.selected {
                        d.tabs[i].toggle_split();
                    }
                })
            }))
            .on_action(Self::shell_action::<ToggleTheme>(cx, |this, w, cx| {
                this.toggle_theme(w, cx)
            }))
            .on_action(Self::shell_action::<RunStatement>(cx, |v, w, cx| {
                v.keyboard_run(execution::Action::Run, w, cx)
            }))
            .on_action(Self::shell_action::<RunScript>(cx, |v, w, cx| {
                v.keyboard_run(execution::Action::Script, w, cx)
            }))
            .on_action(Self::shell_action::<ExplainStatement>(cx, |v, w, cx| {
                v.keyboard_run(execution::Action::Explain, w, cx)
            }))
            .on_action(Self::shell_action::<ExplainMain>(cx, |v, w, cx| {
                let id = v.session.documents.borrow().active().map(|t| t.id.clone());
                if let Some(id) = id {
                    v.run_query((id, Side::Main), execution::Action::Explain, w, cx);
                }
            }))
            .on_action(Self::shell_action::<CancelQuery>(cx, |v, w, cx| {
                let editing = w.context_stack().iter().any(|c| c.contains("Input"));
                if !editing || v.keyboard_owner(w, cx).is_some() {
                    let id = v.session.documents.borrow().active().map(|t| t.id.clone());
                    if let Some(id) = id {
                        v.cancel_tab_query(&id, cx);
                    }
                }
            }))
            .children(
                self.menu_bar
                    .as_ref()
                    .map(|bar| div().h(px(28.)).flex_shrink_0().child(bar.clone())),
            )
            .child(shell)
            .children(palette)
    }
}
