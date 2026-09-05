use std::collections::HashMap;
use std::io::Cursor;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use tempfile::tempdir;
use whisperx_worker::{
    ModelAsset, ModelAssetResponse, ModelAssetSource, ModelManager, ModelManifest, ModelState,
};

struct FakeSource {
    assets: HashMap<String, Vec<u8>>,
    ranges: Mutex<Vec<(String, Option<u64>)>>,
}

impl ModelAssetSource for FakeSource {
    fn open(
        &self,
        url: &str,
        range_start: Option<u64>,
    ) -> Result<ModelAssetResponse, whisperx_worker::ModelError> {
        let name = url.rsplit('/').next().unwrap().split('?').next().unwrap();
        self.ranges
            .lock()
            .unwrap()
            .push((name.to_owned(), range_start));
        let bytes = self.assets.get(name).unwrap().clone();
        let start = range_start.unwrap_or(0) as usize;
        if start > 0 {
            Ok(ModelAssetResponse {
                status: 206,
                content_length: Some((bytes.len() - start) as u64),
                body: Box::new(Cursor::new(bytes[start..].to_vec())),
            })
        } else {
            Ok(ModelAssetResponse {
                status: 200,
                content_length: Some(bytes.len() as u64),
                body: Box::new(Cursor::new(bytes)),
            })
        }
    }
}

fn test_manifest(assets: &[(&str, &[u8])]) -> ModelManifest {
    ModelManifest::for_test(
        "test-model",
        "test/repository",
        "test-revision",
        assets
            .iter()
            .map(|(path, bytes)| ModelAsset::for_test(*path, bytes))
            .collect(),
    )
}

#[test]
fn downloads_and_atomically_installs_a_checksum_validated_model() {
    let assets = [("config.json", br"{}".as_slice()), ("model.bin", b"model")];
    let source = Arc::new(FakeSource {
        assets: assets
            .iter()
            .map(|(name, bytes)| ((*name).to_owned(), bytes.to_vec()))
            .collect(),
        ranges: Mutex::new(Vec::new()),
    });
    let root = tempdir().unwrap();
    let manager = ModelManager::new(root.path().join("models"), test_manifest(&assets)).unwrap();

    let installed = manager.download_with_source(source, |_| {}).unwrap();

    assert_eq!(manager.status().state, ModelState::Ready);
    assert!(installed.join("config.json").is_file());
    assert!(installed.join("model.bin").is_file());
    assert!(!installed.to_string_lossy().contains("partial"));
    assert!(manager.validate_installation().is_ok());
}

#[test]
fn checksum_failure_leaves_the_install_unavailable_and_records_an_error() {
    let assets = [("config.json", br"{}".as_slice())];
    let source = Arc::new(FakeSource {
        assets: [("config.json".to_owned(), b"wrong".to_vec())]
            .into_iter()
            .collect(),
        ranges: Mutex::new(Vec::new()),
    });
    let root = tempdir().unwrap();
    let manager = ModelManager::new(root.path().join("models"), test_manifest(&assets)).unwrap();

    let error = manager.download_with_source(source, |_| {}).unwrap_err();

    assert!(matches!(
        error,
        whisperx_worker::ModelError::SizeMismatch { .. }
            | whisperx_worker::ModelError::ChecksumMismatch { .. }
    ));
    assert_eq!(manager.status().state, ModelState::Error);
    assert!(!manager.installed_path().exists());
}

#[test]
fn cancellation_is_reported_without_promoting_a_partial_install() {
    let assets = [("config.json", br"{}".as_slice())];
    let source = Arc::new(FakeSource {
        assets: assets
            .iter()
            .map(|(name, bytes)| ((*name).to_owned(), bytes.to_vec()))
            .collect(),
        ranges: Mutex::new(Vec::new()),
    });
    let root = tempdir().unwrap();
    let manager = ModelManager::new(root.path().join("models"), test_manifest(&assets)).unwrap();
    let cancel = ModelManager::cancel_handle();
    cancel.store(true, Ordering::Relaxed);

    let error = manager
        .download_with_source_and_cancel(source, cancel, |_| {})
        .unwrap_err();

    assert!(matches!(error, whisperx_worker::ModelError::Canceled));
    assert!(!manager.installed_path().exists());
}

#[test]
fn a_partial_asset_is_resumed_with_a_range_request() {
    let assets = [("config.json", br"{}".as_slice())];
    let source = Arc::new(FakeSource {
        assets: assets
            .iter()
            .map(|(name, bytes)| ((*name).to_owned(), bytes.to_vec()))
            .collect(),
        ranges: Mutex::new(Vec::new()),
    });
    let root = tempdir().unwrap();
    let manager = ModelManager::new(root.path().join("models"), test_manifest(&assets)).unwrap();
    let partial = root.path().join("models/test-model.partial");
    std::fs::create_dir_all(&partial).unwrap();
    std::fs::write(partial.join("config.json.part"), b"{").unwrap();

    manager
        .download_with_source(Arc::clone(&source), |_| {})
        .unwrap();

    assert_eq!(source.ranges.lock().unwrap()[0].1, Some(1));
    assert_eq!(manager.status().state, ModelState::Ready);
}

