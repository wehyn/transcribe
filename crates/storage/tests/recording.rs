use std::fs;

use meeting_storage::{AudioFormat, RecordingBundle, RecordingError, TrackRole};
use tempfile::tempdir;

#[test]
fn recording_bundle_writes_separate_tracks_and_manifest() {
    let directory = tempdir().unwrap();
    let mut bundle = RecordingBundle::create(directory.path().join("recording")).unwrap();

    bundle.append(TrackRole::Microphone, &[0.1, 0.2]).unwrap();
    bundle.append(TrackRole::System, &[0.3]).unwrap();
    bundle.append(TrackRole::Mixed, &[0.4, 0.5, 0.6]).unwrap();
    let sealed = bundle.seal().unwrap();

    assert_eq!(
        fs::read(sealed.track_path(TrackRole::Microphone))
            .unwrap()
            .len(),
        8
    );
    assert_eq!(
        fs::read(sealed.track_path(TrackRole::System))
            .unwrap()
            .len(),
        4
    );
    assert_eq!(
        fs::read(sealed.track_path(TrackRole::Mixed)).unwrap().len(),
        12
    );
    assert!(sealed.root().join("manifest.json").exists());
    assert!(sealed.retained());
}

#[test]
fn sealed_mixed_track_can_be_materialized_as_a_valid_pcm_wav() {
    let directory = tempdir().unwrap();
    let format = AudioFormat::pcm_f32(16_000, 1);
    let mut bundle = RecordingBundle::create_with_format(directory.path(), format).unwrap();
    bundle
        .append(TrackRole::Mixed, &[-1.0, 0.0, 1.0, f32::NAN])
        .unwrap();

    let wav = bundle.seal().unwrap().materialize_mixed_wav().unwrap();
    let bytes = fs::read(wav).unwrap();

    assert_eq!(&bytes[0..4], b"RIFF");
    assert_eq!(&bytes[8..12], b"WAVE");
    assert_eq!(&bytes[12..16], b"fmt ");
    assert_eq!(&bytes[36..40], b"data");
    assert_eq!(u32::from_le_bytes(bytes[40..44].try_into().unwrap()), 8);
    assert_eq!(bytes.len(), 52);
}

#[test]
fn wav_materialization_rejects_incomplete_f32_samples() {
    let directory = tempdir().unwrap();
    let mut bundle = RecordingBundle::create(directory.path()).unwrap();
    bundle.append(TrackRole::Mixed, &[0.25]).unwrap();
    let sealed = bundle.seal().unwrap();
    fs::write(sealed.track_path(TrackRole::Mixed), [1_u8, 2, 3]).unwrap();

    assert!(matches!(
        sealed.materialize_mixed_wav(),
        Err(RecordingError::Wav(_))
    ));
}

#[test]
fn mismatched_audio_frames_are_rejected() {
    let directory = tempdir().unwrap();
    let mut bundle = RecordingBundle::create(directory.path()).unwrap();

    assert!(matches!(
        bundle.append_frame(
            TrackRole::Microphone,
            AudioFormat::pcm_f32(44_100, 1),
            &[0.1]
        ),
        Err(RecordingError::FormatMismatch)
    ));
}
