use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use thiserror::Error;

const MODEL_ID: &str = "large-v3";
const MODEL_REPOSITORY: &str = "Systran/faster-whisper-large-v3";
const MODEL_REVISION: &str = "edaa852ec7e145841d8ffdb056a99866b5f0a478";
const MODEL_LICENSE: &str = "MIT";
const MODEL_ATTRIBUTION: &str =
    "Systran faster-whisper-large-v3, converted from OpenAI Whisper large-v3";
const MODEL_LOCK: &str = "download.lock";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelAsset {
    pub path: String,
    pub size: u64,
    pub sha256: String,
}

impl ModelAsset {
    pub fn for_test(path: impl Into<String>, bytes: &[u8]) -> Self {
        Self {
            path: path.into(),
            size: bytes.len() as u64,
            sha256: sha256_hex(bytes),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelManifest {
    pub model_id: String,
    pub repository: String,
    pub revision: String,
    pub license: String,
    pub attribution: String,
    pub assets: Vec<ModelAsset>,
}

impl ModelManifest {
    pub fn for_test(
        model_id: impl Into<String>,
        repository: impl Into<String>,
        revision: impl Into<String>,
        assets: Vec<ModelAsset>,
    ) -> Self {
        Self {
            model_id: model_id.into(),
            repository: repository.into(),
            revision: revision.into(),
            license: "test".into(),
            attribution: "test model".into(),
            assets,
        }
    }

    pub fn validate(&self) -> Result<(), ModelError> {
        if !is_safe_segment(&self.model_id) {
            return Err(ModelError::InvalidManifest(
                "model id must be one safe path segment".into(),
            ));
        }
        if self.repository.is_empty() || self.revision.is_empty() {
            return Err(ModelError::InvalidManifest(
                "repository and revision are required".into(),
            ));
        }
        if self.assets.is_empty() {
            return Err(ModelError::InvalidManifest(
                "at least one model asset is required".into(),
            ));
        }
        let mut paths = Vec::with_capacity(self.assets.len());
        for asset in &self.assets {
            validate_asset_path(&asset.path)?;
            if asset.size == 0 {
                return Err(ModelError::InvalidManifest(format!(
                    "asset has zero size: {}",
                    asset.path
                )));
            }
            if asset.sha256.len() != 64
                || !asset.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                return Err(ModelError::InvalidManifest(format!(
                    "asset has an invalid SHA-256 checksum: {}",
                    asset.path
                )));
            }
            if paths.contains(&asset.path) {
                return Err(ModelError::InvalidManifest(format!(
                    "duplicate asset path: {}",
                    asset.path
                )));
            }
            paths.push(asset.path.clone());
        }
        Ok(())
    }

    pub fn total_size(&self) -> u64 {
        self.assets.iter().map(|asset| asset.size).sum()
    }

    pub fn asset_url(&self, asset: &ModelAsset) -> String {
        format!(
            "https://huggingface.co/{}/resolve/{}/{}?download=true",
            self.repository, self.revision, asset.path
        )
    }

    pub fn estimated_size_gib(&self) -> f64 {
        self.total_size() as f64 / (1024.0 * 1024.0 * 1024.0)
    }
}

pub fn default_model_manifest() -> ModelManifest {
    ModelManifest {
        model_id: MODEL_ID.into(),
        repository: MODEL_REPOSITORY.into(),
        revision: MODEL_REVISION.into(),
        license: MODEL_LICENSE.into(),
        attribution: MODEL_ATTRIBUTION.into(),
        assets: vec![
            ModelAsset {
                path: "config.json".into(),
                size: 2_394,
                sha256: "a9306624f5ec14270a014b647e5c316b6e03a662c369758d1b90697a7b0655b9".into(),
            },
            ModelAsset {
                path: "model.bin".into(),
                size: 3_087_284_237,
                sha256: "69f74147e3334731bc3a76048724833325d2ec74642fb52620eda87352e3d4f1".into(),
            },
            ModelAsset {
                path: "preprocessor_config.json".into(),
                size: 340,
                sha256: "7ccc62c6f2765af1f3b46c00c9b5894426835a05021c8b9c01eecb6dfb542711".into(),
            },
            ModelAsset {
                path: "tokenizer.json".into(),
                size: 2_480_617,
                sha256: "6d8cbd7cd0d8d5815e478dac67b85a26bbe77c1f5e0c6d76d1ce2abc0e5f21ca".into(),
            },
            ModelAsset {
                path: "vocabulary.json".into(),
                size: 1_068_114,
                sha256: "c69260f2ab26d659b7c398f9a2b2b48ed0df16c3b47d7326782fd9cba71690c1".into(),
            },
        ],
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelState {
    NotDownloaded,
    Downloading,
    Ready,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelStatus {
    pub model_id: String,
    pub state: ModelState,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub percent: u8,
    pub current_asset: Option<String>,
    pub install_path: Option<PathBuf>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelProgress {
    pub model_id: String,
    pub state: ModelState,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub percent: u8,
    pub current_asset: Option<String>,
}

impl ModelStatus {
    fn from_progress(progress: ModelProgress, install_path: Option<PathBuf>) -> Self {
        Self {
            model_id: progress.model_id,
            state: progress.state,
            downloaded_bytes: progress.downloaded_bytes,
            total_bytes: progress.total_bytes,
            percent: progress.percent,
            current_asset: progress.current_asset,
            install_path,
            error: None,
        }
    }
}

pub struct ModelAssetResponse {
    pub status: u16,
    pub content_length: Option<u64>,
    pub body: Box<dyn Read + Send>,
}

pub trait ModelAssetSource: Send + Sync {
    fn open(&self, url: &str, range_start: Option<u64>) -> Result<ModelAssetResponse, ModelError>;
}

#[derive(Debug, Clone, Default)]
pub struct HttpModelSource {
    client: Option<Client>,
}

impl HttpModelSource {
    pub fn new() -> Result<Self, ModelError> {
        let client = Client::builder()
            .user_agent("Meeting Notes/0.1 WhisperX model setup")
            .build()
            .map_err(|error| ModelError::Http(error.to_string()))?;
        Ok(Self {
            client: Some(client),
        })
    }

    fn client(&self) -> Result<Client, ModelError> {
        self.client
            .clone()
            .ok_or_else(|| ModelError::Http("HTTP model source was not initialized".into()))
    }
}

impl ModelAssetSource for HttpModelSource {
    fn open(&self, url: &str, range_start: Option<u64>) -> Result<ModelAssetResponse, ModelError> {
        let client = self.client()?;
        let mut request = client.get(url);
        if let Some(start) = range_start {
            request = request.header(reqwest::header::RANGE, format!("bytes={start}-"));
        }
        let response = request
            .send()
            .map_err(|error| ModelError::Http(error.to_string()))?;
        let status = response.status().as_u16();
        let content_length = response.content_length();
        Ok(ModelAssetResponse {
            status,
            content_length,
            body: Box::new(response),
        })
    }
}

#[derive(Debug, Error)]
pub enum ModelError {
    #[error("model root could not be created: {0}")]
    CreateRoot(#[source] io::Error),
    #[error("model manifest is invalid: {0}")]
    InvalidManifest(String),
    #[error("model asset path is unsafe: {0}")]
    UnsafeAssetPath(String),
    #[error("model download is already running")]
    AlreadyDownloading,
    #[error("model download was canceled")]
    Canceled,
    #[error("model download HTTP request failed: {0}")]
    Http(String),
    #[error("model download returned HTTP status {status} for {asset}")]
    HttpStatus { asset: String, status: u16 },
    #[error("model asset size mismatch for {asset}: expected {expected}, got {actual}")]
    SizeMismatch {
        asset: String,
        expected: u64,
        actual: u64,
    },
    #[error("model asset checksum mismatch for {asset}: expected {expected}, got {actual}")]
    ChecksumMismatch {
        asset: String,
        expected: String,
        actual: String,
    },
    #[error("model asset I/O failed: {0}")]
    Io(#[source] io::Error),
    #[error("model installation is incomplete: {0}")]
    Incomplete(String),
}

#[derive(Debug, Clone)]
pub struct ModelManager {
    root: PathBuf,
    manifest: ModelManifest,
}

pub fn model_is_ready(path: impl AsRef<Path>) -> bool {
    let path = path.as_ref();
    path.is_dir()
        && [
            "config.json",
            "model.bin",
            "preprocessor_config.json",
            "tokenizer.json",
            "vocabulary.json",
        ]
        .iter()
        .all(|asset| {
            let asset_path = path.join(asset);
            fs::symlink_metadata(asset_path)
                .map(|metadata| metadata.file_type().is_file())
                .unwrap_or(false)
        })
}

impl ModelManager {
    pub fn new(root: impl AsRef<Path>, manifest: ModelManifest) -> Result<Self, ModelError> {
        manifest.validate()?;
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(&root).map_err(ModelError::CreateRoot)?;
        Ok(Self { root, manifest })
    }

    pub fn manifest(&self) -> &ModelManifest {
        &self.manifest
    }

    pub fn manifest_json(&self) -> Result<String, ModelError> {
        serde_json::to_string(&self.manifest)
            .map_err(|error| ModelError::InvalidManifest(error.to_string()))
    }

    pub fn installed_path(&self) -> PathBuf {
        self.root.join(&self.manifest.model_id)
    }

    pub fn status(&self) -> ModelStatus {
        let install_path = self.installed_path();
        if self.validate_installation().is_ok() {
            return ModelStatus {
                model_id: self.manifest.model_id.clone(),
                state: ModelState::Ready,
                downloaded_bytes: self.manifest.total_size(),
                total_bytes: self.manifest.total_size(),
                percent: 100,
                current_asset: None,
                install_path: Some(install_path),
                error: None,
            };
        }

        if let Some(error) = read_error(&self.error_path()) {
            return self.status_with_error(error);
        }

        let downloaded_bytes = self.partial_downloaded_bytes();
        let total_bytes = self.manifest.total_size();
        ModelStatus {
            model_id: self.manifest.model_id.clone(),
            state: if self.lock_path().exists() {
                ModelState::Downloading
            } else {
                ModelState::NotDownloaded
            },
            downloaded_bytes,
            total_bytes,
            percent: percent(downloaded_bytes, total_bytes),
            current_asset: None,
            install_path: None,
            error: None,
        }
    }

    pub fn remove(&self) -> Result<(), ModelError> {
        if self.lock_path().exists() {
            return Err(ModelError::AlreadyDownloading);
        }
        let installed = self.installed_path();
        if installed.exists() {
            fs::remove_dir_all(installed).map_err(ModelError::Io)?;
        }
        let partial = self.partial_path();
        if partial.exists() {
            fs::remove_dir_all(partial).map_err(ModelError::Io)?;
        }
        remove_error(&self.error_path());
        Ok(())
    }

    pub fn validate_installation(&self) -> Result<(), ModelError> {
        let install_path = self.installed_path();
        for asset in &self.manifest.assets {
            let path = safe_join(&install_path, &asset.path)?;
            let metadata = fs::metadata(&path).map_err(|error| {
                if error.kind() == io::ErrorKind::NotFound {
                    ModelError::Incomplete(asset.path.clone())
                } else {
                    ModelError::Io(error)
                }
            })?;
            if !metadata.is_file() {
                return Err(ModelError::Incomplete(asset.path.clone()));
            }
            validate_file(&path, asset)?;
        }
        Ok(())
    }

    pub fn download<F>(&self, on_progress: F) -> Result<PathBuf, ModelError>
    where
        F: FnMut(ModelProgress),
    {
        let source = Arc::new(HttpModelSource::new()?);
        self.download_with_source_and_cancel(source, Arc::new(AtomicBool::new(false)), on_progress)
    }

    pub fn download_with_source<S, F>(
        &self,
        source: Arc<S>,
        on_progress: F,
    ) -> Result<PathBuf, ModelError>
    where
        S: ModelAssetSource + 'static,
        F: FnMut(ModelProgress),
    {
        self.download_with_source_and_cancel(source, Arc::new(AtomicBool::new(false)), on_progress)
    }

    pub fn download_with_source_and_cancel<S, F>(
        &self,
        source: Arc<S>,
        cancel: Arc<AtomicBool>,
        mut on_progress: F,
    ) -> Result<PathBuf, ModelError>
    where
        S: ModelAssetSource + 'static,
        F: FnMut(ModelProgress),
    {
        self.manifest.validate()?;
        if self.validate_installation().is_ok() {
            return Ok(self.installed_path());
        }

        let _lock = DownloadLock::acquire(&self.lock_path())?;
        let partial = self.partial_path();
        if partial.exists() && !partial.is_dir() {
            fs::remove_file(&partial).map_err(ModelError::Io)?;
        }
        fs::create_dir_all(&partial).map_err(ModelError::Io)?;
        let total_bytes = self.manifest.total_size();
        let mut downloaded_bytes = self.partial_downloaded_bytes();
        emit_progress(
            &mut on_progress,
            self.progress(ModelState::Downloading, downloaded_bytes, None),
        );
        remove_error(&self.error_path());

        let result = (|| {
            for asset in &self.manifest.assets {
                check_canceled(&cancel)?;
                let part_path = safe_join(&partial, &format!("{}.part", asset.path))?;
                if let Some(parent) = part_path.parent() {
                    fs::create_dir_all(parent).map_err(ModelError::Io)?;
                }
                let mut existing = fs::metadata(&part_path)
                    .map(|metadata| metadata.len())
                    .unwrap_or(0);
                if existing > asset.size {
                    fs::remove_file(&part_path).map_err(ModelError::Io)?;
                    existing = 0;
                }
                let prior_existing = existing;
                if existing == asset.size {
                    if validate_file(&part_path, asset).is_ok() {
                        downloaded_bytes = downloaded_bytes.max(self.partial_downloaded_bytes());
                        emit_progress(
                            &mut on_progress,
                            self.progress(
                                ModelState::Downloading,
                                downloaded_bytes,
                                Some(asset.path.clone()),
                            ),
                        );
                        continue;
                    }
                    fs::remove_file(&part_path).map_err(ModelError::Io)?;
                    existing = 0;
                }

                let mut response = source.open(
                    &self.manifest.asset_url(asset),
                    (prior_existing > 0).then_some(prior_existing),
                )?;
                if prior_existing > 0 && response.status != 206 {
                    drop(response.body);
                    fs::remove_file(&part_path).map_err(ModelError::Io)?;
                    existing = 0;
                    response = source.open(&self.manifest.asset_url(asset), None)?;
                }
                if response.status != 200 && response.status != 206 {
                    return Err(ModelError::HttpStatus {
                        asset: asset.path.clone(),
                        status: response.status,
                    });
                }
                if let Some(length) = response.content_length {
                    let expected_remaining = asset.size.saturating_sub(existing);
                    if length != expected_remaining {
                        return Err(ModelError::SizeMismatch {
                            asset: asset.path.clone(),
                            expected: expected_remaining,
                            actual: length,
                        });
                    }
                }

                let mut output = OpenOptions::new()
                    .create(true)
                    .write(true)
                    .append(existing > 0)
                    .truncate(existing == 0)
                    .open(&part_path)
                    .map_err(ModelError::Io)?;
                let mut body = response.body;
                let mut buffer = [0_u8; 64 * 1024];
                loop {
                    check_canceled(&cancel)?;
                    let read = body.read(&mut buffer).map_err(ModelError::Io)?;
                    if read == 0 {
                        break;
                    }
                    output.write_all(&buffer[..read]).map_err(ModelError::Io)?;
                    downloaded_bytes = downloaded_bytes.saturating_add(read as u64);
                    emit_progress(
                        &mut on_progress,
                        self.progress(
                            ModelState::Downloading,
                            downloaded_bytes,
                            Some(asset.path.clone()),
                        ),
                    );
                }
                output.flush().map_err(ModelError::Io)?;
                output.sync_all().map_err(ModelError::Io)?;
                validate_file(&part_path, asset)?;
            }

            check_canceled(&cancel)?;
            for asset in &self.manifest.assets {
                let part_path = safe_join(&partial, &format!("{}.part", asset.path))?;
                let final_path = safe_join(&partial, &asset.path)?;
                if part_path.exists() {
                    fs::rename(&part_path, &final_path).map_err(ModelError::Io)?;
                }
            }
            let installed = self.installed_path();
            if let Some(installed_parent) = installed.parent() {
                fs::create_dir_all(installed_parent).map_err(ModelError::Io)?;
            }
            if installed.exists() {
                fs::remove_dir_all(&installed).map_err(ModelError::Io)?;
            }
            fs::rename(&partial, &installed).map_err(ModelError::Io)?;
            self.validate_installation()?;
            remove_error(&self.error_path());
            emit_progress(
                &mut on_progress,
                self.progress(ModelState::Ready, total_bytes, None),
            );
            Ok(installed)
        })();

        if let Err(error) = &result {
            match error {
                ModelError::Canceled => {}
                _ => {
                    write_error(&self.error_path(), &error.to_string());
                    emit_progress(
                        &mut on_progress,
                        self.progress(ModelState::Error, self.partial_downloaded_bytes(), None),
                    );
                }
            }
        }
        result
    }

    pub fn cancel_handle() -> Arc<AtomicBool> {
        Arc::new(AtomicBool::new(false))
    }

    fn partial_path(&self) -> PathBuf {
        self.root
            .join(format!("{}.partial", self.manifest.model_id))
    }

    fn lock_path(&self) -> PathBuf {
        self.root
            .join(format!("{}.{}", self.manifest.model_id, MODEL_LOCK))
    }

    fn error_path(&self) -> PathBuf {
        self.root.join(format!("{}.error", self.manifest.model_id))
    }

    fn partial_downloaded_bytes(&self) -> u64 {
        self.manifest
            .assets
            .iter()
            .map(|asset| {
                safe_join(&self.partial_path(), &format!("{}.part", asset.path))
                    .ok()
                    .and_then(|path| fs::metadata(path).ok())
                    .map(|metadata| metadata.len().min(asset.size))
                    .unwrap_or(0)
            })
            .sum()
    }

    fn progress(
        &self,
        state: ModelState,
        downloaded_bytes: u64,
        current_asset: Option<String>,
    ) -> ModelProgress {
        ModelProgress {
            model_id: self.manifest.model_id.clone(),
            state,
            downloaded_bytes,
            total_bytes: self.manifest.total_size(),
            percent: percent(downloaded_bytes, self.manifest.total_size()),
            current_asset,
        }
    }

    fn status_with_error(&self, error: String) -> ModelStatus {
        let progress = self.progress(ModelState::Error, self.partial_downloaded_bytes(), None);
        let mut status = ModelStatus::from_progress(progress, None);
        status.error = Some(error);
        status
    }
}

struct DownloadLock {
    path: PathBuf,
}

impl DownloadLock {
    fn acquire(path: &Path) -> Result<Self, ModelError> {
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|error| {
                if error.kind() == io::ErrorKind::AlreadyExists {
                    ModelError::AlreadyDownloading
                } else {
                    ModelError::Io(error)
                }
            })?;
        Ok(Self {
            path: path.to_path_buf(),
        })
    }
}

impl Drop for DownloadLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn validate_asset_path(path: &str) -> Result<(), ModelError> {
    let candidate = Path::new(path);
    if path.is_empty() || candidate.is_absolute() {
        return Err(ModelError::UnsafeAssetPath(path.into()));
    }
    for component in candidate.components() {
        if !matches!(component, Component::Normal(_)) {
            return Err(ModelError::UnsafeAssetPath(path.into()));
        }
    }
    Ok(())
}

fn safe_join(root: &Path, relative: &str) -> Result<PathBuf, ModelError> {
    validate_asset_path(relative)?;
    Ok(root.join(relative))
}

fn is_safe_segment(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn validate_file(path: &Path, asset: &ModelAsset) -> Result<(), ModelError> {
    let metadata = fs::metadata(path).map_err(ModelError::Io)?;
    let file_type = fs::symlink_metadata(path)
        .map_err(ModelError::Io)?
        .file_type();
    if !file_type.is_file() {
        return Err(ModelError::Incomplete(asset.path.clone()));
    }
    let actual_size = metadata.len();
    if actual_size != asset.size {
        return Err(ModelError::SizeMismatch {
            asset: asset.path.clone(),
            expected: asset.size,
            actual: actual_size,
        });
    }
    let mut file = File::open(path).map_err(ModelError::Io)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 1024 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(ModelError::Io)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let actual = format_digest(hasher.finalize().as_slice());
    if actual != asset.sha256 {
        return Err(ModelError::ChecksumMismatch {
            asset: asset.path.clone(),
            expected: asset.sha256.clone(),
            actual,
        });
    }
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    format_digest(Sha256::digest(bytes).as_slice())
}

fn format_digest(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn percent(downloaded: u64, total: u64) -> u8 {
    if total == 0 {
        return 0;
    }
    ((downloaded.saturating_mul(100) / total).min(100)) as u8
}

fn check_canceled(cancel: &AtomicBool) -> Result<(), ModelError> {
    if cancel.load(Ordering::Relaxed) {
        Err(ModelError::Canceled)
    } else {
        Ok(())
    }
}

fn emit_progress<F>(callback: &mut F, progress: ModelProgress)
where
    F: FnMut(ModelProgress),
{
    callback(progress);
}

fn read_error(path: &Path) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .filter(|value| !value.is_empty())
}

fn write_error(path: &Path, message: &str) {
    let _ = fs::write(path, message);
}

fn remove_error(path: &Path) {
    let _ = fs::remove_file(path);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_manifest_has_only_safe_required_assets() {
        let manifest = default_model_manifest();
        manifest.validate().unwrap();
        assert_eq!(manifest.model_id, "large-v3");
        assert!(
            manifest.assets.iter().all(|asset| {
                validate_asset_path(&asset.path).is_ok() && asset.sha256.len() == 64
            })
        );
    }

    #[test]
    fn unsafe_asset_paths_are_rejected() {
        let manifest = ModelManifest::for_test(
            "model",
            "repo",
            "revision",
            vec![ModelAsset {
                path: "../escape.bin".into(),
                size: 1,
                sha256: "0".repeat(64),
            }],
        );
        assert!(matches!(
            manifest.validate(),
            Err(ModelError::UnsafeAssetPath(_))
        ));
    }
}
