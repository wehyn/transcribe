use crate::{
    AudioWindow, PROTOCOL_VERSION, TranscriptResult, TranscriptWord, WorkerRequest, WorkerResponse,
};
use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoordinatorError {
    #[error("worker process could not start: {0}")]
    Spawn(#[source] io::Error),
    #[error("worker stdin is unavailable")]
    StdinUnavailable,
    #[error("worker stdout is unavailable")]
    StdoutUnavailable,
    #[error("worker process has stopped")]
    WorkerStopped,
    #[error("worker write failed: {0}")]
    Write(#[source] io::Error),
    #[error("worker read failed: {0}")]
    Read(#[source] io::Error),
    #[error("worker returned an unexpected response")]
    UnexpectedResponse,
    #[error("worker returned an error: {code}: {message}")]
    Worker { code: String, message: String },
    #[error("rolling window failed: {0}")]
    Window(String),
}

pub trait WorkerTransport {
    fn send(&mut self, request: &WorkerRequest) -> Result<WorkerResponse, CoordinatorError>;
}

impl<T: WorkerTransport + ?Sized> WorkerTransport for Box<T> {
    fn send(&mut self, request: &WorkerRequest) -> Result<WorkerResponse, CoordinatorError> {
        (**self).send(request)
    }
}

pub struct WorkerCoordinator<T> {
    transport: T,
    in_flight: HashMap<u64, AudioWindow>,
}

pub struct LivePipeline<T> {
    coordinator: WorkerCoordinator<T>,
    windows: crate::RollingWindowBuffer,
    reconciler: crate::TranscriptReconciler,
}

impl<T: WorkerTransport> LivePipeline<T> {
    pub fn new(
        transport: T,
        config: crate::WindowConfig,
        session_id: impl Into<String>,
        sample_rate: u32,
        channels: u16,
        language: crate::LanguageMode,
    ) -> Self {
        Self {
            coordinator: WorkerCoordinator::new(transport),
            windows: crate::RollingWindowBuffer::new(
                config,
                session_id,
                sample_rate,
                channels,
                language,
            ),
            reconciler: crate::TranscriptReconciler::new(),
        }
    }

    pub fn push_pcm(
        &mut self,
        timestamp_micros: u64,
        pcm_f32_le: &[u8],
    ) -> Result<Option<crate::LiveTranscript>, CoordinatorError> {
        self.windows
            .push_pcm(timestamp_micros, pcm_f32_le)
            .map_err(|error| CoordinatorError::Window(format!("{error:?}")))?;
        while let Some(window) = self.windows.pop_window() {
            let result = self.coordinator.send_window(window)?;
            self.reconciler.accept(result);
        }
        Ok(self.reconciler.snapshot())
    }

    pub fn snapshot(&self) -> Option<crate::LiveTranscript> {
        self.reconciler.snapshot()
    }

    pub fn finalize(
        &mut self,
        session_id: impl Into<String>,
        audio_path: impl Into<String>,
        language: crate::LanguageMode,
    ) -> Result<crate::FinalTranscript, CoordinatorError> {
        self.coordinator
            .finish_session(session_id.into(), audio_path.into(), language)
    }
}

impl<T: WorkerTransport> WorkerCoordinator<T> {
    pub fn new(transport: T) -> Self {
        Self {
            transport,
            in_flight: HashMap::new(),
        }
    }

    pub fn send_window(
        &mut self,
        window: AudioWindow,
    ) -> Result<TranscriptResult, CoordinatorError> {
        let sequence = window.sequence;
        self.in_flight.insert(sequence, window.clone());
        let response = self
            .transport
            .send(&WorkerRequest::TranscribeWindow(window))?;
        match response {
            WorkerResponse::Transcript(result) if result.sequence == sequence => {
                self.in_flight.remove(&sequence);
                Ok(result)
            }
            WorkerResponse::Error { code, message } => {
                self.in_flight.remove(&sequence);
                Err(CoordinatorError::Worker { code, message })
            }
            _ => Err(CoordinatorError::UnexpectedResponse),
        }
    }

    pub fn send_recording(
        &mut self,
        session_id: String,
        audio_path: String,
        language: crate::LanguageMode,
    ) -> Result<crate::FinalTranscript, CoordinatorError> {
        let response = self.transport.send(&WorkerRequest::TranscribeRecording {
            session_id,
            audio_path,
            language,
            protocol_only: false,
        })?;
        match response {
            WorkerResponse::FinalTranscript {
                session_id,
                language,
                text,
                segments,
            } => Ok(crate::FinalTranscript {
                session_id,
                language,
                text,
                segments,
            }),
            WorkerResponse::Error { code, message } => {
                Err(CoordinatorError::Worker { code, message })
            }
            _ => Err(CoordinatorError::UnexpectedResponse),
        }
    }

    pub fn capabilities(&mut self) -> Result<crate::WorkerCapabilities, CoordinatorError> {
        match self.transport.send(&WorkerRequest::Capabilities)? {
            WorkerResponse::Capabilities(capabilities) => Ok(capabilities),
            WorkerResponse::Error { code, message } => {
                Err(CoordinatorError::Worker { code, message })
            }
            _ => Err(CoordinatorError::UnexpectedResponse),
        }
    }

    pub fn transcribe_recording(
        &mut self,
        session_id: String,
        audio_path: String,
        language: crate::LanguageMode,
    ) -> Result<crate::FinalTranscript, CoordinatorError> {
        self.send_recording(session_id, audio_path, language)
    }

    pub fn finish_session(
        &mut self,
        session_id: String,
        audio_path: String,
        language: crate::LanguageMode,
    ) -> Result<crate::FinalTranscript, CoordinatorError> {
        let transcript = self.transcribe_recording(session_id.clone(), audio_path, language)?;
        self.finalize(session_id)?;
        Ok(transcript)
    }

    pub fn send_window_best_effort(
        &mut self,
        window: AudioWindow,
    ) -> Result<Option<TranscriptResult>, CoordinatorError> {
        match self.send_window(window) {
            Ok(result) => Ok(Some(result)),
            Err(CoordinatorError::Worker { .. }) => Ok(None),
            Err(error) => Err(error),
        }
    }

    pub fn finalize(&mut self, session_id: String) -> Result<(), CoordinatorError> {
        match self
            .transport
            .send(&WorkerRequest::Finalize { session_id })?
        {
            WorkerResponse::Finalized { .. } => Ok(()),
            WorkerResponse::Error { code, message } => {
                Err(CoordinatorError::Worker { code, message })
            }
            _ => Err(CoordinatorError::UnexpectedResponse),
        }
    }

    pub fn in_flight_len(&self) -> usize {
        self.in_flight.len()
    }

    pub fn into_transport(self) -> T {
        self.transport
    }
}

pub struct JsonLinesWorker {
    child: Child,
    stdin: ChildStdin,
    stdout: io::BufReader<ChildStdout>,
}

impl JsonLinesWorker {
    pub fn from_parts(child: Child, stdin: ChildStdin, stdout: ChildStdout) -> Self {
        Self {
            child,
            stdin,
            stdout: io::BufReader::new(stdout),
        }
    }

    pub fn spawn(program: impl AsRef<std::ffi::OsStr>) -> Result<Self, CoordinatorError> {
        let mut child = Command::new(program)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(CoordinatorError::Spawn)?;
        let stdin = child
            .stdin
            .take()
            .ok_or(CoordinatorError::StdinUnavailable)?;
        let stdout = child
            .stdout
            .take()
            .ok_or(CoordinatorError::StdoutUnavailable)?;
        Ok(Self {
            child,
            stdin,
            stdout: io::BufReader::new(stdout),
        })
    }

    pub fn handshake(&mut self) -> Result<(), CoordinatorError> {
        match self.send(&WorkerRequest::Hello {
            protocol_version: PROTOCOL_VERSION,
        })? {
            WorkerResponse::Ready { protocol_version } if protocol_version == PROTOCOL_VERSION => {
                Ok(())
            }
            _ => Err(CoordinatorError::UnexpectedResponse),
        }
    }

    pub fn send_shutdown(&mut self) -> Result<(), CoordinatorError> {
        self.stdin
            .write_all(b"{\"type\":\"shutdown\"}\n")
            .map_err(CoordinatorError::Write)?;
        self.stdin.flush().map_err(CoordinatorError::Write)
    }
}

impl WorkerTransport for JsonLinesWorker {
    fn send(&mut self, request: &WorkerRequest) -> Result<WorkerResponse, CoordinatorError> {
        let line = serde_json::to_string(request).map_err(|error| {
            CoordinatorError::Write(io::Error::new(io::ErrorKind::InvalidData, error))
        })?;
        writeln!(self.stdin, "{line}").map_err(CoordinatorError::Write)?;
        self.stdin.flush().map_err(CoordinatorError::Write)?;

        let mut response = String::new();
        self.stdout
            .read_line(&mut response)
            .map_err(CoordinatorError::Read)?;
        if response.is_empty() {
            return Err(CoordinatorError::Read(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "worker closed stdout",
            )));
        }
        serde_json::from_str(response.trim()).map_err(|error| {
            CoordinatorError::Read(io::Error::new(io::ErrorKind::InvalidData, error))
        })
    }
}

impl Drop for JsonLinesWorker {
    fn drop(&mut self) {
        let _ = self.stdin.flush();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub fn provisional_result(window: &AudioWindow, text: impl Into<String>) -> TranscriptResult {
    TranscriptResult {
        session_id: window.session_id.clone(),
        sequence: window.sequence,
        start_micros: window.start_micros,
        end_micros: window.end_micros,
        text: text.into(),
        words: Vec::<TranscriptWord>::new(),
        language: window.language,
        provisional: true,
    }
}
