use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::{CoordinatorError, JsonLinesWorker, WorkerConfig, WorkerConfigError};

pub struct WorkerProcess {
    worker: Option<JsonLinesWorker>,
}

impl WorkerProcess {
    pub fn start(config: &WorkerConfig) -> Result<Self, CoordinatorError> {
        config.validate().map_err(|error| {
            CoordinatorError::Spawn(std::io::Error::new(std::io::ErrorKind::InvalidInput, error))
        })?;
        let mut command = Command::new(&config.python);
        command
            .arg(&config.script)
            .arg("--model")
            .arg(&config.model)
            .arg("--device")
            .arg(&config.device)
            .arg("--compute-type")
            .arg(&config.compute_type)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        let mut child = command.spawn().map_err(CoordinatorError::Spawn)?;
        let stdin = child
            .stdin
            .take()
            .ok_or(CoordinatorError::StdinUnavailable)?;
        let stdout = child
            .stdout
            .take()
            .ok_or(CoordinatorError::StdoutUnavailable)?;
        let mut worker = JsonLinesWorker::from_parts(child, stdin, stdout);
        worker.handshake()?;
        Ok(Self {
            worker: Some(worker),
        })
    }

    pub fn worker(&mut self) -> Result<&mut JsonLinesWorker, CoordinatorError> {
        self.worker.as_mut().ok_or(CoordinatorError::WorkerStopped)
    }

    pub fn into_worker(mut self) -> Result<JsonLinesWorker, CoordinatorError> {
        self.worker.take().ok_or(CoordinatorError::WorkerStopped)
    }

    pub fn stop(&mut self) {
        if let Some(mut worker) = self.worker.take() {
            let _ = worker.send_shutdown();
        }
    }
}

impl Drop for WorkerProcess {
    fn drop(&mut self) {
        self.stop();
    }
}

pub fn resolve_worker_config(
    resource_dir: impl AsRef<Path>,
) -> Result<WorkerConfig, WorkerConfigError> {
    WorkerConfig::resolve_worker_config(resource_dir)
}

pub fn bundled_worker_resource_root(resource_dir: impl AsRef<Path>) -> PathBuf {
    resource_dir.as_ref().join("worker")
}

pub fn bundled_worker_root(app_data_dir: impl AsRef<Path>) -> PathBuf {
    app_data_dir.as_ref().join("whisperx-worker")
}

pub fn copy_worker_script(
    source: impl AsRef<Path>,
    destination_root: impl AsRef<Path>,
) -> std::io::Result<PathBuf> {
    let root = destination_root.as_ref();
    fs::create_dir_all(root)?;
    let destination = root.join("whisperx_worker.py");
    fs::copy(source, &destination)?;
    Ok(destination)
}
