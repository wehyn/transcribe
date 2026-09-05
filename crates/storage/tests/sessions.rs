use std::fs;

use meeting_storage::{SessionRecord, SessionStore};
use tempfile::tempdir;
use uuid::Uuid;

#[test]
fn session_store_round_trips_records_and_recovers_them() {
    let directory = tempdir().unwrap();
    let store = SessionStore::new(directory.path()).unwrap();
    let record = SessionRecord {
        id: Uuid::new_v4(),
        title: "Weekly sync".into(),
        state: "processing".into(),
        recording_root: directory.path().join("recording"),
        transcript_path: None,
        draft_notes_path: None,
        final_notes_path: None,
    };

    store.save(&record).unwrap();

    assert_eq!(store.load(record.id).unwrap(), record);
    assert_eq!(store.recoverable().unwrap(), vec![record]);
}

#[test]
fn session_store_exports_and_verified_deletion_remove_all_known_artifacts() {
    let directory = tempdir().unwrap();
    let root = directory.path().join("sessions");
    let store = SessionStore::new(&root).unwrap();
    let recording = root.join("recording");
    fs::create_dir_all(&recording).unwrap();
    fs::write(recording.join("manifest.json"), "{}\n").unwrap();
    fs::write(recording.join("microphone.pcm"), [1_u8, 2]).unwrap();
    let transcript = root.join("transcript.json");
    fs::write(&transcript, "{}\n").unwrap();
    let record = SessionRecord {
        id: Uuid::new_v4(),
        title: "Export me".into(),
        state: "ready".into(),
        recording_root: recording.clone(),
        transcript_path: Some(transcript.clone()),
        draft_notes_path: None,
        final_notes_path: None,
    };
    store.save(&record).unwrap();

    let export = store
        .export(record.id, directory.path().join("exports"))
        .unwrap();
    assert!(export.join("manifest.json").exists());
    assert!(export.join("transcript.json").exists());

    store.delete_verified(record.id).unwrap();
    assert!(!recording.exists());
    assert!(!transcript.exists());
    assert!(store.load(record.id).is_err());
}

#[test]
fn session_store_rejects_artifacts_outside_its_root() {
    let directory = tempdir().unwrap();
    let root = directory.path().join("sessions");
    let store = SessionStore::new(&root).unwrap();
    let record = SessionRecord {
        id: Uuid::new_v4(),
        title: "Unsafe".into(),
        state: "ready".into(),
        recording_root: directory.path().join("outside"),
        transcript_path: None,
        draft_notes_path: None,
        final_notes_path: None,
    };
    store.save(&record).unwrap();

    assert!(matches!(
        store.delete_verified(record.id),
        Err(meeting_storage::SessionStoreError::UnsafePath)
    ));
}
