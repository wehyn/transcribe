use meeting_capture::{
    AudioFrame, AudioSink, CaptureCapabilities, CaptureController, CaptureError, CaptureEvent,
    CaptureSource, CaptureTrack, FakeCaptureSource, MacOsCaptureSource, SourceError,
};
use meeting_domain::{CaptureConfig, LanguageMode, SessionState};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct TestSink {
    #[allow(clippy::type_complexity)]
    frames: Mutex<Vec<(CaptureTrack, u32, u16, u64, Vec<f32>)>>,
    errors: Mutex<Vec<(CaptureTrack, String)>>,
}

impl AudioSink for TestSink {
    fn on_audio_frame(&self, frame: AudioFrame<'_>) {
        self.frames.lock().unwrap().push((
            frame.track,
            frame.sample_rate,
            frame.channels,
            frame.timestamp_micros,
            frame.samples.to_vec(),
        ));
    }

    fn on_capture_error(&self, track: CaptureTrack, error: &str) {
        self.errors.lock().unwrap().push((track, error.to_owned()));
    }
}

fn controller() -> CaptureController {
    CaptureController::new(
        CaptureConfig::dual_source(LanguageMode::English),
        Box::new(FakeCaptureSource::new(CaptureCapabilities {
            microphone_available: true,
            system_audio_available: true,
        })),
    )
}

#[test]
fn sink_receives_owned_timestamped_frames_only_after_open() {
    let sink = Arc::new(TestSink::default());
    let mut source = FakeCaptureSource::new_with_sink(
        CaptureCapabilities {
            microphone_available: true,
            system_audio_available: true,
        },
        sink.clone(),
    );

    source.emit_test_frame(CaptureTrack::Microphone, 48_000, 1, 123, &[0.25, -0.5]);
    assert!(sink.frames.lock().unwrap().is_empty());

    source
        .open(
            &CaptureConfig::dual_source(LanguageMode::English),
            sink.clone(),
        )
        .unwrap();
    source.emit_test_frame(CaptureTrack::Microphone, 48_000, 1, 123, &[0.25, -0.5]);

    assert_eq!(
        sink.frames.lock().unwrap().as_slice(),
        &[(CaptureTrack::Microphone, 48_000, 1, 123, vec![0.25, -0.5])]
    );
}

#[test]
fn closing_source_stops_callbacks_and_releases_sink() {
    let sink = Arc::new(TestSink::default());
    let mut source = FakeCaptureSource::new_with_sink(
        CaptureCapabilities {
            microphone_available: true,
            system_audio_available: true,
        },
        sink.clone(),
    );
    source
        .open(
            &CaptureConfig::dual_source(LanguageMode::English),
            sink.clone(),
        )
        .unwrap();
    source.emit_test_frame(CaptureTrack::System, 48_000, 2, 456, &[1.0, 0.0]);
    source.close();
    source.emit_test_frame(CaptureTrack::System, 48_000, 2, 789, &[0.0, 1.0]);

    let frames = sink.frames.lock().unwrap();
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].3, 456);
}

#[test]
fn macos_source_is_deterministically_unsupported_on_linux() {
    let mut source = MacOsCaptureSource::new();
    assert_eq!(source.capabilities(), CaptureCapabilities::default());
    assert_eq!(
        source.open(
            &CaptureConfig::dual_source(LanguageMode::English),
            Arc::new(meeting_capture::NullAudioSink),
        ),
        Err(SourceError::UnsupportedPlatform)
    );
    assert!(!source.is_open());
}

#[test]
fn capability_inspection_does_not_open_source() {
    let controller = controller();
    let capabilities = controller.capabilities();

    assert!(capabilities.microphone_available);
    assert!(capabilities.system_audio_available);
    assert!(!controller.source_is_open());
}

#[test]
fn unavailable_selected_source_fails_before_capture_starts() {
    let source = FakeCaptureSource::new(CaptureCapabilities {
        microphone_available: true,
        system_audio_available: false,
    });
    let mut controller = CaptureController::new(
        CaptureConfig::dual_source(LanguageMode::English),
        Box::new(source),
    );
    controller.accept_consent();

    assert_eq!(
        controller.start(),
        Err(CaptureError::Source(SourceError::SystemAudioUnavailable))
    );
    assert_eq!(controller.state(), SessionState::Failed);
    assert!(!controller.source_is_open());
}

#[test]
fn no_audio_source_is_open_before_record() {
    let controller = controller();

    assert_eq!(controller.state(), SessionState::Created);
    assert!(!controller.source_is_open());
}

#[test]
fn record_requires_consent_and_is_the_only_start_transition() {
    let mut controller = controller();

    assert_eq!(controller.start(), Err(CaptureError::ConsentRequired));
    assert_eq!(controller.state(), SessionState::Created);
    assert!(!controller.source_is_open());

    controller.accept_consent();
    assert_eq!(controller.start(), Ok(CaptureEvent::Started));
    assert_eq!(controller.state(), SessionState::Listening);
    assert!(controller.source_is_open());
}

#[test]
fn pause_resume_and_stop_explicitly_control_the_source() {
    let mut controller = controller();
    controller.accept_consent();
    controller.start().unwrap();

    assert_eq!(controller.pause(), Ok(CaptureEvent::Paused));
    assert_eq!(controller.state(), SessionState::Paused);
    assert!(!controller.source_is_open());

    assert_eq!(controller.resume(), Ok(CaptureEvent::Resumed));
    assert_eq!(controller.state(), SessionState::Listening);
    assert!(controller.source_is_open());

    assert_eq!(controller.stop(), Ok(CaptureEvent::Stopped));
    assert_eq!(controller.state(), SessionState::Sealed);
    assert!(!controller.source_is_open());
}

#[test]
fn sealed_session_cannot_start_again_without_implicit_restart() {
    let mut controller = controller();
    controller.accept_consent();
    controller.start().unwrap();
    controller.stop().unwrap();

    assert!(matches!(
        controller.start(),
        Err(CaptureError::InvalidTransition {
            from: SessionState::Sealed,
            to: SessionState::Starting
        })
    ));
    assert!(!controller.source_is_open());
}

#[test]
fn capture_source_trait_is_only_opened_by_record_and_closes_on_stop() {
    let source = FakeCaptureSource::new(CaptureCapabilities {
        microphone_available: true,
        system_audio_available: true,
    });
    let mut controller = CaptureController::new(
        CaptureConfig::dual_source(LanguageMode::Filipino),
        Box::new(source),
    );

    assert!(controller.capabilities().system_audio_available);
    assert!(!controller.source_is_open());

    controller.accept_consent();
    controller.start().unwrap();
    assert!(controller.source_is_open());
    controller.stop().unwrap();
    assert!(!controller.source_is_open());
}

#[test]
fn source_trait_can_be_used_for_platform_specific_adapters() {
    fn assert_capture_source<T: CaptureSource>() {}

    assert_capture_source::<FakeCaptureSource>();
}
