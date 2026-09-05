use serde::{Deserialize, Serialize};
use std::time::SystemTime;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    Created,
    Starting,
    Listening,
    Paused,
    Stopping,
    Sealed,
    Stopped,
    Processing,
    Ready,
    Failed,
    Deleted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MeetingStatus {
    Draft,
    Live,
    Processing,
    Ready,
    Failed,
    Deleted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Meeting {
    pub id: Uuid,
    pub title: String,
    pub status: MeetingStatus,
    pub language: crate::LanguageMode,
    pub created_at: SystemTime,
    pub started_at: Option<SystemTime>,
    pub ended_at: Option<SystemTime>,
}

impl Meeting {
    pub fn new(title: impl Into<String>, language: crate::LanguageMode) -> Self {
        Self {
            id: Uuid::new_v4(),
            title: title.into(),
            status: MeetingStatus::Draft,
            language,
            created_at: SystemTime::now(),
            started_at: None,
            ended_at: None,
        }
    }
}

impl SessionState {
    pub fn can_transition_to(self, next: Self) -> bool {
        matches!(
            (self, next),
            (Self::Created, Self::Starting)
                | (Self::Starting, Self::Listening)
                | (Self::Starting, Self::Failed)
                | (Self::Listening, Self::Paused)
                | (Self::Listening, Self::Stopped)
                | (Self::Listening, Self::Stopping)
                | (Self::Listening, Self::Failed)
                | (Self::Paused, Self::Listening)
                | (Self::Paused, Self::Starting)
                | (Self::Paused, Self::Stopping)
                | (Self::Paused, Self::Stopped)
                | (Self::Paused, Self::Failed)
                | (Self::Stopping, Self::Sealed)
                | (Self::Sealed, Self::Processing)
                | (Self::Stopped, Self::Processing)
                | (Self::Stopped, Self::Ready)
                | (Self::Stopped, Self::Failed)
                | (Self::Processing, Self::Ready)
                | (Self::Processing, Self::Failed)
                | (Self::Created, Self::Deleted)
                | (Self::Stopped, Self::Deleted)
                | (Self::Ready, Self::Deleted)
                | (Self::Failed, Self::Deleted)
        )
    }
}
