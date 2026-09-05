use whisperx_worker::{
    AlignmentStatus, AudioWindow, LanguageMode, PROTOCOL_VERSION, ProtocolError, TranscriptResult,
    TranscriptSegment, TranscriptWord, WorkerCapabilities, WorkerRequest, WorkerResponse,
    decode_request, decode_response, encode_request, encode_response,
};

fn sample_window() -> AudioWindow {
    AudioWindow {
        session_id: "session-1".into(),
        sequence: 4,
        start_micros: 8_000_000,
        end_micros: 12_000_000,
        sample_rate: 48_000,
        channels: 1,
        pcm_f32_le: vec![0, 0, 128, 63],
        language: LanguageMode::Taglish,
    }
}

#[test]
fn window_request_round_trips_as_versioned_json() {
    let request = WorkerRequest::TranscribeWindow(sample_window());

    let encoded = encode_request(&request).unwrap();
    let decoded = decode_request(&encoded).unwrap();

    assert_eq!(decoded, request);
}

#[test]
fn hello_rejects_unknown_protocol_versions() {
    let message = r#"{"type":"hello","protocol_version":99}"#;

    assert_eq!(
        decode_request(message),
        Err(ProtocolError::UnsupportedVersion(99))
    );
}

#[test]
fn transcript_response_preserves_alignment_and_provisional_status() {
    let response = WorkerResponse::Transcript(TranscriptResult {
        session_id: "session-1".into(),
        sequence: 4,
        start_micros: 8_000_000,
        end_micros: 12_000_000,
        text: "Kumusta, team".into(),
        words: vec![TranscriptWord {
            text: "Kumusta".into(),
            start_micros: 8_200_000,
            end_micros: 8_800_000,
        }],
        language: LanguageMode::Taglish,
        provisional: true,
    });

    let encoded = encode_response(&response).unwrap();
    let decoded = decode_response(&encoded).unwrap();

    assert_eq!(decoded, response);
}

#[test]
fn worker_lifecycle_messages_are_json_lines_compatible() {
    let requests = [
        WorkerRequest::Hello {
            protocol_version: PROTOCOL_VERSION,
        },
        WorkerRequest::Capabilities,
        WorkerRequest::Finalize {
            session_id: "session-1".into(),
        },
        WorkerRequest::Shutdown,
    ];

    for request in requests {
        let line = encode_request(&request).unwrap();
        assert!(!line.contains('\n'));
        assert_eq!(decode_request(&line).unwrap(), request);
    }

    let ready = WorkerResponse::Ready {
        protocol_version: PROTOCOL_VERSION,
    };
    assert_eq!(
        decode_response(&encode_response(&ready).unwrap()).unwrap(),
        ready
    );
}

#[test]
fn final_transcript_round_trips_aligned_segments() {
    let response = WorkerResponse::FinalTranscript {
        session_id: "session-1".into(),
        language: LanguageMode::English,
        text: "Hello".into(),
        segments: vec![TranscriptSegment {
            start_micros: 0,
            end_micros: 500_000,
            text: "Hello".into(),
            words: vec![],
            speaker: Some("SPEAKER_00".into()),
            alignment_status: AlignmentStatus::Word,
        }],
    };

    let encoded = encode_response(&response).unwrap();

    assert_eq!(decode_response(&encoded).unwrap(), response);
}

#[test]
fn capabilities_round_trip_supported_language_modes() {
    let response = WorkerResponse::Capabilities(WorkerCapabilities {
        protocol_version: PROTOCOL_VERSION,
        whisperx_version: "3.7.4".into(),
        models: vec!["large-v3".into()],
        languages: vec![
            LanguageMode::English,
            LanguageMode::Filipino,
            LanguageMode::Taglish,
        ],
        diarization: false,
    });

    assert_eq!(
        decode_response(&encode_response(&response).unwrap()).unwrap(),
        response
    );
}
