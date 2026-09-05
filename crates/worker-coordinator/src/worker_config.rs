use serde::{Deserialize, Serialize};
use std::env;
use std::io;
use std::path::{Component, Path, PathBuf};
use thiserror::Error;

const DEFAULT_MODEL: &str = "large-v3";
const DEFAULT_DEVICE: &str = "cpu";
const DEFAULT_COMPUTE_TYPE: &str = "int8";
const WORKER_SCRIPT: &str = "whisperx_worker.py";
const WORKER_PYTHON: &str = "bin/python3";

/// Paths and inference settings for the on-demand worker.
///
/// The process launcher passes these values as argv entries. It never evaluates
/// them through a shell. Production bundles use the fixed files below a worker
/// resource root; development-only environment overrides live in
/// [`WorkerConfig::from_environment`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerConfig {
    pub python: PathBuf,
    pub script: PathBuf,
    pub model: String,
    pub device: String,
    pub compute_type: String,
}

impl WorkerConfig {
    /// Resolve the fixed layout prepared under a bundled worker root.
    ///
    /// The root may be a Tauri resource directory or an application-data
    /// directory populated by a provisioning step. No environment variable can
    /// replace the executable or script in this production layout.
    pub fn from_worker_root(root: impl AsRef<Path>) -> Result<Self, WorkerConfigError> {
        let root = absolute_path(root.as_ref())?;
        Ok(Self::with_paths(
            root.join(WORKER_PYTHON),
            root.join(WORKER_SCRIPT),
        ))
    }

    /// Build the source-tree configuration used by local development/smoke tests.
    pub fn for_development(root: impl AsRef<Path>) -> Result<Self, WorkerConfigError> {
        let root = absolute_path(root.as_ref())?;
        Ok(Self::with_paths(
            PathBuf::from("python3"),
            root.join(WORKER_SCRIPT),
        ))
    }

    /// Resolve a packaged worker from a Tauri resource directory.
    ///
    /// `WHISPERX_WORKER_ROOT` is an explicit opt-in for a provisioned worker
    /// root. Relative overrides remain relative to `resource_dir`; the worker
    /// executable and script names are still fixed by [`from_worker_root`].
    pub fn resolve_worker_config(
        resource_dir: impl AsRef<Path>,
    ) -> Result<Self, WorkerConfigError> {
        let resource_dir = absolute_path(resource_dir.as_ref())?;
        let worker_root = env::var_os("WHISPERX_WORKER_ROOT")
            .map(|value| resolve_relative_to(&resource_dir, PathBuf::from(value)))
            .unwrap_or_else(|| bundled_worker_resource_root(&resource_dir));
        Self::from_worker_root(worker_root)
    }

    /// Preserve the source-tree environment interface for local tooling.
    ///
    /// This method is intentionally separate from [`resolve_worker_config`].
    /// `PYTHON` and `WHISPERX_WORKER_SCRIPT` are useful for a developer's
    /// virtual environment, but are not trusted as packaged-app defaults.
    pub fn from_environment(root: impl AsRef<Path>) -> Self {
        let root = absolute_path(root.as_ref()).unwrap_or_else(|_| root.as_ref().to_path_buf());
        let python = env::var_os("PYTHON")
            .map(|value| resolve_python_override(PathBuf::from(value), &root))
            .unwrap_or_else(|| PathBuf::from("python3"));
        let script = env::var_os("WHISPERX_WORKER_SCRIPT")
            .map(|value| resolve_relative_to(&root, PathBuf::from(value)))
            .unwrap_or_else(|| {
                root.join("crates/worker-coordinator/python")
                    .join(WORKER_SCRIPT)
            });
        Self::with_paths(python, script)
    }

    /// Validate paths immediately before spawning the worker.
    pub fn validate(&self) -> Result<(), WorkerConfigError> {
        if !self.script.is_absolute() {
            return Err(WorkerConfigError::RelativeScript(self.script.clone()));
        }
        if !self.script.is_file() {
            return Err(WorkerConfigError::MissingScript(self.script.clone()));
        }

        if self.python.is_absolute() {
            if !self.python.is_file() {
                return Err(WorkerConfigError::MissingPython(self.python.clone()));
            }
        } else if !is_bare_command(&self.python) {
            return Err(WorkerConfigError::UnsafePythonCommand(self.python.clone()));
        }
        Ok(())
    }

    pub fn with_model_path(mut self, model_path: impl AsRef<Path>) -> Self {
        self.model = model_path.as_ref().to_string_lossy().into_owned();
        self
    }

    fn with_paths(python: PathBuf, script: PathBuf) -> Self {
        Self {
            python,
            script,
            model: env::var("WHISPERX_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.into()),
            device: env::var("WHISPERX_DEVICE").unwrap_or_else(|_| DEFAULT_DEVICE.into()),
            compute_type: env::var("WHISPERX_COMPUTE_TYPE")
                .unwrap_or_else(|_| DEFAULT_COMPUTE_TYPE.into()),
        }
    }
}

/// The root used by Tauri's `$RESOURCES/worker` bundle layout.
pub fn bundled_worker_resource_root(resource_dir: impl AsRef<Path>) -> PathBuf {
    resource_dir.as_ref().join("worker")
}

fn absolute_path(path: &Path) -> Result<PathBuf, WorkerConfigError> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    env::current_dir()
        .map(|cwd| cwd.join(path))
        .map_err(WorkerConfigError::ResolveRoot)
}

fn resolve_relative_to(base: &Path, path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        path
    } else {
        base.join(path)
    }
}

fn resolve_python_override(path: PathBuf, base: &Path) -> PathBuf {
    if path.is_absolute() || is_bare_command(&path) {
        path
    } else {
        base.join(path)
    }
}

fn is_bare_command(path: &Path) -> bool {
    let mut components = path.components();
    matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none()
}

#[derive(Debug, Error)]
pub enum WorkerConfigError {
    #[error("could not resolve worker root: {0}")]
    ResolveRoot(#[source] io::Error),
    #[error("worker script path must be absolute: {0}")]
    RelativeScript(PathBuf),
    #[error("worker script does not exist or is not a file: {0}")]
    MissingScript(PathBuf),
    #[error("worker Python executable does not exist or is not a file: {0}")]
    MissingPython(PathBuf),
    #[error("worker Python must be an absolute path or a bare executable name: {0}")]
    UnsafePythonCommand(PathBuf),
}

#[derive(Debug, Error)]
pub enum WorkerLaunchError {
    #[error("worker process could not start: {0}")]
    Spawn(#[source] std::io::Error),
    #[error("worker stdin is unavailable")]
    StdinUnavailable,
    #[error("worker stdout is unavailable")]
    StdoutUnavailable,
}
