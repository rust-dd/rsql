use super::{TerminalView, composition::Composition};
use crate::{
    clipboard_order::ClipboardWrite,
    terminal::{
        Scroll, TermMode,
        input::key_bytes_for_platform,
        session::{CommandError, Edit, EditAdmission, MAX_INPUT_BYTES, SessionPhase, ViewCommand},
    },
};
use gpui_kit::{
    component::input::{Copy, Paste, SelectAll},
    *,
};

pub(super) struct Input {
    pub focus: FocusHandle,
    pub composition: Composition,
    pub notice: Option<&'static str>,
    pub(super) pointer: super::pointer::Pointer,
    _focus: Subscription,
    pub(super) copy: Option<Task<()>>,
    _blur: Subscription,
}

#[derive(Clone, PartialEq, Action)]
#[action(namespace = rsql, no_json)]
struct TerminalKey(&'static str);

struct Bound;
impl Global for Bound {}

impl Input {
    pub fn new(w: &mut Window, cx: &mut Context<TerminalView>) -> Self {
        if !cx.has_global::<Bound>() {
            cx.set_global(Bound);
            let context = Some("RsqlTerminal");
            for stroke in [
                "tab",
                "shift-tab",
                "f6",
                "escape",
                "ctrl-c",
                "ctrl-z",
                "ctrl-q",
                "ctrl-d",
            ] {
                cx.bind_keys([KeyBinding::new(stroke, TerminalKey(stroke), context)]);
            }
            let (copy, paste) = if cfg!(target_os = "macos") {
                ("cmd-c", "cmd-v")
            } else {
                ("ctrl-shift-c", "ctrl-shift-v")
            };
            if cfg!(target_os = "macos") {
                cx.bind_keys([KeyBinding::new("cmd-a", SelectAll, context)]);
            }
            cx.bind_keys([
                KeyBinding::new(copy, Copy, context),
                KeyBinding::new(paste, Paste, context),
            ]);
        }
        let focus = cx.focus_handle();
        let focus_change = cx.on_focus(&focus, w, |v, w, cx| v.sync_focus(w, cx));
        let blur = cx.on_blur(&focus, w, |view, w, cx| {
            view.sync_focus(w, cx);
            view.input.reset();
            cx.notify();
        });
        Self {
            focus,
            composition: Default::default(),
            pointer: Default::default(),
            _focus: focus_change,
            copy: None,
            notice: None,
            _blur: blur,
        }
    }

    pub fn reset(&mut self) {
        self.composition = Default::default();
        self.pointer.reset_local();
    }
}

impl Focusable for TerminalView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.input.focus.clone()
    }
}

impl TerminalView {
    /// Whether this pane owns keyboard dispatch, including shortcuts routed by its workspace.
    pub fn owns_focus(&self, w: &Window) -> bool {
        self.shown()
            && w.is_window_active()
            && self.input.focus.is_focused(w)
            && self.session.accepts(&self.frame)
    }

    /// Lets workspace shortcuts defer to an active native composition.
    pub fn has_marked_text(&self, w: &Window) -> bool {
        self.owns_focus(w) && !self.input.composition.is_empty()
    }

    pub(super) fn can_type(&self, w: &Window) -> bool {
        self.owns_focus(w)
            && matches!(
                self.frame.phase,
                SessionPhase::Starting | SessionPhase::Running
            )
    }

    pub(super) fn mode(&self) -> TermMode {
        self.session.input_mode()
    }

    pub(super) fn input_events<T: StatefulInteractiveElement>(
        &self,
        pane: T,
        cx: &Context<Self>,
    ) -> T {
        pane.key_context("RsqlTerminal")
            .track_focus(&self.input.focus)
            .on_key_down(cx.listener(|v, event: &KeyDownEvent, w, cx| {
                let character = event
                    .keystroke
                    .key_char
                    .as_deref()
                    .is_some_and(|s| !s.is_empty() && s.chars().all(|c| !c.is_control()));
                if !event.prefer_character_input || !character {
                    v.key(&event.keystroke, w, cx);
                }
            }))
            .on_action(cx.listener(|v, key: &TerminalKey, w, cx| {
                v.key(&Keystroke::parse(key.0).expect("terminal binding"), w, cx);
            }))
            .on_action(cx.listener(|v, _: &SelectAll, w, cx| {
                if v.owns_focus(w) && v.input.composition.is_empty() {
                    v.select_all(cx);
                }
            }))
            .on_action(cx.listener(|v, _: &Copy, w, cx| v.copy(w, cx)))
            .on_action(cx.listener(|v, _: &Paste, w, cx| {
                if v.can_type(w)
                    && v.input.composition.is_empty()
                    && let Some(item) = crate::platform::DesktopServices::read_clipboard(cx)
                {
                    v.paste_item(item, cx);
                }
            }))
            .on_any_mouse_down(cx.listener(Self::pointer_down))
            .on_mouse_move(cx.listener(Self::pointer_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::pointer_up))
            .on_mouse_up(MouseButton::Middle, cx.listener(Self::pointer_up))
            .on_mouse_up(MouseButton::Right, cx.listener(Self::pointer_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::pointer_up))
            .on_mouse_up_out(MouseButton::Middle, cx.listener(Self::pointer_up))
            .on_mouse_up_out(MouseButton::Right, cx.listener(Self::pointer_up))
    }

