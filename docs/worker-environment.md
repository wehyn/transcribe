# WhisperX worker environment

Meeting Notes keeps WhisperX outside the Rust/Tauri process. The desktop process
starts one local JSON-lines worker only after the explicit **Record** transition
and stops it when the session ends. No audio, worker, or model process is
started by opening the app, creating a meeting, or checking capabilities.

## Bundle layout

The Tauri configuration is structurally ready to copy the following files into
`$RESOURCES/worker/` when bundling is enabled:

```text
worker/
├── whisperx_worker.py
├── requirements-cpu.txt
├── pyproject.toml
└── worker-environment.md
```

A production worker environment must be provisioned separately under the same
fixed layout, including an executable at `worker/bin/python3`. The app resolves
that layout from the Tauri resource directory; it does not accept a shell
command or interpolate a path. The `WHISPERX_WORKER_ROOT` override is an
explicit path to a provisioned worker root for local diagnostics. `PYTHON` and
`WHISPERX_WORKER_SCRIPT` are development-only overrides used by source-tree
smoke tooling, not packaged-app defaults.

The packaged environment is expected to use the pinned CPU-compatible versions
in `requirements-cpu.txt` and `pyproject.toml`:

- Python 3.11 through 3.13
- WhisperX 3.7.4
- NumPy 2.2.6
- PyTorch 2.8.0+cpu
- TorchAudio 2.8.0+cpu

Install these dependencies in a dedicated virtual environment. Do not install
WhisperX, Torch, or NumPy into the Rust/Tauri runtime. Model files are not
committed or bundled. The desktop setup screen downloads
the pinned model manifest into the app-data `models/` directory, resumes
partial assets, validates size and SHA-256, and atomically promotes the
completed directory. Recording remains disabled until that installation is
ready; model setup never starts the audio capture or WhisperX worker.

## Local setup

For source-tree checks, use a virtual environment and run:

```bash
python3 -m venv .venv
. .venv/bin/activate
python -m pip install -r crates/worker-coordinator/python/requirements-cpu.txt
python crates/worker-coordinator/python/test_worker_protocol.py
python crates/worker-coordinator/python/test_worker_transformations.py
```

The Rust process launcher passes the worker script, model, device, and compute
settings as separate arguments. It never invokes a shell. `WHISPERX_MODEL`,
`WHISPERX_DEVICE`, and `WHISPERX_COMPUTE_TYPE` may select inference settings
for local diagnostics; the defaults are `large-v3`, `cpu`, and `int8`.

## Packaging status

`bundle.active` is enabled for macOS packaging. This Linux checkout cannot produce a
signed or notarized macOS artifact. The resource map, macOS Info.plist, and
entitlements file are configuration for a macOS build host. A signed build
must provision the Python environment, verify the exact worker paths, choose
its Apple signing identity, and test TCC permissions on a supported macOS
installation before distribution.

The capture adapter uses the `CMSampleBuffer::get_audio_buffer_list()` API from
ScreenCaptureKit 0.3.6 and validates that each returned buffer contains complete
little-endian `f32` samples. Validate the actual sample format, channel layout,
permissions, and callback delivery on macOS before claiming end-to-end
system-audio recording.
