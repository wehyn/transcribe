use meeting_domain::{CaptureConfig, LanguageMode, SessionState};
use std::sync::{Arc, Mutex};
use thiserror::Error;

#[cfg(all(feature = "macos-native", target_os = "macos"))]
mod macos;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureTrack {
    Microphone,
    System,
}

/// A source frame borrowed for the duration of an audio callback.
///
/// `timestamp_micros` is monotonic elapsed time from the successful `open` call.
/// A sink that needs to retain a frame must copy `samples` before returning.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AudioFrame<'a> {
    pub track: CaptureTrack,
    pub sample_rate: u32,
    pub channels: u16,
    pub timestamp_micros: u64,
    pub samples: &'a [f32],
}

/// Receives normalized PCM from an opened capture source.
///
/// Implementations must keep callback work bounded. The capture adapters invoke
/// these methods from native audio callbacks, so a sink should copy frames into
/// its own queue rather than perform blocking or durable I/O inline.
pub trait AudioSink: Send + Sync + 'static {
    fn on_audio_frame(&self, frame: AudioFrame<'_>);

    fn on_capture_error(&self, _track: CaptureTrack, _error: &str) {}
}

pub type AudioSinkHandle = Arc<dyn AudioSink>;

#[derive(Debug, Default)]
pub struct NullAudioSink;