    fn key(&mut self, key: &Keystroke, w: &mut Window, cx: &mut Context<Self>) {
        self.key_for_platform(key, cfg!(target_os = "macos"), w, cx);
    }

    pub(super) fn key_for_platform(
        &mut self,
        key: &Keystroke,
        mac: bool,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.owns_focus(w) {
            return;
        }
        if !self.input.composition.is_empty() {
            if key.key == "escape" {
                self.input.composition = Default::default();
                w.notify_text_input_changed(&self.input.focus);
                cx.notify();
            } else if key_bytes_for_platform(key, self.mode(), mac).is_none() {
                return;
            }
        } else if key.modifiers.shift && matches!(key.key.as_str(), "pageup" | "pagedown") {
            let scroll = if key.key == "pageup" {
                Scroll::PageUp
            } else {
                Scroll::PageDown
            };
            self.admission(self.session.view(ViewCommand::Scroll(scroll)), cx);
        } else if let Some(bytes) = key_bytes_for_platform(key, self.mode(), mac) {
            self.apply_edit(Edit::Text(&bytes), cx);
        } else {
            return;
        }
        cx.stop_propagation();
    }

    fn copy(&mut self, w: &Window, cx: &mut Context<Self>) {
        if !self.owns_focus(w) || !self.input.composition.is_empty() {
            return;
        }
        if self.input.pointer.interrupted {
            self.selection_interrupted(cx);
            return;
        }
        let order = ClipboardWrite::begin(cx);
        let request = match self.session.selected_text() {
            Ok(request) => request,
            Err(e) => {
                self.admission(Err(e), cx);
                return;
            }
        };
        let owner = self.session.owner().clone();
        self.input.copy = Some(cx.spawn(async move |view, cx| {
            let text = request.await;
            let _ = view.update(cx, |v, cx| {
                if order.current() && v.session.owner() == &owner && v.session.accepts(&v.frame) {
                    match text {
                        Ok(Some(text)) => crate::platform::DesktopServices::write_clipboard(
                            cx,
                            ClipboardItem::new_string(text),
                        ),
                        Ok(None) => {}
                        Err(CommandError::TooLarge) => {
                            v.input.notice = Some("Selection exceeds the 4 MiB copy limit.");
                            cx.notify();
                        }
                        Err(e) => {
                            v.admission(Err(e), cx);
                        }
                    }
                }
            });
        }));
    }

    pub(super) fn paste_item(&mut self, item: ClipboardItem, cx: &mut Context<Self>) {
        let bracketed = self.mode().contains(TermMode::BRACKETED_PASTE);
        let limit = MAX_INPUT_BYTES - if bracketed { 12 } else { 0 };
        let mut bytes = Vec::new();
        for entry in item.entries {
            if let ClipboardEntry::String(part) = entry {
                if part.text().len() > limit - bytes.len() {
                    self.admission(Err(CommandError::TooLarge), cx);
                    return;
                }
                bytes.extend_from_slice(part.text().as_bytes());
            }
        }
        if !bytes.is_empty() {
            self.apply_edit(
                Edit::Paste {
                    text: bytes,
                    bracketed,
                },
                cx,
            );
        }
    }

    pub(super) fn flush_before_input(&mut self, cx: &mut Context<Self>) -> bool {
        if self.pending.reports.is_pending() {
            self.flush(cx);
            if self.pending.reports.is_pending() {
                return self.admission(Err(CommandError::Busy), cx);
            }
        }
        true
    }

    pub(super) fn apply_edit(&mut self, edit: Edit<'_>, cx: &mut Context<Self>) -> bool {
        let committed = !matches!(edit, Edit::Preedit);
        if committed && !self.flush_before_input(cx) {
            return false;
        }
        let admission = match self.session.edit(edit) {
            Ok(EditAdmission::ObsoleteScroll) => return false,
            Ok(admission) => admission,
            Err(error) => return self.admission(Err(error), cx),
        };
        if committed {
            self.input.pointer.stop_selection();
            self.input.pointer.interrupted = false;
        }
        if admission == EditAdmission::Primary {
            self.pending.scroll = Default::default();
        }
        self.fractional_scroll = 0.;
        if self.input.notice.take().is_some() {
            cx.notify();
        }
        true
    }

    pub(super) fn admission(
        &mut self,
        result: Result<(), CommandError>,
        cx: &mut Context<Self>,
    ) -> bool {
        if let Err(e) = result {
            self.input.notice = Some(match e {
                CommandError::Closed => "Terminal input is closed.",
                CommandError::Busy => "Terminal is busy. Retry the last action.",
                CommandError::TooLarge => "Terminal input exceeds 16 KiB including paste markers.",
            });
            cx.notify();
            false
        } else {
            true
        }
    }
}
