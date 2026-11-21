use super::{CommandError, ViewCommand, writes::Writes};
use crate::terminal::{Scroll, TermMode, TerminalModel};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

pub(crate) enum Edit<'a> {
    Preedit,
    AlternateScroll { up: bool },
    Text(&'a [u8]),
    Paste { text: Vec<u8>, bracketed: bool },
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum EditAdmission {
    Primary,
    Alternate,
    ObsoleteScroll,
}

#[derive(Default)]
struct Flag {
    actual: bool,
    pending: usize,
    projected: bool,
}

impl Flag {
    fn needed(&self) -> bool {
        if self.pending == 0 {
            self.actual
        } else {
            self.projected
        }
    }

    fn admit(&mut self, value: Option<bool>) {
        if let Some(value) = value {
            self.pending += 1;
            self.projected = value;
        }
    }
}

#[derive(Clone, Copy, Default)]
pub(super) struct Effects {
    scroll: Option<bool>,
    selection: Option<bool>,
}

impl ViewCommand {
    pub(super) fn effects(&self) -> Effects {
        let mut effect = Effects::default();
        match self {
            Self::Scroll(scroll) => effect.scroll = Some(!matches!(scroll, Scroll::Bottom)),
            Self::ScrollSelection(..) | Self::ScrollDisplayed(..) => {
                effect.scroll = Some(true);
                effect.selection = Some(true);
            }
            Self::StartSelection(kind, ..) | Self::SelectDisplayed(Some(kind), ..) => {
                effect.selection = Some(matches!(
                    kind,
                    crate::terminal::SelectionType::Semantic
                        | crate::terminal::SelectionType::Lines
                ));
            }
            Self::UpdateSelection(..) | Self::SelectDisplayed(None, ..) | Self::SelectAll => {
                effect.selection = Some(true)
            }
            Self::ClearSelection => effect.selection = Some(false),
            Self::ApplyEdit { reveal, clear } => {
                effect.scroll = reveal.then_some(false);
                effect.selection = clear.then_some(false);
            }
            _ => {}
        }
        effect
    }
}

#[derive(Default)]
struct State {
    mode: TermMode,
    mouse_generation: u64,
    scroll: Flag,
    selection: Flag,
}

impl State {
    fn observe(&mut self, model: &TerminalModel) {
        self.mode = *model.term.mode();
        self.mouse_generation = model.mouse_generation;
        if !self.mode.contains(TermMode::ALT_SCREEN) {
            self.scroll.actual = model.term.grid().display_offset() != 0;
        }
        self.selection.actual = model.term.selection.as_ref().is_some_and(|s| !s.is_empty());
    }

    fn admit(&mut self, effects: Effects) {
        self.scroll.admit(effects.scroll);
        self.selection.admit(effects.selection);
    }
}

#[derive(Clone, Default)]
pub(super) struct ViewState(Arc<Mutex<State>>);

impl ViewState {
    pub fn mode(&self) -> TermMode {
        self.0.lock().unwrap().mode
    }

    pub fn report_input(
        &self,
        bytes: &[u8],
        enabled: impl FnOnce(TermMode, u64) -> bool,
        writes: &Writes,
    ) -> Result<Option<u64>, CommandError> {
        let state = self.0.lock().unwrap();
        if !enabled(state.mode, state.mouse_generation) {
            return Ok(None);
        }
        writes.input(bytes)?;
        Ok(Some(state.mouse_generation))
    }

    pub fn observe(&self, model: &TerminalModel) {
        self.0.lock().unwrap().observe(model);
    }

    pub fn complete(&self, effects: Effects, model: &TerminalModel) {
        let mut s = self.0.lock().unwrap();
        s.scroll.pending -= usize::from(effects.scroll.is_some());
        s.selection.pending -= usize::from(effects.selection.is_some());
        s.observe(model);
    }
}

pub(super) struct ViewQueue {
    sender: mpsc::Sender<ViewCommand>,
    pub state: ViewState,
}

impl ViewQueue {
    pub fn new(sender: mpsc::Sender<ViewCommand>) -> Self {
        Self {
            sender,
            state: ViewState::default(),
        }
    }

    pub fn send(&self, command: ViewCommand) -> Result<(), CommandError> {
        let mut s = self.state.0.lock().unwrap();
        let effects = command.effects();
        self.sender.try_send(command).map_err(admission_error)?;
        s.admit(effects);
        Ok(())
    }

    pub fn edit(&self, edit: Edit<'_>, writes: &Writes) -> Result<EditAdmission, CommandError> {
        let mut s = self.state.0.lock().unwrap();
        let primary = !s.mode.contains(TermMode::ALT_SCREEN);
        if matches!(edit, Edit::AlternateScroll { .. })
            && (primary || s.mode.intersects(TermMode::MOUSE_MODE))
        {
            return Ok(EditAdmission::ObsoleteScroll);
        }
        let reveal = primary && s.scroll.needed();
        let committed = match &edit {
            Edit::Preedit => false,
            Edit::AlternateScroll { .. } => true,
            Edit::Text(bytes) => !bytes.is_empty(),
            Edit::Paste { text, .. } => !text.is_empty(),
        };
        let clear = committed && s.selection.needed();
        let permit = if reveal || clear {
            Some(self.sender.try_reserve().map_err(admission_error)?)
        } else {
            None
        };
        match edit {
            Edit::Preedit => writes.check_open()?,
            Edit::AlternateScroll { up } => {
                let prefix = if s.mode.contains(TermMode::APP_CURSOR) {
                    b'O'
                } else {
                    b'['
                };
                writes.input(&[0x1b, prefix, if up { b'A' } else { b'B' }])?;
            }
            Edit::Text(bytes) => writes.input(bytes)?,
            Edit::Paste { text, bracketed } => writes.paste(text, bracketed)?,
        }
        if let Some(permit) = permit {
            let command = ViewCommand::ApplyEdit { reveal, clear };
            s.admit(command.effects());
            permit.send(command);
        }
        Ok(if primary {
            EditAdmission::Primary
        } else {
            EditAdmission::Alternate
        })
    }
}

fn admission_error<T>(error: mpsc::error::TrySendError<T>) -> CommandError {
    match error {
        mpsc::error::TrySendError::Full(_) => CommandError::Busy,
        mpsc::error::TrySendError::Closed(_) => CommandError::Closed,
    }
}
