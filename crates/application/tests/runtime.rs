use meeting_application::{ApplicationError, MeetingRuntime};
use meeting_capture::{CaptureCapabilities, FakeCaptureSource};
use meeting_domain::{CaptureConfig, LanguageMode, SessionState};
use meeting_storage::{AudioFormat, TrackRole};
use tempfile::tempdir;

fn runtime() -> MeetingRuntime {
    MeetingRuntime::new(
        CaptureConfig::dual_source(LanguageMode::Taglish),
        Box::new(FakeCaptureSource::new(CaptureCapabilities {
            microphone_available: true,
            system_audio_available: true,
        })),
    )
}

#[test]
fn runtime_never_creates_audio_artifacts_before_record() {
    let directory = tempdir().unwrap();
    let recording_path = directory.path().join("meeting");
    let mut runtime = runtime();

    runtime.accept_consent();

    assert_eq!(runtime.state(), SessionState::Created);
    assert!(!recording_path.exists());
}

#[test]
fn runtime_records_tracks_only_after_explicit_record_and_retains_them_after_stop() {
    let directory = tempdir().unwrap();
    let recording_path = directory.path().join("meeting");
    let mut runtime = runtime();
    runtime.accept_consent();

    assert!(matches!(
        runtime.append_track(TrackRole::Microphone, &[0, 0, 128, 63]),
        Err(ApplicationError::NotListening)
    ));

    runtime.record(&recording_path).unwrap();
    runtime
        .append_track(
            TrackRole::Microphone,
            &[205, 204, 204, 61, 205, 204, 76, 62],
        )
        .unwrap();
    runtime
        .append_track(TrackRole::System, &[154, 153, 153, 62])
        .unwrap();
    runtime
        .append_track(TrackRole::Mixed, &[205, 204, 76, 62])
        .unwrap();

    let sealed = runtime.stop().unwrap();

    assert_eq!(sealed, recording_path);
    assert!(recording_path.join("manifest.json").exists());
}

#[test]
fn runtime_flushes_recording_when_paused_and_resumes_only_explicitly() {
    let directory = tempdir().unwrap();
    let recording_path = directory.path().join("meeting");
    let mut runtime = runtime();
    runtime.accept_consent();
    runtime.record(&recording_path).unwrap();
    runtime
        .append_track(TrackRole::Microphone, &[0, 0, 0, 63])
        .unwrap();

    runtime.pause().unwrap();
    assert_eq!(runtime.state(), SessionState::Paused);
    assert!(recording_path.join("microphone.pcm").exists());
    assert!(matches!(
        runtime.append_track(TrackRole::System, &[154, 153, 25, 63]),
        Err(ApplicationError::NotListening)
    ));

    runtime.resume().unwrap();
    runtime
        .append_track(TrackRole::System, &[154, 153, 25, 63])
        .unwrap();
    let sealed = runtime.stop().unwrap();

    assert!(sealed.join("manifest.json").exists());
}

#[test]
fn runtime_records_frames_only_when_their_format_matches_the_bundle() {
    let directory = tempdir().unwrap();
    let recording_path = directory.path().join("meeting");
    let mut runtime = runtime();
    runtime.accept_consent();
    runtime.record(&recording_path).unwrap();

    assert!(
        runtime
            .record_frame(
                TrackRole::Microphone,
                AudioFormat::default(),
                &[51, 51, 51, 63]
            )
            .is_ok()
    );
    assert!(matches!(
        runtime.record_frame(
            TrackRole::System,
            AudioFormat {
                sample_rate: 44_100,
                channels: 1,
            },
            &[205, 204, 76, 63]
        ),
        Err(ApplicationError::Recording(
            meeting_storage::RecordingError::FormatMismatch
        ))
    ));
}

#[test]
fn runtime_rejects_partial_pcm_samples_without_writing_them() {
    let directory = tempdir().unwrap();
    let recording_path = directory.path().join("meeting");
    let mut runtime = runtime();
    runtime.accept_consent();
    runtime.record(&recording_path).unwrap();

    assert!(matches!(
        runtime.append_track(TrackRole::Microphone, &[0, 0, 0]),
        Err(ApplicationError::InvalidPcm)
    ));
    assert_eq!(
        std::fs::metadata(recording_path.join("microphone.pcm"))
            .unwrap()
            .len(),
        0
    );
}
