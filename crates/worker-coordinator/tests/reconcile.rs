use whisperx_worker::{LanguageMode, TranscriptReconciler, TranscriptResult, result_for};

fn result(sequence: u64, text: &str, provisional: bool) -> TranscriptResult {
    result_for(
        &whisperx_worker::AudioWindow {
            session_id: "session-1".into(),
            sequence,
            start_micros: sequence * 3_000_000,
            end_micros: (sequence + 1) * 3_000_000,
            sample_rate: 48_000,
            channels: 1,
            pcm_f32_le: Vec::new(),
            language: LanguageMode::Taglish,
        },
        text,
        provisional,
    )
}

#[test]
fn reconciler_orders_results_and_joins_nonempty_text() {
    let mut reconciler = TranscriptReconciler::new();
    reconciler.accept(result(1, "world", true));
    reconciler.accept(result(0, "Hello", true));
    reconciler.accept(result(2, "", true));

    let snapshot = reconciler.snapshot().unwrap();

    assert_eq!(snapshot.text, "Hello world");
    assert_eq!(snapshot.segments.len(), 3);
    assert_eq!(snapshot.segments[0].sequence, 0);
}

#[test]
fn authoritative_result_replaces_provisional_result_for_same_window() {
    let mut reconciler = TranscriptReconciler::new();
    reconciler.accept(result(0, "helo", true));
    reconciler.accept(result(0, "hello", false));

    let snapshot = reconciler.snapshot().unwrap();

    assert_eq!(snapshot.text, "hello");
    assert!(!snapshot.segments[0].provisional);
}

#[test]
fn later_provisional_result_cannot_overwrite_authoritative_result() {
    let mut reconciler = TranscriptReconciler::new();
    reconciler.accept(result(0, "hello", false));
    reconciler.accept(result(0, "helo", true));

    assert_eq!(reconciler.snapshot().unwrap().text, "hello");
}

#[test]
fn finalize_discards_old_windows_and_returns_authoritative_snapshot() {
    let mut reconciler = TranscriptReconciler::new();
    reconciler.accept(result(0, "draft", true));

    let final_snapshot =
        reconciler.finalize([result(0, "final", false), result(1, "notes", false)]);

    assert_eq!(final_snapshot.text, "final notes");
    assert_eq!(reconciler.len(), 2);
}
