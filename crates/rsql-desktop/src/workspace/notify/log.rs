use chrono::{DateTime, Local};
use gpui_kit::SharedString;
use rsql_core::services::notifications::Notification;
use std::collections::VecDeque;

const ENTRY_LIMIT: usize = 10_000;
const TEXT_LIMIT: usize = 16 * 1024 * 1024;

pub(super) struct Received {
    pub time: DateTime<Local>,
    pub event: Notification,
}

pub(super) struct Row {
    pub id: u64,
    pub time: DateTime<Local>,
    pub channel: SharedString,
    pub payload: SharedString,
}

#[derive(Default)]
pub(super) struct Log {
    pub rows: VecDeque<Row>,
    pub dropped: u64,
    bytes: usize,
    sequence: u64,
}

impl Log {
    pub fn push(&mut self, received: Received) -> Option<usize> {
        let Notification { channel, payload } = received.event;
        let size = channel.len() + payload.len();
        if size > TEXT_LIMIT {
            self.dropped += 1;
            return None;
        }
        let previous = self.rows.len();
        while self.rows.len() == ENTRY_LIMIT || self.bytes + size > TEXT_LIMIT {
            let old = self.rows.pop_front().unwrap();
            self.bytes -= old.channel.len() + old.payload.len();
            self.dropped += 1;
        }
        self.sequence += 1;
        self.bytes += size;
        self.rows.push_back(Row {
            time: received.time,
            id: self.sequence,
            channel: channel.into(),
            payload: payload.into(),
        });
        Some(previous + 1 - self.rows.len())
    }

    pub fn clear(&mut self) {
        self.rows.clear();
        self.bytes = 0;
        self.dropped = 0;
    }
}
