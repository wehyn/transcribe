use whisperx_worker::{LanguageMode, RollingWindowBuffer, WindowConfig, WindowError};

fn push_seconds(buffer: &mut RollingWindowBuffer, start: u64, seconds: u64) {
    let samples_per_second = 48_000_u64;
    let bytes = vec![0_u8; (samples_per_second * seconds * 4) as usize];
    buffer.push_pcm(start, &bytes).unwrap();
}

#[test]
fn rolling_buffer_emits_overlapping_windows_with_monotonic_sequences() {
    let mut buffer = RollingWindowBuffer::new(
        WindowConfig::new(4, 1, 4),
        "session-1",
        48_000,
        1,
        LanguageMode::English,
    );

    push_seconds(&mut buffer, 0, 4);
    push_seconds(&mut buffer, 4_000_000, 3);

    let first = buffer.pop_window().unwrap();
    let second = buffer.pop_window().unwrap();

    assert_eq!(first.sequence, 0);
    assert_eq!(second.sequence, 1);
    assert_eq!(first.start_micros, 0);
    assert_eq!(first.end_micros, 4_000_000);
    assert_eq!(second.start_micros, 3_000_000);
    assert_eq!(second.end_micros, 7_000_000);
    assert_eq!(first.language, LanguageMode::English);
}

#[test]
fn rolling_buffer_applies_backpressure_without_dropping_the_oldest_window() {
    let mut buffer = RollingWindowBuffer::new(
        WindowConfig::new(1, 0, 1),
        "session-2",
        1_000,
        1,
        LanguageMode::Taglish,
    );
    let one_second = vec![0_u8; 4_000];

    buffer.push_pcm(0, &one_second).unwrap();
    assert_eq!(
        buffer.push_pcm(1_000_000, &one_second),
        Err(WindowError::Backpressure)
    );
    assert_eq!(buffer.pending_len(), 1);
    assert_eq!(buffer.pop_window().unwrap().sequence, 0);
}

#[test]
fn rolling_buffer_rejects_out_of_order_audio() {
    let mut buffer = RollingWindowBuffer::new(
        WindowConfig::new(2, 0, 2),
        "session-3",
        1_000,
        1,
        LanguageMode::Filipino,
    );
    let samples = vec![0_u8; 4_000];

    buffer.push_pcm(1_000_000, &samples).unwrap();

    assert_eq!(
        buffer.push_pcm(500_000, &samples),
        Err(WindowError::NonMonotonicTimestamp)
    );
}
