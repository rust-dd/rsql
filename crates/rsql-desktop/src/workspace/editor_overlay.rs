use super::*;
use super::{
    editor_services::{Services, Stamp},
    editor_snippet::Snippet,
};
use gpui_kit::component::{
    ActiveTheme,
    input::{Enter, Escape, IndentInline, MoveDown, MoveUp, OutdentInline},
};
use rsql_desktop::completion::Suggestion;
use std::ops::Range;

#[path = "suggestion_menu.rs"]
mod suggestion_menu;

pub(super) enum Rows {
    Suggestions {
        items: Vec<Suggestion>,
        explicit: bool,
        prefix: String,
    },
    References(Vec<(String, Range<usize>)>),
}
pub(super) struct Menu {
    stamp: Stamp,
    generation: u64,
    rows: Rc<Rows>,
    selected: usize,
    scroll: UniformListScrollHandle,
}
impl Menu {
    pub fn new(stamp: Stamp, generation: u64, rows: Rows) -> Self {
        Self {
            stamp,
            generation,
            rows: Rc::new(rows),
            selected: 0,
            scroll: UniformListScrollHandle::new(),
        }
    }
    fn len(&self) -> usize {
        match &*self.rows {
            Rows::Suggestions { items, .. } => items.len(),
            Rows::References(rows) => rows.len(),
        }
    }
}

impl Services {
    fn presented_menu(&self) -> Option<&Menu> {
        self.menu
            .as_ref()
            .filter(|menu| self.presentation.matches(menu.generation))
    }
}

#[derive(Copy, Clone)]
enum Command {
    Enter,
    Tab,
    Backtab,
    Escape,
    Up,
    Down,
}

impl Workspace {
    pub(super) fn focused_owner(&self, w: &Window, cx: &App) -> Option<Owner> {
        let tab = self.session.documents.borrow().active()?.id.clone();
        [Side::Main, Side::Right]
            .into_iter()
            .map(|s| (tab.clone(), s))
            .find(|owner| {
                self.editors
                    .get(owner)
                    .is_some_and(|e| e.state.read(cx).focus_handle(cx).is_focused(w))
            })
    }

