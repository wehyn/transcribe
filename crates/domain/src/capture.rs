use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LanguageMode {
    English,
    Filipino,
    Taglish,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackRole {
    Microphone,
    System,
    Mixed,
    RawBundle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureConfig {
    pub microphone: bool,
    pub system_audio: bool,
    pub language: LanguageMode,
}

impl CaptureConfig {
    pub fn dual_source(language: LanguageMode) -> Self {
        Self {
            microphone: true,
            system_audio: true,
            language,
        }
    }
}