impl AudioSink for NullAudioSink {
    fn on_audio_frame(&self, _frame: AudioFrame<'_>) {}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CaptureCapabilities {
    pub microphone_available: bool,
    pub system_audio_available: bool,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SourceError {
    #[error("microphone capture is unavailable")]
    MicrophoneUnavailable,
    #[error("system audio capture is unavailable")]
    SystemAudioUnavailable,
    #[error("capture source is already open")]
    AlreadyOpen,
    #[error("native capture is only available on macOS")]
    UnsupportedPlatform,
}

pub trait CaptureSource: Send {
    fn capabilities(&self) -> CaptureCapabilities;
    fn open(&mut self, config: &CaptureConfig, sink: AudioSinkHandle) -> Result<(), SourceError>;
    fn close(&mut self);
    fn is_open(&self) -> bool;
}

#[derive(Debug, Default)]
pub struct NoopCaptureSource {
    open: bool,
}

impl CaptureSource for NoopCaptureSource {
    fn capabilities(&self) -> CaptureCapabilities {
        CaptureCapabilities {
            microphone_available: true,
            system_audio_available: true,
        }
    }

    fn open(&mut self, _config: &CaptureConfig, _sink: AudioSinkHandle) -> Result<(), SourceError> {
        if self.open {
            return Err(SourceError::AlreadyOpen);
        }
        self.open = true;
        Ok(())
    }

    fn close(&mut self) {
        self.open = false;
    }

    fn is_open(&self) -> bool {
        self.open
    }
}

pub struct FakeCaptureSource {
    capabilities: CaptureCapabilities,
    open: bool,
    sink: Option<AudioSinkHandle>,
    pub open_calls: usize,
    pub close_calls: usize,
}

#[allow(clippy::derivable_impls)]
impl Default for FakeCaptureSource {
    fn default() -> Self {
        Self {
            capabilities: CaptureCapabilities::default(),
            open: false,
            sink: None,
            open_calls: 0,
            close_calls: 0,
        }
    }
}

impl FakeCaptureSource {
    pub fn new(capabilities: CaptureCapabilities) -> Self {
        Self {
            capabilities,
            ..Self::default()
        }
    }

    pub fn new_with_sink(capabilities: CaptureCapabilities, sink: AudioSinkHandle) -> Self {
        Self {
            capabilities,
            sink: Some(sink),
            ..Self::default()
        }
    }

    /// Emit a deterministic frame for adapter and lifecycle tests.
    ///
    /// Frames emitted while the source is closed are ignored, matching the
    /// native adapters' callback lifetime.
    pub fn emit_test_frame(
        &self,
        track: CaptureTrack,
        sample_rate: u32,
        channels: u16,
        timestamp_micros: u64,
        samples: &[f32],
    ) {
        if !self.open {
            return;
        }
        if let Some(sink) = self.sink.as_ref() {
            sink.on_audio_frame(AudioFrame {
                track,
                sample_rate,
                channels,
                timestamp_micros,
                samples,
            });
        }
    }

    #[allow(clippy::collapsible_if)]
    pub fn emit_test_error(&self, track: CaptureTrack, error: &str) {
        if self.open {
            if let Some(sink) = self.sink.as_ref() {
                sink.on_capture_error(track, error);
            }
        }
    }
}

impl CaptureSource for FakeCaptureSource {
    fn capabilities(&self) -> CaptureCapabilities {
        self.capabilities
    }

    fn open(&mut self, config: &CaptureConfig, sink: AudioSinkHandle) -> Result<(), SourceError> {
        if self.open {
            return Err(SourceError::AlreadyOpen);
        }
        if config.microphone && !self.capabilities.microphone_available {
            return Err(SourceError::MicrophoneUnavailable);
        }
        if config.system_audio && !self.capabilities.system_audio_available {
            return Err(SourceError::SystemAudioUnavailable);
        }
        self.sink = Some(sink);
        self.open = true;
        self.open_calls += 1;
        Ok(())
    }

    fn close(&mut self) {
        if self.open {
            self.close_calls += 1;
        }
        self.open = false;
        self.sink = None;
    }

    fn is_open(&self) -> bool {
        self.open
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CaptureError {
    #[error("consent is required before recording")]
    ConsentRequired,
    #[error("invalid session transition from {from:?} to {to:?}")]
    InvalidTransition {
        from: SessionState,
        to: SessionState,
    },
    #[error("capture source failed: {0}")]
    Source(#[from] SourceError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureEvent {
    Started,
    Paused,
    Resumed,
    Stopped,
}

pub type SharedCapture = Arc<Mutex<CaptureController>>;

pub struct CaptureController {
    config: CaptureConfig,
    state: SessionState,
    consent_accepted: bool,
    source: Box<dyn CaptureSource>,
    sink: AudioSinkHandle,
}

impl CaptureController {
    pub fn new(config: CaptureConfig, source: Box<dyn CaptureSource>) -> Self {
        Self::with_sink(config, source, Arc::new(NullAudioSink))
    }

    pub fn with_sink(
        config: CaptureConfig,
        source: Box<dyn CaptureSource>,
        sink: AudioSinkHandle,
    ) -> Self {
        Self {
            config,
            state: SessionState::Created,
            consent_accepted: false,
            source,
            sink,
        }
    }

    pub fn new_with_sink(
        config: CaptureConfig,
        source: Box<dyn CaptureSource>,
        sink: AudioSinkHandle,
    ) -> Self {
        Self::with_sink(config, source, sink)
    }

    pub fn state(&self) -> SessionState {
        self.state
    }

    pub fn configuration(&self) -> CaptureConfig {
        self.config
    }

    pub fn consent_accepted(&self) -> bool {
        self.consent_accepted
    }

    pub fn capabilities(&self) -> CaptureCapabilities {
        self.source.capabilities()
    }

    pub fn accept_consent(&mut self) {
        self.consent_accepted = true;
    }

    pub fn start(&mut self) -> Result<CaptureEvent, CaptureError> {
        if !self.consent_accepted {
            return Err(CaptureError::ConsentRequired);
        }
        self.transition(SessionState::Starting)?;
        if let Err(error) = self.source.open(&self.config, Arc::clone(&self.sink)) {
            self.state = SessionState::Failed;
            return Err(error.into());
        }
        self.transition(SessionState::Listening)?;
        Ok(CaptureEvent::Started)
    }

    pub fn pause(&mut self) -> Result<CaptureEvent, CaptureError> {
        self.transition(SessionState::Paused)?;
        self.source.close();
        Ok(CaptureEvent::Paused)
    }

    pub fn resume(&mut self) -> Result<CaptureEvent, CaptureError> {
        self.transition(SessionState::Starting)?;
        if let Err(error) = self.source.open(&self.config, Arc::clone(&self.sink)) {
            self.state = SessionState::Failed;
            return Err(error.into());
        }
        self.transition(SessionState::Listening)?;
        Ok(CaptureEvent::Resumed)
    }

    pub fn stop(&mut self) -> Result<CaptureEvent, CaptureError> {
        self.transition(SessionState::Stopping)?;
        self.source.close();
        self.transition(SessionState::Sealed)?;
        Ok(CaptureEvent::Stopped)
    }

    pub fn source_is_open(&self) -> bool {
        self.source.is_open()
    }

    fn transition(&mut self, next: SessionState) -> Result<(), CaptureError> {
        if !self.state.can_transition_to(next) {
            return Err(CaptureError::InvalidTransition {
                from: self.state,
                to: next,
            });
        }
        self.state = next;
        Ok(())
    }
}

#[cfg(all(feature = "macos-native", target_os = "macos"))]
pub use macos::MacOsCaptureSource;

#[cfg(not(all(feature = "macos-native", target_os = "macos")))]
pub use unsupported::MacOsCaptureSource;

#[cfg(not(all(feature = "macos-native", target_os = "macos")))]
mod unsupported {
    use super::{AudioSinkHandle, CaptureCapabilities, CaptureSource, SourceError};
    use meeting_domain::CaptureConfig;

    #[derive(Debug, Default)]
    pub struct MacOsCaptureSource;

    impl MacOsCaptureSource {
        pub fn new() -> Self {
            Self
        }
    }

    impl CaptureSource for MacOsCaptureSource {
        fn capabilities(&self) -> CaptureCapabilities {
            CaptureCapabilities::default()
        }

        fn open(
            &mut self,
            _config: &CaptureConfig,
            _sink: AudioSinkHandle,
        ) -> Result<(), SourceError> {
            Err(SourceError::UnsupportedPlatform)
        }

        fn close(&mut self) {}

        fn is_open(&self) -> bool {
            false
        }
    }
}

pub fn shared_controller(config: CaptureConfig) -> SharedCapture {
    Arc::new(Mutex::new(CaptureController::new(
        config,
        Box::new(FakeCaptureSource::new(CaptureCapabilities {
            microphone_available: true,
            system_audio_available: true,
        })),
    )))
}

pub fn language_config(language: LanguageMode) -> CaptureConfig {
    CaptureConfig::dual_source(language)
}
