use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionRecord {
    pub id: Uuid,
    pub title: String,
    pub state: String,
    pub recording_root: PathBuf,
    pub transcript_path: Option<PathBuf>,
    pub draft_notes_path: Option<PathBuf>,
    pub final_notes_path: Option<PathBuf>,
}

#[derive(Debug, Error)]
pub enum SessionStoreError {
    #[error("session store I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error("session store JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("session {0} was not found")]
    NotFound(Uuid),
    #[error("refusing to delete path outside the session root")]
    UnsafePath,
}

#[derive(Debug, Clone)]
pub struct SessionStore {
    root: PathBuf,
}

impl SessionStore {
    pub fn new(root: impl AsRef<Path>) -> Result<Self, SessionStoreError> {
        fs::create_dir_all(root.as_ref())?;
        Ok(Self {
            root: root.as_ref().to_path_buf(),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn save(&self, record: &SessionRecord) -> Result<(), SessionStoreError> {
        let path = self.metadata_path(record.id);
        let temporary = path.with_extension("json.tmp");
        let bytes = serde_json::to_vec_pretty(record)?;
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(temporary, path)?;
        Ok(())
    }

    pub fn load(&self, id: Uuid) -> Result<SessionRecord, SessionStoreError> {
        let path = self.metadata_path(id);
        if !path.exists() {
            return Err(SessionStoreError::NotFound(id));
        }
        Ok(serde_json::from_slice(&fs::read(path)?)?)
    }

    pub fn recoverable(&self) -> Result<Vec<SessionRecord>, SessionStoreError> {
        let mut records = Vec::new();
        for entry in fs::read_dir(&self.root)? {
            let path = entry?.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let bytes = fs::read(&path)?;
            let record: SessionRecord = serde_json::from_slice(&bytes)?;
            if matches!(
                record.state.as_str(),
                "starting" | "listening" | "paused" | "stopping" | "sealed" | "processing"
            ) {
                records.push(record);
            }
        }
        records.sort_by_key(|record: &SessionRecord| record.id);
        Ok(records)
    }

    pub fn export(
        &self,
        id: Uuid,
        destination: impl AsRef<Path>,
    ) -> Result<PathBuf, SessionStoreError> {
        let record = self.load(id)?;
        let destination = destination.as_ref().join(id.to_string());
        fs::create_dir_all(&destination)?;
        copy_if_present(&record.recording_root.join("manifest.json"), &destination)?;
        copy_if_present(&record.recording_root.join("microphone.pcm"), &destination)?;
        copy_if_present(&record.recording_root.join("system.pcm"), &destination)?;
        copy_if_present(&record.recording_root.join("mixed.pcm"), &destination)?;
        for path in [
            record.transcript_path.as_ref(),
            record.draft_notes_path.as_ref(),
            record.final_notes_path.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            copy_if_present(path, &destination)?;
        }
        let bytes = serde_json::to_vec_pretty(&record)?;
        let temporary = destination.join("session.json.tmp");
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(temporary, destination.join("session.json"))?;
        Ok(destination)
    }

    pub fn delete_verified(&self, id: Uuid) -> Result<(), SessionStoreError> {
        let record = self.load(id)?;
        let artifacts = [
            Some(record.recording_root.clone()),
            record.transcript_path.clone(),
            record.draft_notes_path.clone(),
            record.final_notes_path.clone(),
        ];
        if artifacts
            .iter()
            .flatten()
            .any(|path| !path.starts_with(&self.root))
        {
            return Err(SessionStoreError::UnsafePath);
        }
        for path in artifacts.iter().flatten() {
            if path.is_dir() {
                fs::remove_dir_all(path)?;
            } else if path.exists() {
                fs::remove_file(path)?;
            }
        }
        fs::remove_file(self.metadata_path(id))?;
        if self.load(id).is_ok() {
            return Err(SessionStoreError::Io(io::Error::other(
                "session metadata deletion could not be verified",
            )));
        }
        if let Some(path) = artifacts.iter().flatten().find(|path| path.exists()) {
            return Err(SessionStoreError::Io(io::Error::other(format!(
                "artifact deletion could not be verified: {}",
                path.display()
            ))));
        }
        Ok(())
    }

    fn metadata_path(&self, id: Uuid) -> PathBuf {
        self.root.join(format!("{id}.json"))
    }
}

fn copy_if_present(source: &Path, destination: &Path) -> io::Result<()> {
    if source.is_file() {
        let filename = source
            .file_name()
            .ok_or_else(|| io::Error::other("source has no filename"))?;
        fs::copy(source, destination.join(filename))?;
    }
    Ok(())
}
