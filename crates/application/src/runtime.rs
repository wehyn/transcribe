use meeting_capture::{CaptureCapabilities, CaptureController, CaptureSource};
use meeting_domain::{CaptureConfig, LanguageMode, SessionState};
use meeting_storage::{AudioFormat, RecordingBundle, RecordingError, TrackRole};
use std::path::{Path, PathBuf};
use thiserror::Error;
use whisperx_worker::{
    CoordinatorError, FinalTranscript, LanguageMode as WorkerLanguageMode, LivePipeline,
    LiveTranscript, WindowConfig, WorkerTransport,
};

fn worker_language(value: LanguageMode) -> WorkerLanguageMode {
    match value {
        LanguageMode::English => WorkerLanguageMode::English,
        LanguageMode::Filipino => WorkerLanguageMode::Filipino,
        LanguageMode::Taglish => WorkerLanguageMode::Taglish,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerStartContext {
    pub session_id: String,
    pub language: WorkerLanguageMode,
}

pub trait WorkerFactory {
    fn start(
        &mut self,
        context: WorkerStartContext,
    ) -> Result<Box<dyn WorkerTransport>, CoordinatorError>;
}

pub struct DefaultWorkerFactory {
    worker_root: PathBuf,
    model_path: Option<PathBuf>,
}

impl Default for DefaultWorkerFactory {
    fn default() -> Self {
        Self {
            worker_root: PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../worker-coordinator/python"),
            model_path: None,
        }
    }
}

impl DefaultWorkerFactory {
    pub fn for_resource_root(root: impl AsRef<Path>) -> Self {
        Self {
            worker_root: root.as_ref().to_path_buf(),
            model_path: None,
        }
    }

    pub fn with_model_path(mut self, path: impl AsRef<Path>) -> Self {
        self.model_path = Some(path.as_ref().to_path_buf());
        self
    }

    pub fn model_path(&self) -> Option<&Path> {
        self.model_path.as_deref()
    }
}

impl WorkerFactory for DefaultWorkerFactory {
    fn start(
        &mut self,
        _context: WorkerStartContext,
    ) -> Result<Box<dyn WorkerTransport>, CoordinatorError> {
        let config = if self.worker_root.join("bin/python3").is_file() {
            whisperx_worker::WorkerConfig::from_worker_root(&self.worker_root).map_err(|error| {
                CoordinatorError::Spawn(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    error.to_string(),
                ))
            })?
        } else {
            whisperx_worker::WorkerConfig::for_development(&self.worker_root).map_err(|error| {
                CoordinatorError::Spawn(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    error.to_string(),
                ))
            })?
        };
        let config = match &self.model_path {
            Some(path) => config.with_model_path(path),
            None => config,
        };
        let process = whisperx_worker::WorkerProcess::start(&config)?;
        Ok(Box::new(process.into_worker()?))
    }
}

