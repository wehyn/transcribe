use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::thread;

use tauri::State;

use meeting_application::{DefaultWorkerFactory, MeetingRuntime};
use meeting_capture::{CaptureSource, MacOsCaptureSource};
use meeting_domain::{CaptureConfig, LanguageMode, SessionState};
use meeting_storage::{LocalSessionStore, SessionRecord};
use whisperx_worker::{ModelManager, ModelStatus, default_model_manifest};

#[derive(Debug, Clone, Serialize)]
pub struct CapabilityResponse {
    pub microphone_available: bool,
    pub system_audio_available: bool,
}

#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LanguageRequest {
    English,
    Filipino,
    Taglish,
}

impl From<LanguageRequest> for LanguageMode {
    fn from(value: LanguageRequest) -> Self {
        match value {
            LanguageRequest::English => Self::English,
            LanguageRequest::Filipino => Self::Filipino,
            LanguageRequest::Taglish => Self::Taglish,
        }
    }
}

#[derive(Default)]
pub struct DesktopState {
    pub(crate) runtime: Option<MeetingRuntime>,
    pub(crate) session_id: Option<String>,
    pub(crate) store_root: Option<PathBuf>,
    pub(crate) model_root: Option<PathBuf>,
    pub(crate) model_cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelDownloadResponse {
    pub started: bool,
    pub total_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelManifestResponse {
    pub model_id: String,
    pub revision: String,
    pub total_bytes: u64,
    pub license: String,
    pub attribution: String,
}

impl Default for ModelDownloadResponse {
    fn default() -> Self {
        Self {
            started: false,
            total_bytes: 0,
        }
    }
}

const MODEL_PROGRESS_EVENT: &str = "model-download-progress";
const MODEL_ERROR_EVENT: &str = "model-download-error";

fn model_manager(app: &tauri::AppHandle) -> Result<ModelManager, String> {
    let root = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?
        .join("models");
    ModelManager::new(root, default_model_manifest()).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn model_status(app: tauri::AppHandle) -> Result<ModelStatus, String> {
    Ok(model_manager(&app)?.status())
}

#[tauri::command]
pub fn model_manifest(app: tauri::AppHandle) -> Result<ModelManifestResponse, String> {
    let manager = model_manager(&app)?;
    let manifest = manager.manifest();
    Ok(ModelManifestResponse {
        model_id: manifest.model_id.clone(),
        revision: manifest.revision.clone(),
        total_bytes: manifest.total_size(),
        license: manifest.license.clone(),
        attribution: manifest.attribution.clone(),
    })
}

#[tauri::command]
pub fn download_model(
    state: State<'_, Mutex<DesktopState>>,
    app: tauri::AppHandle,
) -> Result<ModelDownloadResponse, String> {
    let manager = model_manager(&app)?;
    let total_bytes = manager.manifest().total_size();
    let cancel = ModelManager::cancel_handle();
    {
        let mut state = state.lock().map_err(|_| "desktop state lock poisoned")?;
        if state.model_cancel.is_some() {
            return Err("model download is already running".into());
        }
        state.model_root = Some(manager.installed_path());
        state.model_cancel = Some(Arc::clone(&cancel));
    }
    let thread_manager = manager.clone();
    let thread_result = thread::Builder::new()
        .name("whisperx-model-download".into())
        .spawn(move || {
            let result = thread_manager.download(|progress| {
                let _ = app.emit(MODEL_PROGRESS_EVENT, progress);
            });
            if let Ok(mut state) = state.lock() {
                state.model_cancel = None;
            }
            match result {
                Ok(path) => {
                    let _ = app.emit(
                        MODEL_PROGRESS_EVENT,
                        ModelStatus {
                            model_id: thread_manager.manifest().model_id.clone(),
                            state: whisperx_worker::ModelState::Ready,
                            downloaded_bytes: total_bytes,
                            total_bytes,
                            percent: 100,
                            current_asset: None,
                            install_path: Some(path),
                            error: None,
                        },
                    );
                }
                Err(error) => {
                    let _ = app.emit(MODEL_ERROR_EVENT, error.to_string());
                }
            }
        })
        .map_err(|error| error.to_string());
    if let Err(error) = thread_result {
        if let Ok(mut state) = state.lock() {
            state.model_cancel = None;
        }
        return Err(error);
    }
    Ok(ModelDownloadResponse {
        started: true,
        total_bytes,
    })
}

#[tauri::command]
pub fn cancel_model_download(state: State<'_, Mutex<DesktopState>>) -> Result<(), String> {
    let state = state.lock().map_err(|_| "desktop state lock poisoned")?;
    if let Some(cancel) = &state.model_cancel {
        cancel.store(true, Ordering::Relaxed);
        Ok(())
    } else {
        Err("model download is not running".into())
    }
}

#[tauri::command]
pub fn remove_model(app: tauri::AppHandle) -> Result<ModelStatus, String> {
    let manager = model_manager(&app)?;
    manager.remove().map_err(|error| error.to_string())?;
    Ok(manager.status())
}

fn session_or_error(state: &mut DesktopState) -> Result<&mut MeetingRuntime, String> {
    state.runtime.as_mut().ok_or("session not created".into())
}

#[tauri::command]
pub fn capabilities() -> CapabilityResponse {
    let source = MacOsCaptureSource::new();
    let capabilities = source.capabilities();
    CapabilityResponse {
        microphone_available: capabilities.microphone_available,
        system_audio_available: capabilities.system_audio_available,
    }
}

#[tauri::command]
pub fn create_session(
    state: State<'_, Mutex<DesktopState>>,
    app: tauri::AppHandle,
    title: Option<String>,
    language: Option<LanguageRequest>,
) -> Result<(), String> {
    let mut state = state.lock().map_err(|_| "desktop state lock poisoned")?;
    let language = language.unwrap_or(LanguageRequest::English);
    let _title = title.unwrap_or_else(|| "Untitled meeting".to_owned());
    let model_path = model_manager(&app).ok().and_then(|model| {
        model
            .validate_installation()
            .ok()
            .map(|_| model.installed_path())
    });
    let worker_factory = match model_path {
        Some(path) => DefaultWorkerFactory::default().with_model_path(path),
        None => DefaultWorkerFactory::default(),
    };
    let runtime = MeetingRuntime::with_worker_factory(
        CaptureConfig::dual_source(language.into()),
        Box::new(MacOsCaptureSource::new()),
        worker_factory,
    );
    state.session_id = Some(runtime.session_id().to_owned());
    state.runtime = Some(runtime);
    Ok(())
}

#[tauri::command]
pub fn accept_consent(state: State<'_, Mutex<DesktopState>>) -> Result<(), String> {
    let mut state = state.lock().map_err(|_| "desktop state lock poisoned")?;
    session_or_error(&mut state)?.accept_consent();
    Ok(())
}

#[tauri::command]
pub fn record(state: State<'_, Mutex<DesktopState>>, app: tauri::AppHandle) -> Result<(), String> {
    let mut state = state.lock().map_err(|_| "desktop state lock poisoned")?;
    let model_path = model_manager(&app)?.installed_path();
    if !whisperx_worker::model_is_ready(&model_path) {
        return Err("download the WhisperX model before recording".into());
    }
    let runtime = session_or_error(&mut state)?;
    let session_id = runtime.session_id().to_owned();
    let recording_path = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?
        .join("recordings")
        .join(&session_id);
    let store_root = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?
        .join("sessions");
    let store = LocalSessionStore::new(&store_root).map_err(|error| error.to_string())?;
    let mut record = SessionRecord::new(&session_id);
    record.state = SessionState::Starting;
    store.save(&record).map_err(|error| error.to_string())?;
    if let Err(error) = runtime.record(&recording_path) {
        if let Ok(store) = LocalSessionStore::new(&store_root) {
            let _ = store.delete(&session_id);
        }
        return Err(error.to_string());
    }
    let mut record = store.load(&session_id).map_err(|error| error.to_string())?;
    record.state = SessionState::Listening;
    record.add_recording_artifact(recording_path);
    store.save(&record).map_err(|error| error.to_string())?;
    state.store_root = Some(store_root);
    Ok(())
}

#[tauri::command]
pub fn pause(state: State<'_, Mutex<DesktopState>>) -> Result<(), String> {
    let mut state = state.lock().map_err(|_| "desktop state lock poisoned")?;
    session_or_error(&mut state)?
        .pause()
        .map_err(|error| error.to_string())?;
    update_session_record(&state, SessionState::Paused, None)
}

#[tauri::command]
pub fn resume(state: State<'_, Mutex<DesktopState>>) -> Result<(), String> {
    let mut state = state.lock().map_err(|_| "desktop state lock poisoned")?;
    session_or_error(&mut state)?
        .resume()
        .map_err(|error| error.to_string())?;
    update_session_record(&state, SessionState::Listening, None)
}

#[tauri::command]
pub fn stop(state: State<'_, Mutex<DesktopState>>) -> Result<String, String> {
    let mut state = state.lock().map_err(|_| "desktop state lock poisoned")?;
    let path = session_or_error(&mut state)?
        .stop()
        .map(|path| path.to_string_lossy().into_owned())
        .map_err(|error| error.to_string())?;
    update_session_record(&state, SessionState::Sealed, Some(PathBuf::from(&path)))?;
    Ok(path)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    Markdown,
    Json,
}

#[tauri::command]
pub fn export_meeting(
    state: State<'_, Mutex<DesktopState>>,
    app: tauri::AppHandle,
    destination: PathBuf,
    format: ExportFormat,
) -> Result<String, String> {
    let state = state.lock().map_err(|_| "desktop state lock poisoned")?;
    let runtime = state.runtime.as_ref().ok_or("session not created")?;
    let transcript = runtime
        .final_transcript()
        .ok_or("final transcript is not ready")?;
    let notes =
        whisperx_worker::generate_final_notes(transcript).map_err(|error| error.to_string())?;
    let content = match format {
        ExportFormat::Markdown => whisperx_worker::notes_markdown(&notes, Some(transcript)),
        ExportFormat::Json => {
            whisperx_worker::notes_json(&notes).map_err(|error| error.to_string())?
        }
    };
    let destination = if destination.is_absolute() {
        destination
    } else {
        app.path()
            .app_data_dir()
            .map_err(|error| error.to_string())?
            .join("exports")
            .join(destination)
    };
    fs::create_dir_all(&destination).map_err(|error| error.to_string())?;
    let filename = match format {
        ExportFormat::Markdown => "notes.md",
        ExportFormat::Json => "notes.json",
    };
    let path = destination.join(filename);
    fs::write(&path, content).map_err(|error| error.to_string())?;
    Ok(path.to_string_lossy().into_owned())
}

#[tauri::command]
pub fn delete_meeting(state: State<'_, Mutex<DesktopState>>) -> Result<(), String> {
    let mut state = state.lock().map_err(|_| "desktop state lock poisoned")?;
    let runtime = state.runtime.take().ok_or("session not created")?;
    if matches!(
        runtime.state(),
        SessionState::Listening | SessionState::Paused
    ) {
        state.runtime = Some(runtime);
        return Err("stop the recording before deleting the meeting".into());
    }
    if let Some(path) = runtime.recording_path() {
        if path.exists() {
            fs::remove_dir_all(path).map_err(|error| error.to_string())?;
        }
    }
    if let (Some(root), Some(id)) = (&state.store_root, &state.session_id) {
        let store = LocalSessionStore::new(root).map_err(|error| error.to_string())?;
        if store.load(id).is_ok() {
            store.delete(id).map_err(|error| error.to_string())?;
        }
    }
    state.session_id = None;
    state.store_root = None;
    state.runtime = None;
    Ok(())
}

#[tauri::command]
pub fn shutdown(state: State<'_, Mutex<DesktopState>>) -> Result<(), String> {
    let mut state = state.lock().map_err(|_| "desktop state lock poisoned")?;
    if let Some(runtime) = state.runtime.as_mut() {
        if matches!(
            runtime.state(),
            SessionState::Listening | SessionState::Paused
        ) {
            let _ = runtime.stop();
        }
    }
    state.runtime = None;
    state.session_id = None;
    state.store_root = None;
    Ok(())
}

fn update_session_record(
    state: &DesktopState,
    lifecycle_state: SessionState,
    recording_root: Option<PathBuf>,
) -> Result<(), String> {
    let (Some(root), Some(id)) = (&state.store_root, &state.session_id) else {
        return Ok(());
    };
    let store = LocalSessionStore::new(root).map_err(|error| error.to_string())?;
    let mut record = store.load(id).map_err(|error| error.to_string())?;
    record.state = lifecycle_state;
    if let Some(path) = recording_root {
        record.add_recording_artifact(path);
    }
    store.save(&record).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn session_state(state: State<'_, Mutex<DesktopState>>) -> Result<SessionState, String> {
    let state = state.lock().map_err(|_| "desktop state lock poisoned")?;
    Ok(state
        .runtime
        .as_ref()
        .map(MeetingRuntime::state)
        .unwrap_or(SessionState::Created))
}

#[cfg(test)]
mod tests {
    use super::LanguageRequest;
    use meeting_domain::LanguageMode;

    #[test]
    fn language_requests_map_to_supported_worker_modes() {
        assert_eq!(
            LanguageMode::from(LanguageRequest::English),
            LanguageMode::English
        );
        assert_eq!(
            LanguageMode::from(LanguageRequest::Filipino),
            LanguageMode::Filipino
        );
        assert_eq!(
            LanguageMode::from(LanguageRequest::Taglish),
            LanguageMode::Taglish
        );
    }
}