    pub(super) fn current_language_menu<'a>(
        &self,
        live: &'a LiveEditor,
        w: &Window,
        cx: &App,
    ) -> Option<&'a Menu> {
        live.language
            .presented_menu()
            .filter(|menu| self.is_current(&menu.stamp, w, cx))
    }

    fn accept(
        &mut self,
        stamp: &Stamp,
        row: usize,
        smart: bool,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.is_current(stamp, w, cx) {
            return false;
        }
        let live = self.editors.get_mut(&stamp.owner).unwrap();
        if live
            .state
            .update(cx, |s, cx| s.marked_text_range(w, cx).is_some())
        {
            return false;
        }
        let Some(menu) = live.language.presented_menu() else {
            return false;
        };
        if &menu.stamp != stamp || row >= menu.len() {
            return false;
        }
        let menu = live.language.menu.take().unwrap();
        live.language.presentation.dismiss();
        let accepted = match &*menu.rows {
            Rows::References(rows) => {
                live.state
                    .update(cx, |s, cx| s.set_selected_range(rows[row].1.clone(), cx));
                true
            }
            Rows::Suggestions {
                items, explicit, ..
            } => {
                let item = &items[row];
                live.state.update(cx, |s, cx| {
                    if smart
                        && !explicit
                        && !item.is_snippet
                        && s.text().slice(item.range.clone()) == item.insert_text.as_str()
                    {
                        return false;
                    }
                    s.set_selected_range(item.range.clone(), cx);
                    if item.is_snippet {
                        let (text, mut snippet) =
                            Snippet::expand(&item.insert_text, item.range.start);
                        s.replace(text, w, cx);
                        live.language.snippet = snippet.begin(s, cx).then_some(snippet);
                    } else {
                        s.replace(item.insert_text.clone(), w, cx);
                    }
                    true
                })
            }
        };
        cx.notify();
        accepted
    }

    fn editor_command(&mut self, command: Command, w: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(owner) = self.focused_owner(w, cx) else {
            return false;
        };
        let Some(stamp) = self.stamp(&owner, w, cx) else {
            return false;
        };
        let live = self.editors.get_mut(&owner).unwrap();
        if live
            .state
            .update(cx, |s, cx| s.marked_text_range(w, cx).is_some())
        {
            return false;
        }
        let owns_menu = live
            .language
            .presented_menu()
            .is_some_and(|menu| menu.stamp == stamp);
        if matches!(command, Command::Escape)
            && (live.language.snippet.is_some()
                || live.language.presentation.pending()
                || owns_menu)
        {
            live.language.presentation.dismiss();
            live.language.menu = None;
            if live.language.snippet.take().is_some() {
                live.state.update(cx, |s, cx| {
                    s.set_selected_range_with_direction(
                        s.selected_range(),
                        s.is_selection_reversed(),
                        cx,
                    );
                });
            }
            cx.notify();
            return true;
        }
        if matches!(command, Command::Tab | Command::Backtab)
            && let Some(snippet) = &mut live.language.snippet
        {
            let keep = live.state.update(cx, |s, cx| {
                snippet.step(matches!(command, Command::Backtab), s, cx)
            });
            if keep != Some(true) {
                live.language.snippet = None;
            }
            cx.notify();
            return keep.is_some();
        }
        let Some(menu) = live.language.menu.as_mut().filter(|_| owns_menu) else {
            return false;
        };
        match command {
            Command::Enter | Command::Tab => {
                let row = menu.selected;
                return self.accept(&stamp, row, matches!(command, Command::Enter), w, cx);
            }
            Command::Escape => unreachable!(),
            Command::Up | Command::Down => {
                let count = menu.len();
                let step = if matches!(command, Command::Up) {
                    count - 1
                } else {
                    1
                };
                menu.selected = (menu.selected + step) % count;
                menu.scroll
                    .scroll_to_item(menu.selected, ScrollStrategy::Nearest);
            }
            Command::Backtab => return false,
        }
        cx.notify();
        true
    }

    fn command_listener<A>(
        &self,
        command: Command,
        cx: &Context<Self>,
    ) -> impl Fn(&A, &mut Window, &mut App) + 'static {
        cx.listener(move |v, _, w, cx| {
            if v.editor_command(command, w, cx) {
                cx.stop_propagation();
            } else {
                cx.propagate();
            }
        })
    }

    pub(super) fn language_editor(
        &self,
        state: &Entity<EditorState>,
        side: Side,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let composing = state.update(cx, |s, cx| s.marked_text_range(w, cx).is_some());
        let menu = self
            .session
            .documents
            .borrow()
            .active()
            .and_then(|t| self.editors.get(&(t.id.clone(), side)))
            .and_then(|live| {
                let presentation = &live.language.presentation;
                if composing {
                    if presentation.pending() || live.language.presented_menu().is_some() {
                        presentation.dismiss();
                    }
                    return None;
                }
                self.current_language_menu(live, w, cx)
            });
        let paint = theme::editor::local_paint(cx.theme().mode.is_dark());
        let preview = menu.and_then(|menu| menu.preview(state.read(cx), cx));
        state.update(cx, |state, cx| {
            state.set_editor_paint(Some(paint));
            state.set_inline_preview(preview, cx);
        });
        div()
            .relative()
            .size_full()
            .capture_action(cx.listener(|v, action: &Enter, w, cx| {
                if !action.secondary && !action.shift && v.editor_command(Command::Enter, w, cx) {
                    cx.stop_propagation();
                } else {
                    cx.propagate();
                }
            }))
            .capture_action(self.command_listener::<IndentInline>(Command::Tab, cx))
            .capture_action(self.command_listener::<OutdentInline>(Command::Backtab, cx))
            .capture_action(self.command_listener::<MoveUp>(Command::Up, cx))
            .capture_action(self.command_listener::<MoveDown>(Command::Down, cx))
            .capture_action(self.command_listener::<Escape>(Command::Escape, cx))
            .child(
                Editor::new(state)
                    .h_full()
                    .bordered(false)
                    .rounded_none()
                    .font_features(FontFeatures::disable_ligatures())
                    .text_color(
                        cx.theme()
                            .highlight_theme
                            .style
                            .editor_foreground
                            .unwrap_or(cx.theme().foreground),
                    ),
            )
            .children(menu.map(|menu| self.language_menu(state.read(cx), menu, cx)))
    }
}
