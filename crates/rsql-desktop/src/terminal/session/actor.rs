use super::{
    Completion, OutputAfterFailure, Result, SessionOwner, SessionPhase, SessionUpdate, Spawned,
    Terminals, ViewCommand, Viewport, merge, scheduling::Turn, writes::Writes,
};
use crate::terminal::{FeedStatus, TerminalModel, TerminalSnapshot};

use std::{
    future::pending,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, watch};

pub(super) struct Actor {
    pub core: Arc<Terminals>,
    pub theme: crate::terminal::TerminalTheme,
    pub admission: super::view_queue::ViewState,
    pub id: String,
    pub stopping: watch::Receiver<bool>,
    pub dimensions: watch::Receiver<Viewport>,
    pub commands: mpsc::Receiver<ViewCommand>,
    pub publish: watch::Sender<Arc<SessionUpdate>>,
    pub writes: Arc<Writes>,
    pub owner: SessionOwner,
}

impl Actor {
    pub async fn run(mut self, opened: Result<Spawned>, initial: Viewport) -> Result<()> {
        let mut stream = match opened {
            Ok(output) => output,
            Err(error) => {
                let mut error = format!("Terminal spawn: {error}");
                if let Err(cleanup) = self.close_core().await {
                    error.push_str("; ");
                    error.push_str(&cleanup);
                }
                self.finish(SessionPhase::Failed(error.clone()), None, Vec::new());
                return Err(error);
            }
        };
        if *self.stopping.borrow() {
            drop(stream.output);
            let result = self.close_owned(stream.completion).await;
            self.finish(
                result
                    .clone()
                    .err()
                    .map_or(SessionPhase::Closed, SessionPhase::Failed),
                None,
                Vec::new(),
            );
            return result;
        }

        let mut model = TerminalModel::new(initial.columns.into(), initial.rows.into());
        model.set_theme(self.theme);
        let core = self.core.clone();
        let id = self.id.clone();
        let writes = self.writes.clone();
        let writer = writes.write_all(&core, &id);
        let mut result = Ok(());
        let process = async {
            let phase = self.process(&mut model, &mut stream, &mut result).await;
            drop(stream.output);
            self.writes.seal(!matches!(phase, Ok(SessionPhase::Exited)));
            // Closing the slave can unblock a write. Join both I/O paths.
            (phase, self.close_owned(stream.completion).await)
        };
        let ((phase, closed), written) = tokio::join!(process, writer);
        let healthy_eof = matches!(phase, Ok(SessionPhase::Exited));
        merge(
            &mut result,
            phase.as_ref().map(|_| ()).map_err(Clone::clone),
        );
        merge(&mut result, closed);
        merge(&mut result, written);
        let unwritten = self.writes.unwritten();
        if unwritten > 0 {
            merge(
                &mut result,
                Err(format!(
                    "{unwritten} queued terminal bytes could not be written"
                )),
            );
        }
        let final_phase = match &result {
            Ok(()) => phase?,
            Err(error) => SessionPhase::Failed(error.clone()),
        };
        if healthy_eof {
            self.review_exited(&mut model, &final_phase).await;
        }
        self.finish(
            if final_phase == SessionPhase::Exited {
                SessionPhase::Closed
            } else {
                final_phase
            },
            Some(Arc::new(model.snapshot())),
            model.take_replies(),
        );
        result
    }

    async fn close_owned(&self, completion: Completion) -> Result<()> {
        let closed = self.close_core().await;
        let joined = completion
            .await
            .map_err(|e| format!("Terminal cleanup: {e}"));
        closed.and(joined)
    }

    async fn close_core(&self) -> Result<()> {
        self.core
            .close(&self.id)
            .await
            .map_err(|e| format!("Terminal close: {e}"))
    }

    pub fn finish(
        &self,
        phase: SessionPhase,
        screen: Option<Arc<TerminalSnapshot>>,
        fault_replies: Vec<Vec<u8>>,
    ) {
        self.writes.seal(true);
        self.publish.send_replace(Arc::new(SessionUpdate {
            owner: self.owner.clone(),
            phase,
            screen,
            displayed: None,
            fault_replies,
        }));
    }

