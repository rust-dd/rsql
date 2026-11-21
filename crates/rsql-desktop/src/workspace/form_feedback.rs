use super::{Operation, SharedString, toast::Toast};
use rsql_desktop::{connection::ConnectionProfile, profile_service::Challenge};
use std::{sync::Arc, time::Duration};

pub(super) enum Feedback {
    Success(Message),
    Error(Message),
}

impl Feedback {
    pub(super) fn error(text: impl Into<String>) -> Self {
        Self::Error(Message::new(text.into()))
    }

    pub(super) fn message(&self) -> &Message {
        match self {
            Self::Success(message) | Self::Error(message) => message,
        }
    }

    pub(super) fn connection_failed(&self, profile: &ConnectionProfile) -> Toast {
        Toast::error(format!("Connection failed: {}", profile.display_name()))
            .description(&self.message().preview)
            .lifetime(Duration::from_secs(10))
    }
}

pub(super) struct Message {
    pub(super) full: Arc<String>,
    pub(super) preview: SharedString,
}

impl Message {
    fn new(full: String) -> Self {
        let preview = preview(&full);
        Self {
            full: Arc::new(full),
            preview,
        }
    }

    pub(super) fn can_copy(&self) -> bool {
        self.full.len() > MAX_INLINE_BYTES
    }
}

const MAX_INLINE_BYTES: usize = 2048;

pub(super) fn preview(text: &str) -> SharedString {
    if text.len() <= MAX_INLINE_BYTES {
        return SharedString::from(text);
    }
    let end = text.floor_char_boundary(MAX_INLINE_BYTES);
    format!("{}…", &text[..end]).into()
}

pub(super) enum Outcome {
    Connected,
    Trust(Challenge),
    Feedback(Feedback),
}

impl Outcome {
    pub(super) fn prepare(
        result: Result<String, String>,
        target: &ConnectionProfile,
        op: Operation,
        retry: bool,
    ) -> Self {
        match result {
            Ok(_) if op == Operation::Connect => Self::Connected,
            Ok(version) => Self::Feedback(Feedback::Success(Message::new(version))),
            Err(error) => match Challenge::parse(&error) {
                Some(_) if retry => Self::error("SSH key verification failed after approval."),
                Some(challenge) => Self::Trust(challenge),
                None => Self::error(target.safe_error(error)),
            },
        }
    }

    pub(super) fn error(text: impl Into<String>) -> Self {
        Self::Feedback(Feedback::error(text))
    }
}