#[test]
fn an_invalid_full_size_partial_asset_restarts_from_zero() {
    let assets = [("config.json", br"{}".as_slice())];
    let source = Arc::new(FakeSource {
        assets: assets
            .iter()
            .map(|(name, bytes)| ((*name).to_owned(), bytes.to_vec()))
            .collect(),
        ranges: Mutex::new(Vec::new()),
    });
    let root = tempdir().unwrap();
    let manager = ModelManager::new(root.path().join("models"), test_manifest(&assets)).unwrap();
    let partial = root.path().join("models/test-model.partial");
    std::fs::create_dir_all(&partial).unwrap();
    std::fs::write(partial.join("config.json.part"), b"xx").unwrap();

    manager
        .download_with_source(Arc::clone(&source), |_| {})
        .unwrap();

    assert_eq!(source.ranges.lock().unwrap()[0].1, None);
    assert_eq!(manager.status().state, ModelState::Ready);
}

#[test]
fn a_model_lock_prevents_concurrent_downloads() {
    let assets = [("config.json", br"{}".as_slice())];
    let root = tempdir().unwrap();
    let manager = ModelManager::new(root.path().join("models"), test_manifest(&assets)).unwrap();
    let lock = root.path().join("models/test-model.download.lock");
    std::fs::write(lock, b"").unwrap();
    let source = Arc::new(FakeSource {
        assets: [("config.json".to_owned(), b"{}".to_vec())]
            .into_iter()
            .collect(),
        ranges: Mutex::new(Vec::new()),
    });

    let error = manager.download_with_source(source, |_| {}).unwrap_err();

    assert!(matches!(
        error,
        whisperx_worker::ModelError::AlreadyDownloading
    ));
}

#[cfg(unix)]
#[test]
fn symlinked_model_assets_are_not_considered_ready() {
    let assets = [("config.json", br"{}".as_slice())];
    let root = tempdir().unwrap();
    let manager = ModelManager::new(root.path().join("models"), test_manifest(&assets)).unwrap();
    let installed = manager.installed_path();
    std::fs::create_dir_all(&installed).unwrap();
    std::os::unix::fs::symlink("/etc/hosts", installed.join("config.json")).unwrap();

    assert_eq!(manager.status().state, ModelState::NotDownloaded);
    assert!(manager.validate_installation().is_err());
}

#[cfg(unix)]
#[test]
fn symlinked_model_directory_is_not_followed_during_download() {
    let assets = [("config.json", br"{}".as_slice())];
    let root = tempdir().unwrap();
    let manager = ModelManager::new(root.path().join("models"), test_manifest(&assets)).unwrap();
    let outside = tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), manager.installed_path()).unwrap();
    let source = Arc::new(FakeSource {
        assets: [("config.json".to_owned(), b"{}".to_vec())]
            .into_iter()
            .collect(),
        ranges: Mutex::new(Vec::new()),
    });

    let result = manager.download_with_source(source, |_| {});

    assert!(result.is_err());
}

#[test]
fn model_is_ready_requires_regular_required_assets() {
    let assets = [
        ("config.json", br"{}".as_slice()),
        ("model.bin", b"model".as_slice()),
        ("preprocessor_config.json", b"preprocessor".as_slice()),
        ("tokenizer.json", b"tokenizer".as_slice()),
        ("vocabulary.json", b"vocabulary".as_slice()),
    ];
    let root = tempdir().unwrap();
    let manager = ModelManager::new(root.path().join("models"), test_manifest(&assets)).unwrap();
    let installed = manager.installed_path();
    std::fs::create_dir_all(&installed).unwrap();
    for (path, bytes) in assets {
        std::fs::write(installed.join(path), bytes).unwrap();
    }

    assert!(whisperx_worker::model_is_ready(&installed));

    std::fs::remove_file(installed.join("config.json")).unwrap();
    std::fs::create_dir(installed.join("config.json")).unwrap();
    assert!(!whisperx_worker::model_is_ready(&installed));
}

#[test]
fn completed_downloads_emit_one_ready_terminal_progress_event() {
    let assets = [("config.json", br"{}".as_slice())];
    let root = tempdir().unwrap();
    let manager = ModelManager::new(root.path().join("models"), test_manifest(&assets)).unwrap();
    let source = Arc::new(FakeSource {
        assets: [("config.json".to_owned(), b"{}".to_vec())]
            .into_iter()
            .collect(),
        ranges: Mutex::new(Vec::new()),
    });
    let mut progress = Vec::new();

    manager
        .download_with_source(source, |event| progress.push(event))
        .unwrap();

    assert_eq!(
        progress
            .iter()
            .filter(|event| event.state == ModelState::Ready)
            .count(),
        1
    );
}
