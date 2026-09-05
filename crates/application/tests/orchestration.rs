use std::io;
use std::sync::{Arc, Mutex};

use meeting_application::{ApplicationError, MeetingRuntime, WorkerFactory, WorkerStartContext};
use meeting_capture::{CaptureCapabilities, CaptureSource, SourceError};
use meeting_domain::{CaptureConfig, LanguageMode, SessionState};
use meeting_storage::AudioFormat;
use tempfile::tempdir;
use whisperx_worker::{
    AlignmentStatus, CoordinatorError, FinalTranscript, LanguageMode as WorkerLanguageMode,
    TranscriptResult, TranscriptSegment, WorkerRequest, WorkerResponse, WorkerTransport,
};

#[derive(Clone, Default)]
struct FakeWorkerFactory {
    state: Arc<Mutex<FakeWorkerState>>,
}

#[derive(Default)]
struct FakeWorkerState {
    starts: Vec<WorkerStartContext>,
    requests: Vec<WorkerRequest>,
    startup_error: bool,
}

impl FakeWorkerFactory {
    fn starts(&self) -> usize {
        self.state.lock().unwrap().starts.len()
    }

    fn requests(&self) -> Vec<WorkerRequest> {
        self.state.lock().unwrap().requests.clone()
    }

    fn failing() -> Self {
        Self {
            state: Arc::new(Mutex::new(FakeWorkerState {
                startup_error: true,
                ..FakeWorkerState::default()
            })),
        }
    }
}

impl WorkerFactory for FakeWorkerFactory {
    fn start(
        &mut self,
        context: WorkerStartContext,
    ) -> Result<Box<dyn WorkerTransport>, CoordinatorError> {
        let mut state = self.state.lock().unwrap();
        state.starts.push(context);
        if state.startup_error {
            return Err(CoordinatorError::Spawn(io::Error::new(
                io::ErrorKind::NotFound,
                "fake worker unavailable",
            )));
        }
        Ok(Box::new(FakeWorker {
            state: Arc::clone(&self.state),
        }))
    }
}

struct FakeWorker {
    state: Arc<Mutex<FakeWorkerState>>,
}

impl WorkerTransport for FakeWorker {
    fn send(&mut self, request: &WorkerRequest) -> Result<WorkerResponse, CoordinatorError> {
        self.state.lock().unwrap().requests.push(request.clone());
        match request {
            WorkerRequest::TranscribeWindow(window) => {
                Ok(WorkerResponse::Transcript(TranscriptResult {
                    session_id: window.session_id.clone(),
                    sequence: window.sequence,
                    start_micros: window.start_micros,
                    end_micros: window.end_micros,
                    text: "live draft".into(),
                    words: Vec::new(),
                    language: window.language,
                    provisional: true,
                }))
            }
            WorkerRequest::TranscribeRecording { session_id, .. } => {
                Ok(WorkerResponse::FinalTranscript {
                    session_id: session_id.clone(),
                    language: WorkerLanguageMode::Taglish,
                    text: "authoritative transcript".into(),
                    segments: vec![TranscriptSegment {
                        start_micros: 0,
                        end_micros: 1_000_000,
                        text: "authoritative transcript".into(),
                        words: Vec::new(),
                        speaker: None,
                        alignment_status: AlignmentStatus::Segment,
                    }],
                })
            }
            WorkerRequest::Finalize { session_id } => Ok(WorkerResponse::Finalized {
                session_id: session_id.clone(),
            }),
            _ => Ok(WorkerResponse::Error {
                code: "unexpected_request".into(),
                message: "fake only supports transcription lifecycle".into(),
            }),
        }
    }
}

#[derive(Default)]
struct TrackingSource {
    open: bool,
    opens: Arc<Mutex<usize>>,
    closes: Arc<Mutex<usize>>,
}

impl CaptureSource for TrackingSource {
    fn capabilities(&self) -> CaptureCapabilities {
        CaptureCapabilities {
            microphone_available: true,
            system_audio_available: true,
        }
    }

    fn open(
        &mut self,
        _config: &CaptureConfig,
        _sink: meeting_capture::AudioSinkHandle,
    ) -> Result<(), SourceError> {
        self.open = true;
        *self.opens.lock().unwrap() += 1;
        Ok(())
    }

    fn close(&mut self) {
        if self.open {
            *self.closes.lock().unwrap() += 1;
        }
        self.open = false;
    }

    fn is_open(&self) -> bool {
        self.open
    }
}

