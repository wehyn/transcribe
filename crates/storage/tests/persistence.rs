use meeting_domain::SessionState;
use meeting_storage::{DurableSessionRecord, LocalSessionStore, PersistenceError};
use std::fs;
use tempfile::tempdir;

#[test]
fn session_metadata_survives_a_new_store_instance() {
    let store_directory = tempdir().unwrap();
    let artifacts = tempdir().unwrap();
    let recording = artifacts.path().join("recording");
    fs::create_dir(&recording).unwrap();
    fs::write(recording.join("manifest.json"), b"manifest").unwrap();
    let transcript = artifacts.path().join("transcript.json");
    let note = artifacts.path().join("notes.md");
    fs::write(&transcript, b"transcript").unwrap();
    fs::write(&note, b"notes").unwrap();

    let mut session = DurableSessionRecord::new("session-1");
    session.state = SessionState::Processing;
    session.add_recording_artifact(&recording);
    session.transcript_path = Some(transcript.clone());
    session.note_path = Some(note.clone());

    LocalSessionStore::new(store_directory.path())
        .unwrap()
        .save(&session)
        .unwrap();

    let reloaded = LocalSessionStore::new(store_directory.path())
        .unwrap()
        .load("session-1")
        .unwrap();

    assert_eq!(reloaded, session);
    assert!(
        store_directory
            .path()
            .join("session-1/session.json")
            .is_file()
    );
    assert!(
        !store_directory
            .path()
            .join("session-1/session.json.tmp")
            .exists()
    );
}

#[test]
fn recovery_lists_sessions_left_in_non_terminal_states() {
    let store_directory = tempdir().unwrap();
    let store = LocalSessionStore::new(store_directory.path()).unwrap();

    for (id, state) in [
        ("starting", SessionState::Starting),
        ("listening", SessionState::Listening),
        ("paused", SessionState::Paused),
        ("stopping", SessionState::Stopping),
        ("sealed", SessionState::Sealed),
        ("processing", SessionState::Processing),
        ("ready", SessionState::Ready),
        ("failed", SessionState::Failed),
    ] {
        let mut session = DurableSessionRecord::new(id);
        session.state = state;
        store.save(&session).unwrap();
    }

    let mut recoverable = store
        .recoverable_sessions()
        .unwrap()
        .into_iter()
        .map(|session| session.id)
        .collect::<Vec<_>>();
    recoverable.sort();

    assert_eq!(
        recoverable,
        [
            "listening",
            "paused",
            "processing",
            "sealed",
            "starting",
            "stopping"
        ]
    );
}

#[test]
fn export_copies_recording_transcript_and_note_to_selected_directory() {
    let store_directory = tempdir().unwrap();
    let artifacts = tempdir().unwrap();
    let recording = artifacts.path().join("recording-bundle");
    fs::create_dir(&recording).unwrap();
    fs::write(recording.join("manifest.json"), b"manifest").unwrap();
    fs::write(recording.join("mixed.pcm"), b"pcm").unwrap();
    let transcript = artifacts.path().join("transcript.json");
    let note = artifacts.path().join("notes.md");
    fs::write(&transcript, b"transcript").unwrap();
    fs::write(&note, b"notes").unwrap();

    let mut session = DurableSessionRecord::new("session-1");
    session.state = SessionState::Ready;
    session.add_recording_artifact(&recording);
    session.transcript_path = Some(transcript.clone());
    session.note_path = Some(note.clone());
    let store = LocalSessionStore::new(store_directory.path()).unwrap();
    store.save(&session).unwrap();

    let export_directory = tempdir().unwrap();
    let exported = store.export("session-1", export_directory.path()).unwrap();
    let exported_root = export_directory.path().join("session-1");

    assert_eq!(exported.directory, exported_root);
    assert!(
        exported_root
            .join("recordings/recording-bundle/manifest.json")
            .is_file()
    );
    assert_eq!(
        fs::read(exported_root.join("recordings/recording-bundle/mixed.pcm")).unwrap(),
        b"pcm"
    );
    assert_eq!(
        fs::read(exported_root.join("transcript/transcript.json")).unwrap(),
        b"transcript"
    );
    assert_eq!(
        fs::read(exported_root.join("notes/notes.md")).unwrap(),
        b"notes"
    );
    assert!(recording.join("mixed.pcm").is_file());
    assert!(transcript.is_file());
    assert!(note.is_file());

    let exported_manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(exported_root.join("session.json")).unwrap()).unwrap();
    assert_eq!(
        exported_manifest["recording_artifacts"][0],
        "recordings/recording-bundle"
    );
    assert_eq!(
        exported_manifest["transcript_path"],
        "transcript/transcript.json"
    );
    assert_eq!(exported_manifest["note_path"], "notes/notes.md");
}

#[test]
fn delete_removes_and_verifies_every_registered_artifact() {
    let store_directory = tempdir().unwrap();
    let artifacts = tempdir().unwrap();
    let recording = artifacts.path().join("recording-bundle");
    fs::create_dir(&recording).unwrap();
    fs::write(recording.join("microphone.pcm"), b"pcm").unwrap();
    fs::write(recording.join("manifest.json"), b"manifest").unwrap();
    let transcript = artifacts.path().join("transcript.json");
    let note = artifacts.path().join("notes.md");
    fs::write(&transcript, b"transcript").unwrap();
    fs::write(&note, b"notes").unwrap();

    let mut session = DurableSessionRecord::new("session-1");
    session.state = SessionState::Ready;
    session.add_recording_artifact(&recording);
    session.transcript_path = Some(transcript.clone());
    session.note_path = Some(note.clone());
    let store = LocalSessionStore::new(store_directory.path()).unwrap();
    store.save(&session).unwrap();

    let deletion = store.delete("session-1").unwrap();

    assert!(deletion.verified);
    assert!(!recording.exists());
    assert!(!transcript.exists());
    assert!(!note.exists());
    assert!(!store_directory.path().join("session-1").exists());
    assert!(store.load("session-1").is_err());
}

#[test]
fn export_rejects_a_missing_artifact_without_touching_source_data() {
    let store_directory = tempdir().unwrap();
    let store = LocalSessionStore::new(store_directory.path()).unwrap();
    let mut session = DurableSessionRecord::new("session-1");
    let missing = store_directory.path().join("does-not-exist.pcm");
    session.add_recording_artifact(&missing);
    store.save(&session).unwrap();

    let result = store.export("session-1", tempdir().unwrap().path());

    assert!(matches!(result, Err(PersistenceError::MissingArtifact(path)) if path == missing));
    assert!(store.load("session-1").is_ok());
}