    fn repaint(&self, model: &TerminalModel, phase: SessionPhase) {
        let screen = Arc::new(model.snapshot());
        // Serialize live publication with replacement/close while the stop guard is held.
        let stopped = self.stopping.borrow();
        if !*stopped {
            self.publish.send_replace(Arc::new(SessionUpdate {
                owner: self.owner.clone(),
                phase,
                screen: Some(screen),
                displayed: Some(model.displayed_rows()),
                fault_replies: Vec::new(),
            }));
        }
    }

    async fn process(
        &mut self,
        model: &mut TerminalModel,
        stream: &mut Spawned,
        control: &mut Result<()>,
    ) -> Result<SessionPhase> {
        let mut turn = Turn::default();
        let mut bytes = Vec::new();
        let mut offset = 0;
        let output = &mut stream.output;
        let mut completion = stream.completion.clone();
        let mut draining = false;
        let mut eof = false;
        let mut repaint = None;
        self.admission.observe(model);
        self.repaint(model, SessionPhase::Running);
        loop {
            if *self.stopping.borrow() {
                return Ok(SessionPhase::Closed);
            }
            if !draining && (control.is_err() || self.writes.failed()) {
                self.writes.seal_input();
                let decision = tokio::select! {
                    biased;
                    _ = self.stopping.changed() => return Ok(SessionPhase::Closed),
                    result = &mut stream.after_failure => result,
                };
                match decision {
                    Ok(OutputAfterFailure::CloseReceiver) => output.close(),
                    Ok(OutputAfterFailure::Drain) => {}
                    Err(error) => merge(control, Err(format!("Terminal child status: {error}"))),
                }
                draining = true;
                continue;
            }
            let room = self.writes.reply_space();
            let deadline = model.synchronized_update_deadline();
            if room && deadline.is_some_and(|d| eof || d <= Instant::now()) {
                let status = model.flush_synchronized_update();
                self.admission.observe(model);
                if let FeedStatus::Fault(fault) = status {
                    tokio::task::yield_now().await;
                    return Err(format!("Terminal protocol: {fault:?}"));
                }
                self.writes.replies(model.take_replies());
                tokio::task::yield_now().await;
                if *self.stopping.borrow() {
                    return Ok(SessionPhase::Closed);
                }
                repaint.get_or_insert_with(next_frame);
                turn = Turn::default();
                continue;
            }
            if eof && deadline.is_none() {
                return Ok(SessionPhase::Exited);
            }
            tokio::select! {
                _ = self.stopping.changed() => return Ok(SessionPhase::Closed),
                _ = self.writes.progress.notified() => {}
                _ = &mut completion, if draining && !output.is_closed() => output.close(),
                _ = wait_until(deadline), if room => {}
                _ = wait_until(repaint) => {
                    self.repaint(model, SessionPhase::Running);
                    repaint = None;
                }
                size = self.dimensions.changed(), if !draining && !eof => {
                    if size.is_err() { return Ok(SessionPhase::Closed); }
                    let size = *self.dimensions.borrow_and_update();
                    match self.core.resize(&self.id, size.columns, size.rows).await {
                        Ok(()) => {
                            model.resize(size.columns.into(), size.rows.into());
                            self.admission.observe(model);
                            repaint.get_or_insert_with(next_frame);
                        }
                        Err(error) => {
                            self.writes.seal_input();
                            *control = Err(format!("Terminal resize: {error}"));
                        }
                    }
                }
                command = self.commands.recv() => {
                    let Some(command) = command else { return Ok(SessionPhase::Closed); };
                    if apply(model, command, &self.admission) {
                        repaint.get_or_insert_with(next_frame);
                    }
                }
                chunk = output.recv(), if room && !eof && offset == bytes.len() => {
                    match chunk {
                        Some(chunk) => {
                            bytes = chunk;
                            offset = 0;
                        }
                        None => {
                            self.writes.seal_input();
                            eof = true;
                        }
                    }
                }
                () = std::future::ready(()), if room && offset < bytes.len() => {
                    let (outcome, must_yield) = turn.feed(model, &bytes[offset..]);
                    self.admission.observe(model);
                    if let FeedStatus::Fault(fault) = outcome.status {
                        if must_yield { tokio::task::yield_now().await; }
                        return Err(format!("Terminal protocol: {fault:?}"));
                    }
                    offset += outcome.accepted;
                    self.writes.replies(model.take_replies());
                    if must_yield { tokio::task::yield_now().await; }
                    repaint.get_or_insert_with(next_frame);
                }
            }
        }
    }
    async fn review_exited(&mut self, model: &mut TerminalModel, phase: &SessionPhase) {
        for _ in 0..self.commands.len() {
            let Ok(command) = self.commands.try_recv() else {
                break;
            };
            apply(model, command, &self.admission);
        }
        let mut repaint = None;
        self.repaint(model, phase.clone());
        while !*self.stopping.borrow() {
            tokio::select! {
                _ = self.stopping.changed() => return,
                _ = wait_until(repaint) => {
                    self.repaint(model, phase.clone());
                    repaint = None;
                }
                changed = self.dimensions.changed() => {
                    if changed.is_err() { return; }
                    let size = *self.dimensions.borrow_and_update();
                    model.resize(size.columns.into(), size.rows.into());
                    self.admission.observe(model);
                    repaint.get_or_insert_with(next_frame);
                }
                command = self.commands.recv() => {
                    let Some(command) = command else { return; };
                    if apply(model, command, &self.admission) {
                        repaint.get_or_insert_with(next_frame);
                    }
                }
            }
        }
    }
}

