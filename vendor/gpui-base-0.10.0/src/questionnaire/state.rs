use std::{collections::HashSet, time::Duration};

use crate::input::{InputEvent, InputState};
use gpui::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable as _, SharedString, Subscription,
    Task, Window,
};

use super::types::*;

/// How long a newly chosen single answer stays on screen, selected, before the
/// item confirms. Long enough to register the selection, short enough not to
/// read as a wait.
const CHOICE_CONFIRM_DELAY: Duration = Duration::from_millis(150);

struct ItemRuntime {
    disabled: bool,
    choice_disabled: Vec<bool>,
    input_disabled: bool,
    answer: QuestionnaireAnswer,
    initial_answer: QuestionnaireAnswer,
    initial_input_value: Option<SharedString>,
    skipped: bool,
    validation_attempted: bool,
    internal_error: Option<QuestionnaireValidationError>,
    external_error: Option<QuestionnaireValidationError>,
    focus_handle: FocusHandle,
    choice_focus_handles: Vec<FocusHandle>,
    input_focus_handle: Option<FocusHandle>,
}

/// Owns questionnaire answers, validation, navigation and focus state.
pub struct QuestionnaireState {
    items: Vec<QuestionnaireItemDefinition>,
    runtime: Vec<ItemRuntime>,
    current: Option<usize>,
    initial_current: Option<SharedString>,
    shortcut_mode: Option<QuestionnaireShortcutMode>,
    complete: bool,
    /// The confirm a chosen single answer scheduled. Changing an answer or the
    /// current item, or confirming by any other path, drops it first.
    pending_confirm: Option<Task<()>>,
    focus_handle: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl QuestionnaireState {
    pub fn new(
        items: Vec<QuestionnaireItemDefinition>,
        cx: &mut Context<Self>,
    ) -> Result<Self, QuestionnaireSchemaError> {
        Self::validate_schema(&items)?;

        let mut runtime = Vec::with_capacity(items.len());
        let mut subscriptions = Vec::new();

        for item in &items {
            let mut answer = QuestionnaireAnswer::new().with_choices(
                item.choices()
                    .iter()
                    .filter(|choice| choice.is_default_selected() && !choice.is_disabled())
                    .map(|choice| choice.value().clone()),
            );

            let mut initial_input_value = None;
            let mut input_focus_handle = None;
            if let Some(input) = item.input() {
                let state = input.state();
                state.update(cx, |state, cx| {
                    state.set_disabled(item.is_disabled() || input.is_disabled(), cx);
                });

                let value = state.read(cx).value();
                initial_input_value = Some(value.clone());
                input_focus_handle = Some(state.focus_handle(cx));
                if !value.trim().is_empty() && !input.is_disabled() {
                    if !item.is_multiple() {
                        answer = QuestionnaireAnswer::new();
                    }
                    answer.freeform = Some(value);
                }

                subscriptions.push(cx.subscribe(state, |this, input, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        this.on_input_change(&input, cx);
                    }
                }));
            }

            runtime.push(ItemRuntime {
                disabled: item.is_disabled(),
                choice_disabled: item
                    .choices()
                    .iter()
                    .map(QuestionnaireChoiceDefinition::is_disabled)
                    .collect(),
                input_disabled: item
                    .input()
                    .is_none_or(QuestionnaireInputDefinition::is_disabled),
                initial_answer: answer.clone(),
                initial_input_value,
                answer,
                skipped: false,
                validation_attempted: false,
                internal_error: None,
                external_error: None,
                focus_handle: cx.focus_handle(),
                choice_focus_handles: item.choices().iter().map(|_| cx.focus_handle()).collect(),
                input_focus_handle,
            });
        }

        let current = runtime.iter().position(|item| !item.disabled);
        let initial_current = current.map(|ix| items[ix].name().clone());

