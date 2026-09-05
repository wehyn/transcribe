use serde::{Deserialize, Serialize};
use std::time::SystemTime;
use uuid::Uuid;

use crate::{CaptureConfig, SessionState, TrackRole};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordingArtifact {
    pub id: Uuid,
    pub role: TrackRole,
    pub path: String,
    pub sample_rate: u32,
    pub channels: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveSession {
    pub id: Uuid,
    pub meeting_id: Uuid,
    pub title: String,
    pub config: CaptureConfig,
    pub state: SessionState,
    pub consent_accepted: bool,
    pub recordings: Vec<RecordingArtifact>,
    pub created_at: SystemTime,
    pub started_at: Option<SystemTime>,
    pub ended_at: Option<SystemTime>,
}

impl LiveSession {
    pub fn new(title: impl Into<String>, config: CaptureConfig) -> Self {
        Self {
            id: Uuid::new_v4(),
            meeting_id: Uuid::new_v4(),
            title: title.into(),
            config,
            state: SessionState::Created,
            consent_accepted: false,
            recordings: Vec::new(),
            created_at: SystemTime::now(),
            started_at: None,
            ended_at: None,
        }
    }
}
