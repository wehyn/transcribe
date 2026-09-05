use std::collections::VecDeque;

use whisperx_worker::{
    CoordinatorError, FinalTranscript, LanguageMode, LivePipeline, TranscriptResult,
    TranscriptSegment, WindowConfig, WorkerRequest, WorkerResponse, WorkerTransport,
};

struct FakeWorker {
    responses: VecDeque<WorkerResponse>,
}

impl FakeWorker {
    fn new() -> Self {
        Self {
            responses: VecDeque::from([
                WorkerResponse::Transcript(TranscriptResult {
                    session_id: "session-1".into(),
                    sequence: 0,
                    start_micros: 0,
                    end_micros: 1_000_000,
                    text: "Kumusta team".into(),
                    words: Vec::new(),
                    language: LanguageMode::Taglish,
                    provisional: true,
                }),
                WorkerResponse::FinalTranscript {
                    session_id: "session-1".into(),
                    language: LanguageMode::Taglish,
                    text: "Kumusta team".into(),
                    segments: vec![TranscriptSegment {
                        start_micros: 0,
                        end_micros: 1_000_000,
                        text: "Kumusta team".into(),
                        words: Vec::new(),
                        speaker: None,
                        alignment_status: whisperx_worker::AlignmentStatus::Segment,
                    }],
                },
                WorkerResponse::Finalized {
                    session_id: "session-1".into(),
                },
            ]),
        }
    }
}

impl WorkerTransport for FakeWorker {
    fn send(&mut self, request: &WorkerRequest) -> Result<WorkerResponse, CoordinatorError> {
        match request {
            WorkerRequest::TranscribeWindow(window) => {
                let response = self.responses.pop_front().expect("window response");
                match response {
                    WorkerResponse::Transcript(mut result) => {
                        result.session_id = window.session_id.clone();
                        result.sequence = window.sequence;
                        Ok(WorkerResponse::Transcript(result))
                    }
                    other => Ok(other),
                }
            }
            WorkerRequest::TranscribeRecording { .. } => {
                Ok(self.responses.pop_front().expect("recording response"))
            }
            WorkerRequest::Finalize { .. } => {
                Ok(self.responses.pop_front().expect("finalize response"))
            }
            _ => unreachable!("handshake is outside this pipeline test"),
        }
    }
}

fn pcm_for_one_second() -> Vec<u8> {
    vec![0; 4_000]
}

#[test]
fn live_pipeline_emits_reconciled_snapshot_after_a_complete_window() {
    let mut pipeline = LivePipeline::new(
        FakeWorker::new(),
        WindowConfig::new(1, 0, 2),
        "session-1",
        1_000,
        1,
        LanguageMode::Taglish,
    );

    let snapshot = pipeline.push_pcm(0, &pcm_for_one_second()).unwrap();

    assert_eq!(snapshot.unwrap().text, "Kumusta team");
}

#[test]
fn live_pipeline_replaces_live_windows_with_authoritative_final_transcript() {
    let mut pipeline = LivePipeline::new(
        FakeWorker::new(),
        WindowConfig::new(1, 0, 2),
        "session-1",
        1_000,
        1,
        LanguageMode::Taglish,
    );
    pipeline.push_pcm(0, &pcm_for_one_second()).unwrap();

    let final_transcript = pipeline
        .finalize("session-1", "/tmp/mixed.wav", LanguageMode::Taglish)
        .unwrap();

    assert_eq!(
        final_transcript,
        FinalTranscript {
            session_id: "session-1".into(),
            language: LanguageMode::Taglish,
            text: "Kumusta team".into(),
            segments: vec![TranscriptSegment {
                start_micros: 0,
                end_micros: 1_000_000,
                text: "Kumusta team".into(),
                words: Vec::new(),
                speaker: None,
                alignment_status: whisperx_worker::AlignmentStatus::Segment,
            }],
        }
    );
}
