use super::{CommandError, Result, Terminals};
use crate::terminal::MAX_REPLY_BYTES;
use std::{collections::VecDeque, sync::Mutex};
use tokio::sync::Notify;

pub const MAX_INPUT_BYTES: usize = 16 * 1024;
pub const MAX_INPUT_COMMANDS: usize = 32;

// Leave headroom for one entire callback batch before entering the model.
const QUEUED_REPLY_BYTES: usize = 4 * MAX_REPLY_BYTES;

enum Write {
    User(Vec<u8>),
    Paste { text: Vec<u8>, bracketed: bool },
    Replies(Vec<u8>),
}

impl Write {
    fn into_bytes(self) -> Vec<u8> {
        match self {
            Self::User(bytes) | Self::Replies(bytes) => bytes,
            Self::Paste { text, bracketed } => {
                crate::terminal::input::paste_bytes(&text, bracketed)
            }
        }
    }
}

#[derive(Default, PartialEq, Eq)]
enum WriteState {
    #[default]
    Open,
    InputClosed,
    Finished,
    Failed,
}

#[derive(Default)]
struct Queue {
    fifo: VecDeque<Write>,
    users: usize,
    reply_bytes: usize,
    state: WriteState,
    unwritten: usize,
}

#[derive(Default)]
pub(super) struct Writes {
    queue: Mutex<Queue>,
    available: Notify,
    pub progress: Notify,
}

impl Writes {
    pub fn check_open(&self) -> std::result::Result<(), CommandError> {
        if self.queue.lock().unwrap().state == WriteState::Open {
            Ok(())
        } else {
            Err(CommandError::Closed)
        }
    }

    pub fn input(&self, bytes: &[u8]) -> std::result::Result<(), CommandError> {
        self.admit(bytes.len(), || Write::User(bytes.to_vec()))
    }

    pub fn paste(&self, text: Vec<u8>, bracketed: bool) -> std::result::Result<(), CommandError> {
        let size = text.len().saturating_add(if bracketed { 12 } else { 0 });
        self.admit(size, || Write::Paste { text, bracketed })
    }

    fn admit(
        &self,
        size: usize,
        build: impl FnOnce() -> Write,
    ) -> std::result::Result<(), CommandError> {
        let mut queue = self.queue.lock().unwrap();
        if queue.state != WriteState::Open {
            return Err(CommandError::Closed);
        }
        if size > MAX_INPUT_BYTES {
            return Err(CommandError::TooLarge);
        }
        if queue.users == MAX_INPUT_COMMANDS {
            return Err(CommandError::Busy);
        }
        if size != 0 {
            queue.fifo.push_back(build());
            queue.users += 1;
            self.available.notify_one();
        }
        Ok(())
    }

    pub fn reply_space(&self) -> bool {
        self.queue.lock().unwrap().reply_bytes <= QUEUED_REPLY_BYTES - MAX_REPLY_BYTES
    }

    // The actor is the only reply producer. Its reserved space is independent
    // of user admissions so a full paste queue never blocks ordinary PTY reads.
    pub fn replies(&self, replies: Vec<Vec<u8>>) {
        if replies.is_empty() {
            return;
        }
        let mut queue = self.queue.lock().unwrap();
        if queue.state == WriteState::Failed {
            for bytes in replies {
                queue.unwritten = queue.unwritten.saturating_add(bytes.len());
            }
            return;
        }
        let bytes = replies.concat();
        debug_assert!(queue.reply_bytes + bytes.len() <= QUEUED_REPLY_BYTES);
        queue.reply_bytes += bytes.len();
        // Coalesce adjacent byte writes only; no bytes are discarded or reordered.
        if let Some(Write::Replies(tail)) = queue.fifo.back_mut()
            && tail.len() + bytes.len() <= MAX_REPLY_BYTES
        {
            tail.extend(bytes);
        } else {
            queue.fifo.push_back(Write::Replies(bytes));
        }
        self.available.notify_one();
    }

    pub fn seal_input(&self) {
        let mut queue = self.queue.lock().unwrap();
        if queue.state == WriteState::Open {
            queue.state = WriteState::InputClosed;
        }
    }

    pub fn failed(&self) -> bool {
        self.queue.lock().unwrap().state == WriteState::Failed
    }

    pub fn unwritten(&self) -> usize {
        self.queue.lock().unwrap().unwritten
    }

    fn write_failed(&self) {
        let mut queue = self.queue.lock().unwrap();
        queue.state = WriteState::Failed;
        let queued = std::mem::take(&mut queue.fifo);
        queue.users = 0;
        queue.reply_bytes = 0;
        drop(queue);
        let unwritten = queued
            .into_iter()
            .map(|write| write.into_bytes().len())
            .sum::<usize>();
        let mut queue = self.queue.lock().unwrap();
        queue.unwritten = queue.unwritten.saturating_add(unwritten);
        self.progress.notify_one();
    }

    pub fn seal(&self, cancel: bool) {
        let mut queue = self.queue.lock().unwrap();
        if queue.state != WriteState::Failed {
            queue.state = WriteState::Finished;
        }
        if cancel {
            queue.fifo.clear();
            queue.users = 0;
            queue.reply_bytes = 0;
        }
        self.available.notify_one();
    }

    pub async fn write_all(&self, core: &Terminals, id: &str) -> Result<()> {
        loop {
            let write = {
                let mut queue = self.queue.lock().unwrap();
                match queue.fifo.pop_front() {
                    Some(write @ (Write::User(_) | Write::Paste { .. })) => {
                        queue.users -= 1;
                        Some(write)
                    }
                    Some(Write::Replies(bytes)) => {
                        queue.reply_bytes -= bytes.len();
                        self.progress.notify_one();
                        Some(Write::Replies(bytes))
                    }
                    None if matches!(queue.state, WriteState::Finished | WriteState::Failed) => {
                        return Ok(());
                    }
                    None => None,
                }
            };
            if let Some(write) = write {
                if let Err(error) = core.write(id, write.into_bytes()).await {
                    self.write_failed();
                    return Err(format!("Terminal write: {error}"));
                }
            } else {
                self.available.notified().await;
            }
        }
    }
}
