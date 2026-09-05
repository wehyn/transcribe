use whisperx_worker::{
    AudioWindow, CoordinatorError, WorkerCoordinator, WorkerRequest, WorkerResponse,
    WorkerTransport,
};

struct FailureTransport;

impl WorkerTransport for FailureTransport {
    fn send(&mut self, request: &WorkerRequest) -> Result<WorkerResponse, CoordinatorError> {
        match request {
            WorkerRequest::TranscribeWindow(_) => Ok(WorkerResponse::Error {
                code: "model_unavailable".into(),
                message: "model is not installed".into(),
            }),
            _ => Ok(WorkerResponse::Finalized {
                session_id: "session".into(),
            }),
        }
    }
}

fn window() -> AudioWindow {
    AudioWindow {
        session_id: "session".into(),
        sequence: 1,
        start_micros: 0,
        end_micros: 1_000_000,
        sample_rate: 16_000,
        channels: 1,
        pcm_f32_le: Vec::new(),
        language: whisperx_worker::LanguageMode::English,
    }
}

#[test]
fn best_effort_live_window_keeps_capture_path_alive_on_worker_error() {
    let mut coordinator = WorkerCoordinator::new(FailureTransport);

    let result = coordinator.send_window_best_effort(window()).unwrap();

    assert_eq!(result, None);
    assert_eq!(coordinator.in_flight_len(), 0);
}
