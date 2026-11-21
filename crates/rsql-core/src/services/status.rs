use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, Default, Debug, PartialEq, Serialize, Deserialize)]
pub enum ProjectConnectionStatus {
    Connected,
    Connecting,
    #[default]
    Disconnected,
    Failed,
}

impl std::error::Error for ProjectConnectionStatus {}
impl fmt::Display for ProjectConnectionStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProjectConnectionStatus::Connected => write!(f, "Connected"),
            ProjectConnectionStatus::Connecting => write!(f, "Connecting"),
            ProjectConnectionStatus::Disconnected => write!(f, "Disconnected"),
            ProjectConnectionStatus::Failed => write!(f, "Failed"),
        }
    }
}
