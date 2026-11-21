use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard};

use super::{InputLimit, Rgb, TerminalFault};
use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::term::color::{COUNT, Colors};

/// Maximum retained reply payload, including 64-byte reservations per callback.
pub const MAX_REPLY_BYTES: usize = 64 * 1024;
/// Maximum combined ready replies and pending formatter events.
pub const MAX_REPLY_MESSAGES: usize = 1024;
const CALLBACK_RESERVATION: usize = 64;
const BYTE_EVENT_RESERVATION: usize = 16;

#[derive(Clone, Default)]
pub(super) struct Events(Arc<Mutex<State>>);

#[derive(Default)]
pub(super) struct State {
    pending: VecDeque<Event>,
    pending_bytes: usize,
    replies: Vec<Vec<u8>>,
    reply_bytes: usize,
    pub title: Option<String>,
    pub fault: Option<TerminalFault>,
}

impl Events {
    pub fn lock(&self) -> MutexGuard<'_, State> {
        self.0.lock().expect("terminal event state poisoned")
    }
}

impl EventListener for Events {
    fn send_event(&self, event: Event) {
        let mut state = self.lock();
        let cost = match &event {
            Event::CombiningLimitExceeded(limit) => {
                state.fault.get_or_insert(TerminalFault::InputLimit(
                    InputLimit::CombiningScalars { limit: *limit },
                ));
                return;
            }
            Event::PtyWrite(text) => text.len(),
            Event::ColorRequest(..) | Event::TextAreaSizeRequest(..) => CALLBACK_RESERVATION,
            Event::Title(title) => {
                state.title = Some(title.clone());
                return;
            }
            Event::ResetTitle => {
                state.title = None;
                return;
            }
            _ => return,
        };
        if state.fault.is_some()
            || state.pending.len() + state.replies.len() == MAX_REPLY_MESSAGES
            || cost > MAX_REPLY_BYTES.saturating_sub(state.pending_bytes + state.reply_bytes)
        {
            state.reject_reply();
            return;
        }
        state.pending_bytes += cost;
        state.pending.push_back(event);
    }
}

impl State {
    pub fn has_headroom(&self) -> bool {
        self.pending.len() + self.replies.len() <= MAX_REPLY_MESSAGES - BYTE_EVENT_RESERVATION
            && self.pending_bytes + self.reply_bytes
                <= MAX_REPLY_BYTES - CALLBACK_RESERVATION * BYTE_EVENT_RESERVATION
    }

    pub fn resolve(&mut self, colors: &Colors, palette: &[Rgb; COUNT], size: WindowSize) {
        while let Some(event) = self.pending.pop_front() {
            let (reply, reservation) = match event {
                Event::PtyWrite(text) => {
                    let length = text.len();
                    (text.into_bytes(), length)
                }
                Event::ColorRequest(index, format) => {
                    let color = if index < COUNT {
                        colors[index].unwrap_or(palette[index])
                    } else {
                        Rgb::default()
                    };
                    (format(color).into_bytes(), CALLBACK_RESERVATION)
                }
                Event::TextAreaSizeRequest(format) => {
                    (format(size).into_bytes(), CALLBACK_RESERVATION)
                }
                _ => unreachable!("only reply events are queued"),
            };
            self.pending_bytes -= reservation;
            if reply.len() > MAX_REPLY_BYTES.saturating_sub(self.reply_bytes + self.pending_bytes) {
                self.reject_reply();
                for _ in 0..self.pending.len() {
                    self.reject_reply();
                }
                self.pending.clear();
                self.pending_bytes = 0;
                break;
            }
            self.reply_bytes += reply.len();
            self.replies.push(reply);
        }
    }

    pub fn take_replies(&mut self) -> Vec<Vec<u8>> {
        self.reply_bytes = 0;
        std::mem::take(&mut self.replies)
    }

    fn reject_reply(&mut self) {
        let rejected_replies = match self.fault {
            Some(TerminalFault::ReplyOverflow { rejected_replies }) => {
                rejected_replies.saturating_add(1)
            }
            Some(TerminalFault::InputLimit(_)) => return,
            None => 1,
        };
        self.fault = Some(TerminalFault::ReplyOverflow { rejected_replies });
    }
}