        Ok(Self {
            items,
            runtime,
            current,
            initial_current,
            shortcut_mode: None,
            complete: false,
            pending_confirm: None,
            focus_handle: cx.focus_handle(),
            _subscriptions: subscriptions,
        })
    }

    fn validate_schema(
        items: &[QuestionnaireItemDefinition],
    ) -> Result<(), QuestionnaireSchemaError> {
        let mut item_names = HashSet::new();
        for item in items {
            if !item_names.insert(item.name().to_string()) {
                return Err(QuestionnaireSchemaError::DuplicateItem(item.name().clone()));
            }

            let mut choices = HashSet::new();
            let mut defaults = 0;
            for choice in item.choices() {
                if !choices.insert(choice.value().to_string()) {
                    return Err(QuestionnaireSchemaError::DuplicateChoice {
                        item: item.name().clone(),
                        choice: choice.value().clone(),
                    });
                }
                defaults += usize::from(choice.is_default_selected());
            }
            if !item.is_multiple() && defaults > 1 {
                return Err(QuestionnaireSchemaError::MultipleDefaultsForSingleItem(
                    item.name().clone(),
                ));
            }
        }
        Ok(())
    }

    pub fn with_current_item(
        mut self,
        name: impl Into<SharedString>,
    ) -> Result<Self, QuestionnaireSchemaError> {
        let name = name.into();
        let ix = self.item_ix(&name)?;
        if !self.runtime[ix].disabled {
            self.current = Some(ix);
            self.initial_current = Some(name);
        }
        Ok(self)
    }

    pub fn with_shortcuts(mut self, mode: QuestionnaireShortcutMode) -> Self {
        self.shortcut_mode = Some(mode);
        self
    }

    pub fn current_item(&self) -> Option<&SharedString> {
        self.current.map(|ix| self.items[ix].name())
    }

    pub fn current_ix(&self) -> Option<usize> {
        let current = self.current?;
        self.enabled_indices().position(|ix| ix == current)
    }

    pub fn total(&self) -> usize {
        self.enabled_indices().count()
    }

    pub fn progress(&self) -> QuestionnaireProgressState {
        QuestionnaireProgressState::new(self.current_ix().map_or(0, |ix| ix + 1), self.total())
    }

    pub fn item_definition(&self, name: &str) -> Option<&QuestionnaireItemDefinition> {
        self.items.iter().find(|item| item.name().as_ref() == name)
    }

    pub fn choice_definition(
        &self,
        item: &str,
        value: &str,
    ) -> Option<&QuestionnaireChoiceDefinition> {
        self.item_definition(item)?
            .choices()
            .iter()
            .find(|choice| choice.value().as_ref() == value)
    }

    pub fn item_state(&self, name: &str) -> Option<QuestionnaireItemState> {
        let ix = self.item_ix_opt(name)?;
        let definition = &self.items[ix];
        Some(QuestionnaireItemState::new(
            definition.name().clone(),
            self.status(ix),
            definition.is_required(),
            definition.is_multiple(),
            self.runtime[ix].disabled,
            self.error_at(ix).is_some(),
            definition.input().is_some(),
        ))
    }

    pub fn choice_state(&self, item: &str, value: &str) -> Option<QuestionnaireChoiceState> {
        let item_ix = self.item_ix_opt(item)?;
        let choice_ix = self.choice_ix_opt(item_ix, value)?;
        let runtime = &self.runtime[item_ix];
        let definition = &self.items[item_ix].choices()[choice_ix];
        Some(QuestionnaireChoiceState::new(
            definition.value().clone(),
            runtime.answer.choices.contains(definition.value()),
            runtime.disabled || runtime.choice_disabled[choice_ix],
            self.error_at(item_ix).is_some(),
            self.shortcut_for_choice(item, value),
        ))
    }

    /// One-based position of a choice among its item's enabled choices, with
    /// the enabled total. Assistive technology announces the pair.
    pub fn choice_position(&self, item: &str, value: &str) -> Option<(usize, usize)> {
        let definition = self.item_definition(item)?;
        let enabled: Vec<_> = definition
            .choices()
            .iter()
            .filter(|choice| {
                self.choice_state(item, choice.value())
                    .is_some_and(|choice| !choice.is_disabled())
            })
            .collect();
        enabled
            .iter()
            .position(|choice| choice.value().as_ref() == value)
            .map(|position| (position + 1, enabled.len()))
    }

    pub fn navigation_state(&self) -> QuestionnaireNavigationState {
        let Some(ix) = self.current_ix() else {
            return QuestionnaireNavigationState::default();
        };
        let total = self.total();
        let item_ix = self
            .current
            .expect("current item exists when enabled index exists");
        QuestionnaireNavigationState::new(
            ix > 0,
            ix + 1 < total,
            !self.items[item_ix].is_required(),
            ix + 1 == total,
            self.status(item_ix) != QuestionnaireItemStatus::Unanswered,
        )
    }

    pub fn answer(&self, name: &str) -> Option<QuestionnaireAnswer> {
        let ix = self.item_ix_opt(name)?;
        Some(self.effective_answer(ix))
    }

    pub fn answers(&self) -> QuestionnaireAnswers {
        QuestionnaireAnswers::from_entries(
            self.enabled_indices()
                .map(|ix| (self.items[ix].name().clone(), self.effective_answer(ix)))
                .collect(),
        )
    }

    /// The active validation failure for an item, if any. Base reports the
    /// reason; the presentation layer turns a built-in reason into text.
    pub fn error(&self, name: &str) -> Option<&QuestionnaireValidationError> {
        self.item_ix_opt(name).and_then(|ix| self.error_at(ix))
    }

    pub fn is_complete(&self) -> bool {
        self.complete
    }

    pub fn input_state(&self, name: &str) -> Option<Entity<InputState>> {
        self.item_definition(name)?
            .input()
            .map(|input| input.state().clone())
    }

    pub fn focus_handle(&self) -> &FocusHandle {
        &self.focus_handle
    }

    pub fn item_focus_handle(&self, name: &str) -> Option<&FocusHandle> {
        self.item_ix_opt(name)
            .map(|ix| &self.runtime[ix].focus_handle)
    }

    pub fn choice_focus_handle(&self, item: &str, value: &str) -> Option<&FocusHandle> {
        let item_ix = self.item_ix_opt(item)?;
        let choice_ix = self.choice_ix_opt(item_ix, value)?;
        Some(&self.runtime[item_ix].choice_focus_handles[choice_ix])
    }

    pub fn is_current_input_focused(&self, window: &Window) -> bool {
        let Some(ix) = self.current else { return false };
        self.runtime[ix]
            .input_focus_handle
            .as_ref()
            .is_some_and(|handle| handle.is_focused(window))
    }

    /// Whether the active item's freeform input currently holds text. The skin
    /// needs it to decide whether an arrow key moves focus or edits the draft.
    pub fn current_input_has_text(&self, cx: &App) -> bool {
        let Some(item_ix) = self.current else {
            return false;
        };
        self.items[item_ix]
            .input()
            .is_some_and(|input| !input.state().read(cx).value().trim().is_empty())
    }

    pub fn focused_current_choice(&self, window: &Window) -> Option<&SharedString> {
        let ix = self.current?;
        self.runtime[ix]
            .choice_focus_handles
            .iter()
            .position(|handle| handle.is_focused(window))
            .map(|choice_ix| self.items[ix].choices()[choice_ix].value())
    }

    pub fn shortcut_mode(&self) -> Option<QuestionnaireShortcutMode> {
        self.shortcut_mode
    }

    pub fn shortcut_for_choice(&self, item: &str, value: &str) -> Option<SharedString> {
        let mode = self.shortcut_mode?;
        let item_ix = self.item_ix_opt(item)?;
        let choice_ix = self.choice_ix_opt(item_ix, value)?;
        if self.runtime[item_ix].disabled || self.runtime[item_ix].choice_disabled[choice_ix] {
            return None;
        }
        let enabled_position = (0..=choice_ix)
            .filter(|ix| !self.runtime[item_ix].choice_disabled[*ix])
            .count()
            .checked_sub(1)?;
        match mode {
            QuestionnaireShortcutMode::Letters if enabled_position < 26 => {
                Some(char::from(b'A' + enabled_position as u8).to_string().into())
            }
            QuestionnaireShortcutMode::Numbers if enabled_position < 9 => {
                Some((enabled_position + 1).to_string().into())
            }
            _ => None,
        }
    }

    pub fn choice_for_shortcut(&self, item: &str, key: &str) -> Option<&SharedString> {
        let item_ix = self.item_ix_opt(item)?;
        self.items[item_ix].choices().iter().find_map(|choice| {
            self.shortcut_for_choice(item, choice.value())
                .is_some_and(|shortcut| shortcut.as_ref().eq_ignore_ascii_case(key))
                .then_some(choice.value())
        })
    }

    pub fn activate_shortcut(
        &mut self,
        key: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(item_ix) = self.current else {
            return false;
        };
        let item = self.items[item_ix].name().clone();
        let Some(choice) = self.choice_for_shortcut(&item, key).cloned() else {
            return false;
        };
        self.choose(&item, &choice, window, cx).is_ok()
    }

    pub fn set_current_item(
        &mut self,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), QuestionnaireSchemaError> {
        let ix = self.item_ix(name)?;
        if !self.runtime[ix].disabled {
            self.pending_confirm = None;
            self.current = Some(ix);
            self.focus_current_item(window, cx);
            cx.notify();
        }
        Ok(())
    }

    pub fn set_answer(
        &mut self,
        item: &str,
        mut answer: QuestionnaireAnswer,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), QuestionnaireSchemaError> {
        let item_ix = self.item_ix(item)?;
        let before = self.effective_answer(item_ix);
        let before_status = self.status(item_ix);
        if answer
            .freeform
            .as_ref()
            .is_some_and(|value| value.trim().is_empty())
        {
            answer.freeform = None;
        }
        self.check_answer(item_ix, &answer)?;
        answer.choices = self.items[item_ix]
            .choices()
            .iter()
            .filter(|choice| answer.choices.contains(choice.value()))
            .map(|choice| choice.value().clone())
            .collect();
        self.runtime[item_ix].answer = answer.clone();
        self.runtime[item_ix].skipped = false;
        if let (Some(input), Some(value)) =
            (self.items[item_ix].input(), answer.freeform().cloned())
        {
            input
                .state()
                .update(cx, |input, cx| input.set_value(value, window, cx));
        }
        if before != self.effective_answer(item_ix) || before_status != self.status(item_ix) {
            self.answer_did_change(item_ix, false, cx);
        }
        Ok(())
    }

    pub fn set_input_value(
        &mut self,
        item: &str,
        value: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), QuestionnaireSchemaError> {
        let item_ix = self.item_ix(item)?;
        let Some(input) = self.items[item_ix]
            .input()
            .map(|input| input.state().clone())
        else {
            return Err(QuestionnaireSchemaError::AnswerDoesNotMatchItem(
                self.items[item_ix].name().clone(),
            ));
        };
        let value: SharedString = value.into();
        input.update(cx, |input, cx| input.set_value(value, window, cx));
        self.sync_input_answer(item_ix, false, cx);
        Ok(())
    }

    pub fn set_item_disabled(
        &mut self,
        name: &str,
        disabled: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), QuestionnaireSchemaError> {
        let ix = self.item_ix(name)?;
        if self.runtime[ix].disabled == disabled {
            return Ok(());
        }
        self.runtime[ix].disabled = disabled;
        self.pending_confirm = None;
        if let Some(input) = self.items[ix].input() {
            let input_disabled = disabled || self.runtime[ix].input_disabled;
            input
                .state()
                .update(cx, |input, cx| input.set_disabled(input_disabled, cx));
        }
        self.complete = false;
        if self.current == Some(ix) && disabled {
            let next = self
                .enabled_indices()
                .find(|candidate| *candidate > ix)
                .or_else(|| {
                    self.enabled_indices()
                        .rev()
                        .find(|candidate| *candidate < ix)
                });
            self.current = next;
            self.focus_current_item(window, cx);
        } else if self.current.is_none() && !disabled {
            self.current = Some(ix);
            self.focus_current_item(window, cx);
        }
        cx.notify();
        Ok(())
    }

    pub fn set_choice_disabled(
        &mut self,
        item: &str,
        value: &str,
        disabled: bool,
        cx: &mut Context<Self>,
    ) -> Result<(), QuestionnaireSchemaError> {
        let item_ix = self.item_ix(item)?;
        let choice_ix = self.choice_ix(item_ix, value)?;
        if self.runtime[item_ix].choice_disabled[choice_ix] == disabled {
            return Ok(());
        }
        self.runtime[item_ix].choice_disabled[choice_ix] = disabled;
        self.answer_did_change(item_ix, false, cx);
        Ok(())
    }

    pub fn set_external_error(
        &mut self,
        item: &str,
        error: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) -> Result<(), QuestionnaireSchemaError> {
        let ix = self.item_ix(item)?;
        self.runtime[ix].external_error = Some(QuestionnaireValidationError::Message(error.into()));
        self.complete = false;
        cx.notify();
        Ok(())
    }

    pub fn clear_external_error(
        &mut self,
        item: &str,
        cx: &mut Context<Self>,
    ) -> Result<(), QuestionnaireSchemaError> {
        let ix = self.item_ix(item)?;
        self.runtime[ix].external_error = None;
        cx.notify();
        Ok(())
    }

    pub fn reset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for ix in 0..self.items.len() {
            let initial = self.runtime[ix].initial_answer.clone();
            self.runtime[ix].answer = initial;
            self.runtime[ix].skipped = false;
            self.runtime[ix].validation_attempted = false;
            self.runtime[ix].internal_error = None;
            if let Some(input) = self.items[ix].input() {
                let value = self.runtime[ix]
                    .initial_input_value
                    .as_ref()
                    .cloned()
                    .unwrap_or_default();
                input
                    .state()
                    .update(cx, |input, cx| input.set_value(value, window, cx));
            }
        }
        self.complete = false;
        self.pending_confirm = None;
        self.current = self
            .initial_current
            .as_ref()
            .and_then(|name| self.item_ix_opt(name));
        if self.current.is_some_and(|ix| self.runtime[ix].disabled) {
            let next = self.enabled_indices().next();
            self.current = next;
        }
        self.focus_current_item(window, cx);
        cx.notify();
    }

    /// Selects a choice, or toggles it on a multiple-answer item, without
    /// confirming the item. Arrow-key movement inside a radio group uses this;
    /// a person's click, Space, Enter or shortcut goes through [`Self::choose`].
    pub fn activate_choice(
        &mut self,
        item: &str,
        value: &str,
        cx: &mut Context<Self>,
    ) -> Result<(), QuestionnaireSchemaError> {
        let item_ix = self.item_ix(item)?;
        let choice_ix = self.choice_ix(item_ix, value)?;
        if self.runtime[item_ix].disabled || self.runtime[item_ix].choice_disabled[choice_ix] {
            return Ok(());
        }
        let before = self.effective_answer(item_ix);
        let before_status = self.status(item_ix);

        if self.items[item_ix].is_multiple() {
            let selected = self.runtime[item_ix]
                .answer
                .choices
                .iter()
                .position(|choice| choice.as_ref() == value);
            if let Some(ix) = selected {
                self.runtime[item_ix].answer.choices.remove(ix);
            } else {
                self.runtime[item_ix].answer.choices.push(value.into());
            }
        } else {
            self.runtime[item_ix].answer.choices.clear();
            self.runtime[item_ix].answer.choices.push(value.into());
            self.runtime[item_ix].answer.freeform = None;
        }
        self.runtime[item_ix].skipped = false;
        if before != self.effective_answer(item_ix) || before_status != self.status(item_ix) {
            self.answer_did_change(item_ix, true, cx);
        }
        Ok(())
    }

    /// Applies a person's activation of a choice: a click, Space or Enter on the
    /// focused choice, or its shortcut.
    ///
    /// A multiple-answer item only toggles the choice. A single-answer item
    /// selects it and then confirms the current item as [`Self::confirm_current`]
    /// does: it moves to the next enabled item, or submits on the last one,
    /// unless validation fails. A choice that changes the answer stays selected
    /// on screen for a moment before the confirm runs, so the person sees what
    /// they picked; with reduced motion the confirm runs as soon as the answer
    /// change has been delivered. Choosing the selected answer again confirms
    /// at once. Disabled items and choices ignore activation.
    ///
    /// Arrow-key movement inside a radio group uses [`Self::activate_choice`]
    /// and never confirms.
    pub fn choose(
        &mut self,
        item: &str,
        value: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), QuestionnaireSchemaError> {
        let item_ix = self.item_ix(item)?;
        let choice_ix = self.choice_ix(item_ix, value)?;
        if self.runtime[item_ix].disabled || self.runtime[item_ix].choice_disabled[choice_ix] {
            return Ok(());
        }
        let before = self.effective_answer(item_ix);
        self.activate_choice(item, value, cx)?;
        self.focus_choice(item, value, window, cx);
        if self.items[item_ix].is_multiple() || self.current != Some(item_ix) {
            return Ok(());
        }
        if before == self.effective_answer(item_ix) {
            self.confirm_current(window, cx);
        } else {
            self.schedule_confirm(item_ix, window, cx);
        }
        Ok(())
    }

    /// Confirms `item_ix` after the selection has had time to show. The task
    /// runs on the next turn of the event loop at the earliest, after
    /// `AnswerChanged` subscribers have reacted (a host enabling a follow-up
    /// item, say), and does nothing if another item became current meanwhile.
    fn schedule_confirm(&mut self, item_ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let delay = (!cx.reduce_motion()).then_some(CHOICE_CONFIRM_DELAY);
        self.pending_confirm = Some(cx.spawn_in(window, async move |this, cx| {
            if let Some(delay) = delay {
                cx.background_executor().timer(delay).await;
            }
            let _ = this.update_in(cx, |this, window, cx| {
                this.pending_confirm = None;
                if this.current == Some(item_ix) {
                    this.confirm_current(window, cx);
                }
            });
        }));
    }

    pub fn confirm_current(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(current_ix) = self.current_ix() else {
            return false;
        };
        if current_ix + 1 == self.total() {
            self.submit(window, cx)
        } else {
            self.go_next(window, cx)
        }
    }

    pub fn go_previous(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.pending_confirm = None;
        let Some(ix) = self.current_ix() else {
            return false;
        };
        let enabled: Vec<_> = self.enabled_indices().collect();
        if ix == 0 {
            return false;
        }
        self.change_current(Some(enabled[ix - 1]), true, window, cx);
        true
    }

    pub fn go_next(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.pending_confirm = None;
        let Some(current) = self.current else {
            return false;
        };
        if !self.validate_item(current) {
            self.focus_invalid_item(self.items[current].name(), window, cx);
            cx.notify();
            return false;
        }
        let Some(ix) = self.current_ix() else {
            return false;
        };
        let enabled: Vec<_> = self.enabled_indices().collect();
        if ix + 1 >= enabled.len() {
            return false;
        }
        self.change_current(Some(enabled[ix + 1]), true, window, cx);
        true
    }

    pub fn skip_current(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(ix) = self.current else { return false };
        if self.items[ix].is_required() {
            return false;
        }
        self.runtime[ix].answer = QuestionnaireAnswer::new();
        self.runtime[ix].skipped = true;
        self.complete = false;
        self.emit_answer_changed(ix, cx);
        if self
            .current_ix()
            .is_some_and(|current| current + 1 == self.total())
        {
            self.submit(window, cx)
        } else {
            self.go_next(window, cx)
        }
    }

    pub fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.pending_confirm = None;
        let enabled: Vec<_> = self.enabled_indices().collect();
        let mut first_invalid = None;
        for ix in enabled {
            if !self.validate_item(ix) && first_invalid.is_none() {
                first_invalid = Some(ix);
            }
        }
        if let Some(ix) = first_invalid {
            self.change_current(Some(ix), true, window, cx);
            self.focus_invalid_item(self.items[ix].name(), window, cx);
            cx.notify();
            return false;
        }

        let submission = self.submission();
        if !self.complete {
            self.complete = true;
            cx.emit(QuestionnaireEvent::Completed(submission.clone()));
        }
        cx.emit(QuestionnaireEvent::Submit(submission));
        cx.notify();
        true
    }

    pub fn focus_current_item(&self, window: &mut Window, cx: &mut App) -> bool {
        let Some(ix) = self.current else { return false };
        self.runtime[ix].focus_handle.focus(window, cx);
        true
    }

    pub fn focus_invalid_item(&self, item: &str, window: &mut Window, cx: &mut App) -> bool {
        let Some(item_ix) = self.item_ix_opt(item) else {
            return false;
        };
        if self.runtime[item_ix].answer.freeform().is_some()
            && !self.runtime[item_ix].input_disabled
            && let Some(focus_handle) = &self.runtime[item_ix].input_focus_handle
        {
            focus_handle.focus(window, cx);
            return true;
        }
        for (choice_ix, choice) in self.items[item_ix].choices().iter().enumerate() {
            if self.runtime[item_ix]
                .answer
                .choices
                .contains(choice.value())
                && !self.runtime[item_ix].choice_disabled[choice_ix]
            {
                self.runtime[item_ix].choice_focus_handles[choice_ix].focus(window, cx);
                return true;
            }
        }
        if let Some(choice_ix) = self.runtime[item_ix]
            .choice_disabled
            .iter()
            .position(|disabled| !disabled)
        {
            self.runtime[item_ix].choice_focus_handles[choice_ix].focus(window, cx);
            return true;
        }
        if !self.runtime[item_ix].input_disabled
            && let Some(focus_handle) = &self.runtime[item_ix].input_focus_handle
        {
            focus_handle.focus(window, cx);
            return true;
        }
        self.runtime[item_ix].focus_handle.focus(window, cx);
        true
    }

    pub fn focus_choice(&self, item: &str, value: &str, window: &mut Window, cx: &mut App) -> bool {
        let Some(item_ix) = self.item_ix_opt(item) else {
            return false;
        };
        let Some(choice_ix) = self.choice_ix_opt(item_ix, value) else {
            return false;
        };
        self.runtime[item_ix].choice_focus_handles[choice_ix].focus(window, cx);
        true
    }

    pub fn focus_input(&self, item: &str, window: &mut Window, cx: &mut App) -> bool {
        let Some(item_ix) = self.item_ix_opt(item) else {
            return false;
        };
        let Some(focus_handle) = &self.runtime[item_ix].input_focus_handle else {
            return false;
        };
        focus_handle.focus(window, cx);
        true
    }

    pub fn focus_previous_answer(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.focus_adjacent_answer(-1, window, cx)
    }

    pub fn focus_next_answer(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.focus_adjacent_answer(1, window, cx)
    }

    pub fn move_current_radio(
        &mut self,
        direction: isize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(item_ix) = self.current else {
            return false;
        };
        if self.items[item_ix].is_multiple() || self.is_current_input_focused(window) {
            return false;
        }
        let enabled: Vec<_> = self.runtime[item_ix]
            .choice_disabled
            .iter()
            .enumerate()
            .filter_map(|(ix, disabled)| (!disabled).then_some(ix))
            .collect();
        if enabled.is_empty() {
            return false;
        }
        let current = enabled
            .iter()
            .position(|ix| self.runtime[item_ix].choice_focus_handles[*ix].is_focused(window))
            .or_else(|| {
                enabled.iter().position(|ix| {
                    self.runtime[item_ix]
                        .answer
                        .choices
                        .contains(self.items[item_ix].choices()[*ix].value())
                })
            });
        let target = match (current, direction.is_negative()) {
            (Some(ix), true) => ix.checked_sub(1).unwrap_or(enabled.len() - 1),
            (Some(ix), false) => (ix + 1) % enabled.len(),
            (None, true) => enabled.len() - 1,
            (None, false) => 0,
        };
        let choice_ix = enabled[target];
        let item = self.items[item_ix].name().clone();
        let choice = self.items[item_ix].choices()[choice_ix].value().clone();
        let _ = self.activate_choice(&item, &choice, cx);
        self.runtime[item_ix].choice_focus_handles[choice_ix].focus(window, cx);
        true
    }

    fn focus_adjacent_answer(
        &mut self,
        direction: isize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(item_ix) = self.current else {
            return false;
        };
        if self.is_current_input_focused(window) && self.current_input_has_text(cx) {
            return false;
        }

        #[derive(Clone, Copy)]
        enum Target {
            Choice(usize),
            Input,
        }
        let mut targets = Vec::new();
        for choice_ix in 0..self.items[item_ix].choices().len() {
            if !self.runtime[item_ix].choice_disabled[choice_ix] {
                targets.push(Target::Choice(choice_ix));
            }
        }
        if self.items[item_ix].input().is_some() && !self.runtime[item_ix].input_disabled {
            targets.push(Target::Input);
        }
        if targets.is_empty() {
            return false;
        }

        let focused = targets.iter().position(|target| match target {
            Target::Choice(ix) => {
                self.runtime[item_ix].choice_focus_handles[*ix].is_focused(window)
            }
            Target::Input => self.is_current_input_focused(window),
        });

        if focused.is_none() && self.runtime[item_ix].focus_handle.is_focused(window) {
            let filled: Vec<_> = targets
                .iter()
                .enumerate()
                .filter_map(|(ix, target)| match target {
                    Target::Choice(choice_ix)
                        if self.runtime[item_ix]
                            .answer
                            .choices
                            .contains(self.items[item_ix].choices()[*choice_ix].value()) =>
                    {
                        Some(ix)
                    }
                    Target::Input if self.runtime[item_ix].answer.freeform().is_some() => Some(ix),
                    _ => None,
                })
                .collect();
            let filled_ix = if direction.is_negative() {
                filled.last().copied()
            } else {
                filled.first().copied()
            };
            if let Some(filled_ix) = filled_ix {
                match targets[filled_ix] {
                    Target::Choice(choice_ix) => {
                        self.runtime[item_ix].choice_focus_handles[choice_ix].focus(window, cx);
                    }
                    Target::Input => {
                        if let Some(focus_handle) = &self.runtime[item_ix].input_focus_handle {
                            focus_handle.focus(window, cx);
                        }
                    }
                }
                return true;
            }
        }

        let target_ix = match (focused, direction.is_negative()) {
            (Some(ix), true) => ix.checked_sub(1).unwrap_or(targets.len() - 1),
            (Some(ix), false) => (ix + 1) % targets.len(),
            (None, true) => targets.len() - 1,
            (None, false) => 0,
        };

        // GPUI's Radio primitive intentionally leaves group arrow behavior to
        // its owner. Let the root emulate native radio movement only when both
        // adjacent answers are radios; crossing the boundary to an Input stays
        // in this schema-ordered answer sequence.
        if !self.items[item_ix].is_multiple()
            && focused.is_some_and(|focused_ix| {
                matches!(targets[focused_ix], Target::Choice(_))
                    && matches!(targets[target_ix], Target::Choice(_))
            })
        {
            return false;
        }

        match targets[target_ix] {
            Target::Choice(choice_ix) => {
                if !self.items[item_ix].is_multiple() {
                    let item = self.items[item_ix].name().clone();
                    let choice = self.items[item_ix].choices()[choice_ix].value().clone();
                    let _ = self.activate_choice(&item, &choice, cx);
                }
                self.runtime[item_ix].choice_focus_handles[choice_ix].focus(window, cx);
            }
            Target::Input => {
                if let Some(focus_handle) = &self.runtime[item_ix].input_focus_handle {
                    focus_handle.focus(window, cx);
                }
            }
        }
        true
    }

    fn on_input_change(&mut self, input: &Entity<InputState>, cx: &mut Context<Self>) {
        if let Some(ix) = self.input_item_ix(input) {
            self.sync_input_answer(ix, true, cx);
        }
    }

    fn sync_input_answer(&mut self, item_ix: usize, emit: bool, cx: &mut Context<Self>) {
        if self.runtime[item_ix].disabled || self.runtime[item_ix].input_disabled {
            return;
        }
        let before = self.effective_answer(item_ix);
        let before_status = self.status(item_ix);
        let Some(input) = self.items[item_ix].input() else {
            return;
        };
        let value = input.state().read(cx).value();
        if value.trim().is_empty() {
            self.runtime[item_ix].answer.freeform = None;
        } else {
            if !self.items[item_ix].is_multiple() {
                self.runtime[item_ix].answer.choices.clear();
            }
            self.runtime[item_ix].answer.freeform = Some(value);
            self.runtime[item_ix].skipped = false;
        }
        if before != self.effective_answer(item_ix) || before_status != self.status(item_ix) {
            self.answer_did_change(item_ix, emit, cx);
        }
    }

    fn answer_did_change(&mut self, item_ix: usize, emit: bool, cx: &mut Context<Self>) {
        self.pending_confirm = None;
        if self.runtime[item_ix].validation_attempted {
            self.validate_item(item_ix);
        } else {
            self.runtime[item_ix].internal_error = None;
        }
        self.complete = false;
        if emit {
            self.emit_answer_changed(item_ix, cx);
        }
        cx.notify();
    }

    fn emit_answer_changed(&self, item_ix: usize, cx: &mut Context<Self>) {
        cx.emit(QuestionnaireEvent::AnswerChanged(
            QuestionnaireAnswerChange::new(
                self.items[item_ix].name().clone(),
                self.effective_answer(item_ix),
                self.status(item_ix),
            ),
        ));
    }

    fn validate_item(&mut self, item_ix: usize) -> bool {
        if self.runtime[item_ix].disabled || self.runtime[item_ix].skipped {
            return true;
        }
        self.runtime[item_ix].validation_attempted = true;
        let answer = self.effective_answer(item_ix);
        let error = if answer.is_empty() {
            Some(if self.items[item_ix].is_required() {
                QuestionnaireValidationError::Required
            } else {
                QuestionnaireValidationError::Unanswered
            })
        } else if let Some(validator) = self.items[item_ix].validator().cloned() {
            validator(&QuestionnaireValidationContext::new(
                self.items[item_ix].name().clone(),
                answer,
                self.answers(),
            ))
            .err()
            .map(QuestionnaireValidationError::Message)
        } else {
            None
        };
        self.runtime[item_ix].internal_error = error;
        self.error_at(item_ix).is_none()
    }

    fn submission(&self) -> QuestionnaireSubmission {
        QuestionnaireSubmission::new(
            self.enabled_indices()
                .map(|ix| {
                    QuestionnaireSubmissionItem::new(
                        self.items[ix].name().clone(),
                        self.status(ix),
                        self.effective_answer(ix),
                    )
                })
                .collect(),
        )
    }

    fn status(&self, item_ix: usize) -> QuestionnaireItemStatus {
        if self.runtime[item_ix].skipped {
            QuestionnaireItemStatus::Skipped
        } else if self.effective_answer(item_ix).is_empty() {
            QuestionnaireItemStatus::Unanswered
        } else {
            QuestionnaireItemStatus::Answered
        }
    }

    fn effective_answer(&self, item_ix: usize) -> QuestionnaireAnswer {
        if self.runtime[item_ix].disabled {
            return QuestionnaireAnswer::new();
        }
        let runtime = &self.runtime[item_ix];
        QuestionnaireAnswer {
            choices: self.items[item_ix]
                .choices()
                .iter()
                .filter(|choice| {
                    runtime.answer.choices.contains(choice.value())
                        && self
                            .choice_ix_opt(item_ix, choice.value())
                            .is_some_and(|ix| !runtime.choice_disabled[ix])
                })
                .map(|choice| choice.value().clone())
                .collect(),
            freeform: (!runtime.input_disabled)
                .then(|| runtime.answer.freeform.clone())
                .flatten(),
        }
    }

    fn error_at(&self, item_ix: usize) -> Option<&QuestionnaireValidationError> {
        if self.runtime[item_ix].skipped || self.runtime[item_ix].disabled {
            return None;
        }
        self.runtime[item_ix].external_error.as_ref().or_else(|| {
            self.runtime[item_ix]
                .validation_attempted
                .then_some(self.runtime[item_ix].internal_error.as_ref())
                .flatten()
        })
    }

    fn check_answer(
        &self,
        item_ix: usize,
        answer: &QuestionnaireAnswer,
    ) -> Result<(), QuestionnaireSchemaError> {
        let item = &self.items[item_ix];
        let sources = answer.choices.len() + usize::from(answer.freeform.is_some());
        if (!item.is_multiple() && sources > 1)
            || (answer.freeform.is_some() && item.input().is_none())
        {
            return Err(QuestionnaireSchemaError::AnswerDoesNotMatchItem(
                item.name().clone(),
            ));
        }
        for choice in &answer.choices {
            let choice_ix = self.choice_ix(item_ix, choice)?;
            if self.runtime[item_ix].choice_disabled[choice_ix] {
                return Err(QuestionnaireSchemaError::AnswerDoesNotMatchItem(
                    item.name().clone(),
                ));
            }
        }
        Ok(())
    }

    fn change_current(
        &mut self,
        next: Option<usize>,
        emit: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.current == next {
            return;
        }
        self.pending_confirm = None;
        let previous = self.current.map(|ix| self.items[ix].name().clone());
        self.current = next;
        self.focus_current_item(window, cx);
        if emit {
            cx.emit(QuestionnaireEvent::CurrentItemChanged {
                previous,
                current: next.map(|ix| self.items[ix].name().clone()),
            });
        }
        cx.notify();
    }

    fn enabled_indices(&self) -> impl DoubleEndedIterator<Item = usize> + '_ {
        self.runtime
            .iter()
            .enumerate()
            .filter_map(|(ix, runtime)| (!runtime.disabled).then_some(ix))
    }

    fn item_ix(&self, name: &str) -> Result<usize, QuestionnaireSchemaError> {
        self.item_ix_opt(name)
            .ok_or_else(|| QuestionnaireSchemaError::UnknownItem(name.into()))
    }

    fn item_ix_opt(&self, name: &str) -> Option<usize> {
        self.items
            .iter()
            .position(|item| item.name().as_ref() == name)
    }

    fn choice_ix(&self, item_ix: usize, value: &str) -> Result<usize, QuestionnaireSchemaError> {
        self.choice_ix_opt(item_ix, value)
            .ok_or_else(|| QuestionnaireSchemaError::UnknownChoice {
                item: self.items[item_ix].name().clone(),
                choice: value.into(),
            })
    }

    fn choice_ix_opt(&self, item_ix: usize, value: &str) -> Option<usize> {
        self.items[item_ix]
            .choices()
            .iter()
            .position(|choice| choice.value().as_ref() == value)
    }

    fn input_item_ix(&self, input: &Entity<InputState>) -> Option<usize> {
        self.items.iter().position(|item| {
            item.input()
                .is_some_and(|definition| definition.state().entity_id() == input.entity_id())
        })
    }
}

impl EventEmitter<QuestionnaireEvent> for QuestionnaireState {}