fn next_frame() -> Instant {
    Instant::now() + Duration::from_millis(16)
}

async fn wait_until(at: Option<Instant>) {
    match at {
        Some(at) => tokio::time::sleep_until(at.into()).await,
        None => pending().await,
    }
}

fn apply(
    model: &mut TerminalModel,
    command: ViewCommand,
    state: &super::view_queue::ViewState,
) -> bool {
    let effects = command.effects();
    match command {
        ViewCommand::ScrollDisplayed(delta, row, target, side) => {
            model.scroll_displayed(delta, row, target, side);
        }
        ViewCommand::ScrollSelection(delta, row, col, side) => {
            if !model
                .term
                .mode()
                .contains(crate::terminal::TermMode::ALT_SCREEN)
            {
                model.scroll(crate::terminal::Scroll::Delta(delta));
                model.update_selection(row, col, side);
            }
        }
        ViewCommand::ApplyEdit { reveal, clear } => {
            if clear {
                model.clear_selection();
            }
            if reveal
                && !model
                    .term
                    .mode()
                    .contains(crate::terminal::TermMode::ALT_SCREEN)
            {
                model.scroll(crate::terminal::Scroll::Bottom);
            }
        }
        ViewCommand::SelectAll => model.select_all(),
        ViewCommand::SelectDisplayed(kind, point, side) => {
            model.select_displayed(kind, point, side)
        }
        ViewCommand::Scroll(scroll) => model.scroll(scroll),
        ViewCommand::StartSelection(kind, row, column, side) => {
            model.start_selection(kind, row, column, side)
        }
        ViewCommand::UpdateSelection(row, column, side) => {
            model.update_selection(row, column, side)
        }
        ViewCommand::CopySelection(request) => {
            if !request.0.is_closed() {
                let text = model
                    .term
                    .selection_to_string_limited(super::MAX_COPY_BYTES)
                    .map_err(|_| super::CommandError::TooLarge);
                let _ = request.0.send(text);
            }
            return false;
        }
        ViewCommand::ClearSelection => model.clear_selection(),
        ViewCommand::Theme(theme) => model.set_theme(theme),
        ViewCommand::CellSize(width, height) => model.set_cell_size(width, height),
    }
    state.complete(effects, model);
    true
}
