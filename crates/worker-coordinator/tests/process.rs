use std::fs;

use tempfile::tempdir;
use whisperx_worker::{
    WorkerConfig, bundled_worker_resource_root, bundled_worker_root, copy_worker_script,
    resolve_worker_config,
};

#[test]
fn worker_config_is_rooted_in_the_supplied_resource_directory() {
    let directory = tempdir().unwrap();
    let config = resolve_worker_config(directory.path()).unwrap();

    assert_eq!(
        config.script,
        bundled_worker_resource_root(directory.path()).join("whisperx_worker.py")
    );
    assert_eq!(
        config.python,
        bundled_worker_resource_root(directory.path()).join("bin/python3")
    );
    assert!(config.script.is_absolute());
    assert!(config.python.is_absolute());
}

#[test]
fn worker_config_can_target_a_materialized_worker_environment() {
    let directory = tempdir().unwrap();
    let root = bundled_worker_root(directory.path());
    fs::create_dir_all(root.join("bin")).unwrap();
    fs::write(root.join("bin/python3"), "#!/bin/sh\n").unwrap();
    fs::write(root.join("whisperx_worker.py"), "print('worker')\n").unwrap();

    let config = WorkerConfig::from_worker_root(&root).unwrap();

    assert_eq!(config.script, root.join("whisperx_worker.py"));
    assert_eq!(config.python, root.join("bin/python3"));
    config.validate().unwrap();
}

#[test]
fn copying_a_worker_script_uses_a_fixed_destination() {
    let directory = tempdir().unwrap();
    let source = directory.path().join("source.py");
    fs::write(&source, "print('worker')\n").unwrap();
    let root = bundled_worker_root(directory.path());
    let copied = copy_worker_script(&source, &root).unwrap();

    assert_eq!(copied, root.join("whisperx_worker.py"));
    assert_eq!(fs::read_to_string(copied).unwrap(), "print('worker')\n");
}

#[test]
fn development_config_uses_a_direct_python_command_without_shell_interpolation() {
    let directory = tempdir().unwrap();
    let root = directory.path().join("python-worker");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("whisperx_worker.py"), "print('worker')\n").unwrap();

    let config = WorkerConfig::for_development(&root).unwrap();

    assert_eq!(config.python, std::path::PathBuf::from("python3"));
    assert_eq!(config.script, root.join("whisperx_worker.py"));
    config.validate().unwrap();
}