fn runtime(factory: FakeWorkerFactory) -> MeetingRuntime {
    MeetingRuntime::with_worker_factory_and_pipeline(
        CaptureConfig::dual_source(LanguageMode::Taglish),
        Box::new(TrackingSource::default()),
        factory,
        whisperx_worker::WindowConfig::new(1, 0, 2),
        AudioFormat::pcm_f32(1_000, 1),
    )
}

fn one_second_of_pcm() -> Vec<u8> {
    vec![0; 4_000]
}

#[test]
fn worker_is_not_started_until_record_and_mixed_frames_reach_live_pipeline() {
    let directory = tempdir().unwrap();
    let recording_path = directory.path().join("meeting");
    let factory = FakeWorkerFactory::default();
    let mut runtime = runtime(factory.clone());

    runtime.accept_consent();
    assert_eq!(factory.starts(), 0);
    assert!(runtime.live_transcript_snapshot().is_none());

    runtime.record(&recording_path).unwrap();
    assert_eq!(factory.starts(), 1);

    let snapshot = runtime
        .append_mixed_frame(0, AudioFormat::pcm_f32(1_000, 1), &one_second_of_pcm())
        .unwrap()
        .unwrap();

    assert_eq!(snapshot.text, "live draft");
    assert_eq!(
        runtime.live_transcript_snapshot().unwrap().text,
        "live draft"
    );
    assert_eq!(
        std::fs::metadata(recording_path.join("mixed.pcm"))
            .unwrap()
            .len(),
        4_000
    );
}

#[test]
fn stop_seals_recording_before_running_authoritative_final_worker_pass() {
    let directory = tempdir().unwrap();
    let recording_path = directory.path().join("meeting");
    let factory = FakeWorkerFactory::default();
    let mut runtime = runtime(factory.clone());
    runtime.accept_consent();
    runtime.record(&recording_path).unwrap();
    runtime
        .append_mixed_frame(0, AudioFormat::pcm_f32(1_000, 1), &one_second_of_pcm())
        .unwrap();

    let result = runtime.stop_and_finalize().unwrap();

    assert_eq!(result.recording_path, recording_path);
    assert_eq!(result.transcript.text, "authoritative transcript");
    assert!(recording_path.join("manifest.json").exists());
    assert!(!runtime.source_is_open());
    assert_eq!(runtime.state(), SessionState::Sealed);
    assert_eq!(
        runtime.final_transcript().unwrap().text,
        "authoritative transcript"
    );

    let requests = factory.requests();
    assert!(matches!(requests[0], WorkerRequest::TranscribeWindow(_)));
    assert!(matches!(
        requests[1],
        WorkerRequest::TranscribeRecording { .. }
    ));
    assert!(matches!(requests[2], WorkerRequest::Finalize { .. }));
}

#[test]
fn worker_startup_failure_closes_capture_and_does_not_leave_runtime_listening() {
    let directory = tempdir().unwrap();
    let recording_path = directory.path().join("meeting");
    let factory = FakeWorkerFactory::failing();
    let mut runtime = runtime(factory.clone());
    runtime.accept_consent();

    assert!(matches!(
        runtime.record(&recording_path),
        Err(ApplicationError::Worker(CoordinatorError::Spawn(_)))
    ));
    assert_eq!(factory.starts(), 1);
    assert!(!runtime.source_is_open());
    assert_ne!(runtime.state(), SessionState::Listening);
    assert!(runtime.live_transcript_snapshot().is_none());
}

#[test]
fn worker_startup_failure_does_not_create_recording_artifacts_or_open_capture() {
    let directory = tempdir().unwrap();
    let recording_path = directory.path().join("meeting");
    let factory = FakeWorkerFactory::failing();
    let mut runtime = runtime(factory);
    runtime.accept_consent();

    assert!(runtime.record(&recording_path).is_err());
    assert!(!recording_path.exists());
    assert!(!runtime.source_is_open());
}

#[test]
fn stop_keeps_legacy_recording_path_result_while_finalizing() {
    let directory = tempdir().unwrap();
    let recording_path = directory.path().join("meeting");
    let mut runtime = runtime(FakeWorkerFactory::default());
    runtime.accept_consent();
    runtime.record(&recording_path).unwrap();

    assert_eq!(runtime.stop().unwrap(), recording_path);
    assert_eq!(
        runtime.final_transcript().unwrap().text,
        "authoritative transcript"
    );
}

#[test]
fn final_transcript_type_is_available_to_application_tests() {
    let _ = FinalTranscript {
        session_id: "session".into(),
        language: WorkerLanguageMode::English,
        text: String::new(),
        segments: Vec::new(),
    };
}