#[derive(Debug, Error)]
pub enum ApplicationError {
    #[error("capture error: {0}")]
    Capture(#[from] meeting_capture::CaptureError),
    #[error("recording error: {0}")]
    Recording(#[from] RecordingError),
    #[error("worker error: {0}")]
    Worker(#[from] CoordinatorError),
    #[error("session has not started listening")]
    NotListening,
    #[error("recording path is not configured")]
    RecordingPathMissing,
    #[error("PCM data must contain complete 32-bit float samples")]
    InvalidPcm,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinalizationResult {
    pub recording_path: PathBuf,
    pub transcript: FinalTranscript,
}

pub struct MeetingRuntime {
    capture: CaptureController,
    recording: Option<RecordingBundle>,
    recording_path: Option<PathBuf>,
    session_id: String,
    language: WorkerLanguageMode,
    worker_factory: Box<dyn WorkerFactory>,
    live_pipeline: Option<LivePipeline<Box<dyn WorkerTransport>>>,
    final_transcript: Option<FinalTranscript>,
    window_config: WindowConfig,
    audio_format: AudioFormat,
}

impl MeetingRuntime {
    pub fn new(config: CaptureConfig, source: Box<dyn CaptureSource>) -> Self {
        Self::with_worker_factory_and_pipeline(
            config,
            source,
            DefaultWorkerFactory::default(),
            WindowConfig::new(4, 1, 4),
            AudioFormat::default(),
        )
    }

    pub fn with_worker_factory(
        config: CaptureConfig,
        source: Box<dyn CaptureSource>,
        worker_factory: impl WorkerFactory + 'static,
    ) -> Self {
        Self::with_worker_factory_and_pipeline(
            config,
            source,
            worker_factory,
            WindowConfig::new(4, 1, 4),
            AudioFormat::default(),
        )
    }

    pub fn with_worker_factory_and_pipeline(
        config: CaptureConfig,
        source: Box<dyn CaptureSource>,
        worker_factory: impl WorkerFactory + 'static,
        window_config: WindowConfig,
        audio_format: AudioFormat,
    ) -> Self {
        Self {
            capture: CaptureController::new(config, source),
            recording: None,
            recording_path: None,
            session_id: format!("session-{}", uuid::Uuid::new_v4()),
            language: worker_language(config.language),
            worker_factory: Box::new(worker_factory),
            live_pipeline: None,
            final_transcript: None,
            window_config,
            audio_format,
        }
    }

    pub fn with_fake_source(config: CaptureConfig, capabilities: CaptureCapabilities) -> Self {
        Self::new(
            config,
            Box::new(meeting_capture::FakeCaptureSource::new(capabilities)),
        )
    }

    pub fn state(&self) -> SessionState {
        self.capture.state()
    }

    pub fn source_is_open(&self) -> bool {
        self.capture.source_is_open()
    }

    pub fn accept_consent(&mut self) {
        self.capture.accept_consent();
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn final_notes(&self) -> Option<meeting_domain::FinalNotes> {
        let transcript = self.final_transcript.as_ref()?;
        let ranges = transcript
            .segments
            .iter()
            .filter_map(|segment| {
                meeting_domain::TranscriptSegmentRange::new(
                    segment.start_micros,
                    segment.end_micros,
                )
                .ok()
            })
            .collect::<Vec<_>>();
        meeting_domain::final_from_transcript(&transcript.text, &ranges).ok()
    }

    pub fn record(&mut self, root: impl AsRef<Path>) -> Result<(), ApplicationError> {
        let root = root.as_ref().to_path_buf();
        let transport = match self.worker_factory.start(WorkerStartContext {
            session_id: self.session_id.clone(),
            language: self.language,
        }) {
            Ok(transport) => transport,
            Err(error) => {
                return Err(error.into());
            }
        };
        let recording = match RecordingBundle::create_with_format(&root, self.audio_format) {
            Ok(recording) => recording,
            Err(error) => {
                drop(transport);
                return Err(error.into());
            }
        };
        if let Err(error) = self.capture.start() {
            drop(transport);
            drop(recording);
            return Err(error.into());
        }
        self.recording_path = Some(root);
        self.recording = Some(recording);
        self.live_pipeline = Some(LivePipeline::new(
            transport,
            self.window_config,
            self.session_id.clone(),
            self.audio_format.sample_rate,
            self.audio_format.channels,
            self.language,
        ));
        Ok(())
    }

    pub fn pause(&mut self) -> Result<(), ApplicationError> {
        self.capture.pause()?;
        if let Some(recording) = self.recording.as_mut() {
            recording.flush()?;
        }
        Ok(())
    }

    pub fn resume(&mut self) -> Result<(), ApplicationError> {
        self.capture.resume()?;
        Ok(())
    }

    pub fn append_track(
        &mut self,
        role: TrackRole,
        pcm_f32_le: &[u8],
    ) -> Result<(), ApplicationError> {
        self.append_track_with_format(role, self.audio_format, pcm_f32_le)
            .map(|_| ())
    }

    pub fn record_frame(
        &mut self,
        role: TrackRole,
        format: AudioFormat,
        pcm_f32_le: &[u8],
    ) -> Result<(), ApplicationError> {
        self.append_track_with_format(role, format, pcm_f32_le)
            .map(|_| ())
    }

    pub fn append_mixed_frame(
        &mut self,
        timestamp_micros: u64,
        format: AudioFormat,
        pcm_f32_le: &[u8],
    ) -> Result<Option<LiveTranscript>, ApplicationError> {
        if self.state() != SessionState::Listening {
            return Err(ApplicationError::NotListening);
        }
        let recording = self
            .recording
            .as_mut()
            .ok_or(ApplicationError::RecordingPathMissing)?;
        let samples = pcm_samples(pcm_f32_le)?;
        recording.append_frame(TrackRole::Mixed, format, &samples)?;
        let pipeline = self
            .live_pipeline
            .as_mut()
            .ok_or(ApplicationError::RecordingPathMissing)?;
        pipeline
            .push_pcm(timestamp_micros, pcm_f32_le)
            .map_err(ApplicationError::Worker)
    }

    pub fn live_transcript_snapshot(&self) -> Option<LiveTranscript> {
        self.live_pipeline
            .as_ref()
            .and_then(|pipeline| pipeline.snapshot())
    }

    pub fn final_transcript(&self) -> Option<&FinalTranscript> {
        self.final_transcript.as_ref()
    }

    pub fn stop_and_finalize(&mut self) -> Result<FinalizationResult, ApplicationError> {
        self.capture.stop()?;
        let recording = self
            .recording
            .take()
            .ok_or(ApplicationError::RecordingPathMissing)?;
        let sealed = recording.seal()?;
        let recording_path = sealed.root().to_path_buf();
        let mut pipeline = self
            .live_pipeline
            .take()
            .ok_or(ApplicationError::RecordingPathMissing)?;
        let audio_path = sealed.materialize_mixed_wav()?;
        let transcript = pipeline.finalize(
            self.session_id.clone(),
            audio_path.to_string_lossy().into_owned(),
            self.language,
        )?;
        self.final_transcript = Some(transcript.clone());
        Ok(FinalizationResult {
            recording_path,
            transcript,
        })
    }

    pub fn stop(&mut self) -> Result<PathBuf, ApplicationError> {
        self.capture.stop()?;
        let recording = self
            .recording
            .take()
            .ok_or(ApplicationError::RecordingPathMissing)?;
        let sealed = recording.seal()?;
        if let Some(mut pipeline) = self.live_pipeline.take() {
            let audio_path = sealed.materialize_mixed_wav()?;
            if let Ok(transcript) = pipeline.finalize(
                self.session_id.clone(),
                audio_path.to_string_lossy().into_owned(),
                self.language,
            ) {
                self.final_transcript = Some(transcript);
            }
        }
        Ok(sealed.root().to_path_buf())
    }

    pub fn recording_path(&self) -> Option<&Path> {
        self.recording_path.as_deref()
    }

    pub fn configuration(&self) -> CaptureConfig {
        self.capture.configuration()
    }

    fn append_track_with_format(
        &mut self,
        role: TrackRole,
        format: AudioFormat,
        pcm_f32_le: &[u8],
    ) -> Result<Option<LiveTranscript>, ApplicationError> {
        if self.state() != SessionState::Listening {
            return Err(ApplicationError::NotListening);
        }
        let recording = self
            .recording
            .as_mut()
            .ok_or(ApplicationError::RecordingPathMissing)?;
        let samples = pcm_samples(pcm_f32_le)?;
        recording.append_frame(role, format, &samples)?;
        Ok(None)
    }
}

fn pcm_samples(pcm_f32_le: &[u8]) -> Result<Vec<f32>, ApplicationError> {
    let (chunks, remainder) = pcm_f32_le.as_chunks::<4>();
    if !remainder.is_empty() {
        return Err(ApplicationError::InvalidPcm);
    }
    Ok(chunks
        .iter()
        .map(|bytes| f32::from_le_bytes(*bytes))
        .collect())
}
