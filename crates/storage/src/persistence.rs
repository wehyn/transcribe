use meeting_domain::SessionState;
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use thiserror::Error;

const SESSION_FILE: &str = "session.json";
const TEMP_SESSION_FILE: &str = "session.json.tmp";

/// The durable metadata needed to recover or manage one local session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionRecord {
    pub id: String,
    pub state: SessionState,
    pub recording_artifacts: Vec<PathBuf>,
    pub transcript_path: Option<PathBuf>,
    pub note_path: Option<PathBuf>,
}

impl SessionRecord {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            state: SessionState::Created,
            recording_artifacts: Vec::new(),
            transcript_path: None,
            note_path: None,
        }
    }

    pub fn add_recording_artifact(&mut self, path: impl AsRef<Path>) {
        let path = path.as_ref().to_path_buf();
        if !self.recording_artifacts.contains(&path) {
            self.recording_artifacts.push(path);
        }
    }
}

#[derive(Debug, Error)]
pub enum PersistenceError {
    #[error("session store directory could not be created: {0}")]
    CreateStoreDirectory(#[source] io::Error),
    #[error("session id is invalid: {0}")]
    InvalidSessionId(String),
    #[error("session metadata could not be read: {0}")]
    ReadMetadata(#[source] io::Error),
    #[error("session metadata could not be parsed: {0}")]
    ParseMetadata(#[source] serde_json::Error),
    #[error("session metadata could not be serialized: {0}")]
    SerializeMetadata(#[source] serde_json::Error),
    #[error("session metadata could not be written: {0}")]
    WriteMetadata(#[source] io::Error),
    #[error("session metadata could not be removed: {0}")]
    RemoveMetadata(#[source] io::Error),
    #[error("session was not found: {0}")]
    SessionNotFound(String),
    #[error("artifact path does not exist: {0}")]
    MissingArtifact(PathBuf),
    #[error("artifact path is not a file or directory: {0}")]
    InvalidArtifact(PathBuf),
    #[error("artifact metadata could not be inspected: {0}")]
    InspectArtifact(#[source] io::Error),
    #[error("artifact could not be copied: {0}")]
    CopyArtifact(#[source] io::Error),
    #[error("artifact could not be removed: {0}")]
    RemoveArtifact(#[source] io::Error),
    #[error("deletion verification failed for: {0}")]
    DeletionVerification(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportedSession {
    pub directory: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeletionReport {
    pub verified: bool,
}

#[derive(Debug, Clone)]
pub struct LocalSessionStore {
    root: PathBuf,
}

impl LocalSessionStore {
    pub fn new(root: impl AsRef<Path>) -> Result<Self, PersistenceError> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(&root).map_err(PersistenceError::CreateStoreDirectory)?;
        Ok(Self { root })
    }

    /// Atomically replace a session's JSON metadata.
    pub fn save(&self, session: &SessionRecord) -> Result<(), PersistenceError> {
        validate_session_id(&session.id)?;
        let directory = self.session_directory(&session.id);
        fs::create_dir_all(&directory).map_err(PersistenceError::WriteMetadata)?;
        let serialized =
            serde_json::to_vec_pretty(session).map_err(PersistenceError::SerializeMetadata)?;
        atomic_write(&directory.join(SESSION_FILE), &serialized)
            .map_err(PersistenceError::WriteMetadata)
    }

    pub fn load(&self, id: &str) -> Result<SessionRecord, PersistenceError> {
        validate_session_id(id)?;
        let path = self.session_directory(id).join(SESSION_FILE);
        let bytes = fs::read(path).map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                PersistenceError::SessionNotFound(id.to_string())
            } else {
                PersistenceError::ReadMetadata(error)
            }
        })?;
        serde_json::from_slice(&bytes).map_err(PersistenceError::ParseMetadata)
    }

    /// Find sessions whose lifecycle indicates that the process may have
    /// stopped before a clean stop/finalization checkpoint.
    pub fn recoverable_sessions(&self) -> Result<Vec<SessionRecord>, PersistenceError> {
        let entries = fs::read_dir(&self.root).map_err(PersistenceError::ReadMetadata)?;
        let mut sessions = Vec::new();
        for entry in entries {
            let entry = entry.map_err(PersistenceError::ReadMetadata)?;
            if !entry
                .file_type()
                .map_err(PersistenceError::ReadMetadata)?
                .is_dir()
            {
                continue;
            }
            let id = entry.file_name().to_string_lossy().into_owned();
            let path = entry.path().join(SESSION_FILE);
            if !path.is_file() {
                continue;
            }
            let session = self.load(&id)?;
            if is_recoverable(session.state) {
                sessions.push(session);
            }
        }
        Ok(sessions)
    }

    /// Copy all registered artifacts and a portable metadata manifest into a
    /// directory selected by the caller. The session id scopes the export so
    /// multiple sessions can be exported to the same parent directory.
    pub fn export(
        &self,
        id: &str,
        destination: impl AsRef<Path>,
    ) -> Result<ExportedSession, PersistenceError> {
        let session = self.load(id)?;
        let destination = destination.as_ref().join(id);
        fs::create_dir_all(&destination).map_err(PersistenceError::CopyArtifact)?;

        for source in &session.recording_artifacts {
            validate_artifact(source)?;
            let name = artifact_name(source)?;
            copy_artifact(source, &destination.join("recordings").join(name))
                .map_err(PersistenceError::CopyArtifact)?;
        }
        if let Some(source) = session.transcript_path.as_ref() {
            validate_artifact(source)?;
            let name = artifact_name(source)?;
            copy_artifact(source, &destination.join("transcript").join(name))
                .map_err(PersistenceError::CopyArtifact)?;
        }
        if let Some(source) = session.note_path.as_ref() {
            validate_artifact(source)?;
            let name = artifact_name(source)?;
            copy_artifact(source, &destination.join("notes").join(name))
                .map_err(PersistenceError::CopyArtifact)?;
        }

        let manifest = ExportManifest::from_session(&session);
        let serialized =
            serde_json::to_vec_pretty(&manifest).map_err(PersistenceError::SerializeMetadata)?;
        atomic_write(&destination.join(SESSION_FILE), &serialized)
            .map_err(PersistenceError::CopyArtifact)?;

        Ok(ExportedSession {
            directory: destination,
        })
    }

    /// Remove every registered artifact and the session metadata, then verify
    /// that none of those paths remain. Already-missing artifacts are treated
    /// as successfully deleted, making cleanup safe to retry.
    pub fn delete(&self, id: &str) -> Result<DeletionReport, PersistenceError> {
        let session = self.load(id)?;
        let artifacts = all_artifacts(&session);
        for artifact in &artifacts {
            if path_exists(artifact) {
                remove_artifact(artifact).map_err(PersistenceError::RemoveArtifact)?;
            }
        }

        let directory = self.session_directory(id);
        if directory.exists() {
            fs::remove_dir_all(&directory).map_err(PersistenceError::RemoveMetadata)?;
        }
        for artifact in artifacts {
            if path_exists(&artifact) {
                return Err(PersistenceError::DeletionVerification(artifact));
            }
        }
        if directory.exists() {
            return Err(PersistenceError::DeletionVerification(directory));
        }
        Ok(DeletionReport { verified: true })
    }

    fn session_directory(&self, id: &str) -> PathBuf {
        self.root.join(id)
    }
}

#[derive(Debug, Serialize)]
struct ExportManifest {
    id: String,
    state: SessionState,
    recording_artifacts: Vec<PathBuf>,
    transcript_path: Option<PathBuf>,
    note_path: Option<PathBuf>,
}

impl ExportManifest {
    fn from_session(session: &SessionRecord) -> Self {
        Self {
            id: session.id.clone(),
            state: session.state,
            recording_artifacts: session
                .recording_artifacts
                .iter()
                .filter_map(|path| artifact_name(path).ok())
                .map(|name| PathBuf::from("recordings").join(name))
                .collect(),
            transcript_path: session
                .transcript_path
                .as_ref()
                .and_then(|path| artifact_name(path).ok())
                .map(|name| PathBuf::from("transcript").join(name)),
            note_path: session
                .note_path
                .as_ref()
                .and_then(|path| artifact_name(path).ok())
                .map(|name| PathBuf::from("notes").join(name)),
        }
    }
}

fn is_recoverable(state: SessionState) -> bool {
    matches!(
        state,
        SessionState::Starting
            | SessionState::Listening
            | SessionState::Paused
            | SessionState::Stopping
            | SessionState::Sealed
            | SessionState::Processing
    )
}

fn validate_session_id(id: &str) -> Result<(), PersistenceError> {
    if id.is_empty()
        || id == "."
        || id == ".."
        || id.contains('/')
        || id.contains('\\')
        || id.contains('\0')
    {
        return Err(PersistenceError::InvalidSessionId(id.to_string()));
    }
    Ok(())
}

fn all_artifacts(session: &SessionRecord) -> Vec<PathBuf> {
    let mut artifacts = Vec::new();
    for artifact in session
        .recording_artifacts
        .iter()
        .chain(session.transcript_path.iter())
        .chain(session.note_path.iter())
    {
        if !artifacts.contains(artifact) {
            artifacts.push(artifact.clone());
        }
    }
    artifacts
}

fn validate_artifact(path: &Path) -> Result<(), PersistenceError> {
    if !path_exists(path) {
        return Err(PersistenceError::MissingArtifact(path.to_path_buf()));
    }
    let metadata = fs::symlink_metadata(path).map_err(PersistenceError::InspectArtifact)?;
    if !metadata.file_type().is_file() && !metadata.file_type().is_dir() {
        return Err(PersistenceError::InvalidArtifact(path.to_path_buf()));
    }
    Ok(())
}

fn artifact_name(path: &Path) -> Result<&std::ffi::OsStr, PersistenceError> {
    path.file_name()
        .filter(|name| !name.is_empty())
        .ok_or_else(|| PersistenceError::InvalidArtifact(path.to_path_buf()))
}

fn copy_artifact(source: &Path, target: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(source)?;
    if metadata.file_type().is_dir() {
        fs::create_dir_all(target)?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            copy_artifact(&entry.path(), &target.join(entry.file_name()))?;
        }
    } else {
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(source, target)?;
    }
    Ok(())
}

fn remove_artifact(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

fn path_exists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let temporary = path.with_file_name(TEMP_SESSION_FILE);
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&temporary)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    fs::rename(&temporary, path)?;
    Ok(())
}
