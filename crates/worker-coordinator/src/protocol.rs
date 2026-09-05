use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const PROTOCOL_VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LanguageMode {
    English,
    Filipino,
    Taglish,
}

impl LanguageMode {
    pub fn as_protocol_language(self) -> &'static str {
        match self {
            Self::English => "english",
            Self::Filipino => "filipino",
            Self::Taglish => "taglish",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioWindow {
    pub session_id: String,
    pub sequence: u64,
    pub start_micros: u64,
    pub end_micros: u64,
    pub sample_rate: u32,
    pub channels: u16,
    pub pcm_f32_le: Vec<u8>,
    pub language: LanguageMode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerCapabilities {
    pub protocol_version: u16,
    pub whisperx_version: String,
    pub models: Vec<String>,
    pub languages: Vec<LanguageMode>,
    pub diarization: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranscriptWord {
    pub text: String,
    pub start_micros: u64,
    pub end_micros: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranscriptResult {
    pub session_id: String,
    pub sequence: u64,
    pub start_micros: u64,
    pub end_micros: u64,
    pub text: String,
    pub words: Vec<TranscriptWord>,
    pub language: LanguageMode,
    pub provisional: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranscriptSegment {
    pub start_micros: u64,
    pub end_micros: u64,
    pub text: String,
    pub words: Vec<TranscriptWord>,
    pub speaker: Option<String>,
    pub alignment_status: AlignmentStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlignmentStatus {
    Word,
    Segment,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkerRequest {
    Hello {
        protocol_version: u16,
    },
    TranscribeWindow(AudioWindow),
    TranscribeRecording {
        session_id: String,
        audio_path: String,
        language: LanguageMode,
        #[serde(default)]
        protocol_only: bool,
    },
    Finalize {
        session_id: String,
    },
    Capabilities,
    Shutdown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkerResponse {
    Ready {
        protocol_version: u16,
    },
    Capabilities(WorkerCapabilities),
    Transcript(TranscriptResult),
    FinalTranscript {
        session_id: String,
        language: LanguageMode,
        text: String,
        segments: Vec<TranscriptSegment>,
    },
    Finalized {
        session_id: String,
    },
    Error {
        code: String,
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FinalTranscript {
    pub session_id: String,
    pub language: LanguageMode,
    pub text: String,
    pub segments: Vec<TranscriptSegment>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ProtocolError {
    #[error("invalid JSON message: {0}")]
    InvalidJson(String),
    #[error("unsupported protocol version {0}")]
    UnsupportedVersion(u16),
}

pub fn encode_request(request: &WorkerRequest) -> Result<String, ProtocolError> {
    serde_json::to_string(request).map_err(|error| ProtocolError::InvalidJson(error.to_string()))
}

pub fn decode_request(message: &str) -> Result<WorkerRequest, ProtocolError> {
    let request: WorkerRequest = serde_json::from_str(message)
        .map_err(|error| ProtocolError::InvalidJson(error.to_string()))?;
    if let WorkerRequest::Hello { protocol_version } = request {
        if protocol_version != PROTOCOL_VERSION {
            return Err(ProtocolError::UnsupportedVersion(protocol_version));
        }
        return Ok(WorkerRequest::Hello { protocol_version });
    }
    Ok(request)
}

pub fn encode_response(response: &WorkerResponse) -> Result<String, ProtocolError> {
    serde_json::to_string(response).map_err(|error| ProtocolError::InvalidJson(error.to_string()))
}

pub fn decode_response(message: &str) -> Result<WorkerResponse, ProtocolError> {
    serde_json::from_str(message).map_err(|error| ProtocolError::InvalidJson(error.to_string()))
}
