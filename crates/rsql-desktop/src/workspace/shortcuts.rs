use super::*;

impl Workspace {
    pub(super) fn protect_composition(window: &Window, cx: &mut Context<Self>) -> Subscription {
        let owner = cx.weak_entity();
        let handle = window.window_handle();
        // Native menus and buttons dispatch the same actions directly. Only suppress key bindings.
        cx.intercept_keystrokes(move |event, w, cx| {
            if w.window_handle() != handle || event.input_preference != InputPreference::KeyBindings
            {
                return;
            }
            let Some(view) = owner.upgrade() else { return };
            let view = view.read(cx);
            let composing = if let Some(editor) = view.focused_owner(w, cx) {
                let editor = view.editors[&editor].state.clone();
                editor.update(cx, |s, cx| s.marked_text_range(w, cx).is_some())
            } else {
                view.session
                    .documents
                    .borrow()
                    .active()
                    .and_then(|tab| view.terminal_panels.get(&tab.id))
                    .and_then(|p| p.as_ref().ok())
                    .is_some_and(|terminal| terminal.read(cx).has_marked_text(w))
            };
            if !composing {
                return;
            }
            let (bindings, _) = cx
                .key_bindings()
                .borrow()
                .bindings_for_input(std::slice::from_ref(&event.keystroke), &event.context_stack);
            if let Some(binding) = bindings.first() {
                let action = binding.action().as_any();
                if action.is::<OpenTerminal>()
                    || action.is::<NewQuery>()
                    || action.is::<CloseTab>()
                    || action.is::<TogglePalette>()
                {
                    cx.stop_propagation();
                }
            }
        })
    }
}
