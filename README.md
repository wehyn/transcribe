# Meeting Notes

A macOS-first, local-first meeting recorder and note taker built around a Rust/Tauri runtime, separate microphone and system-audio tracks, and an isolated WhisperX worker.

## Privacy invariant

> The application only listens when the user presses **Record**.

Before Record the runtime does not open audio devices, create streams, allocate live audio buffers, persist audio, start VAD, launch WhisperX, or generate notes. Capability checks and meeting/session creation inspect metadata only. Pause, Stop, close, capture errors, permission revocation, and worker failures close capture without automatic restart.

## Workspace

- `crates/domain` — lifecycle, capture configuration, note schema, and citation validation.
- `crates/capture` — record-gated capture source abstraction, test source, and macOS CPAL/ScreenCaptureKit adapter boundary.
- `crates/storage` — retained microphone/system/mixed PCM tracks, valid PCM WAV materialization, session metadata, recovery, export, and verified deletion.
- `crates/application` — lifecycle orchestration, recording, rolling live pipeline, and finalization.
- `crates/worker-coordinator` — versioned JSON-lines transport, bounded rolling windows, reconciliation, supervised worker process, notes, and exports.
- `crates/worker-coordinator/python` — isolated, pinned WhisperX worker with lazy ML imports.
- `apps/desktop/src-tauri` — Tauri commands, close/shutdown handling, resources, entitlements, and macOS bundle metadata.
- `apps/desktop/ui` — thin React/Vite setup, live Draft, review, export, and deletion interface.

## Development checks

```bash
cargo fmt --all
cargo test --workspace --exclude meeting-desktop
cargo clippy --workspace --exclude meeting-desktop --all-targets -- -D warnings
python3 crates/worker-coordinator/python/test_worker_protocol.py
python3 crates/worker-coordinator/python/test_worker_transformations.py
python3 crates/worker-coordinator/python/test_worker_final.py
python3 -m py_compile crates/worker-coordinator/python/whisperx_worker.py
cd apps/desktop/ui && npm install && npm run build
```

The portable Rust and UI checks pass on Linux. The Tauri crate itself requires Linux GTK/WebKit development packages to compile on Linux; macOS packaging and native capture must be built on macOS.

## WhisperX environment

The Python worker is on-demand and communicates over protocol version `1`. Install the pinned CPU environment from `crates/worker-coordinator/python/requirements-cpu.txt`; no model is downloaded by the Rust build. Configure `WHISPERX_PYTHON`, `WHISPERX_WORKER_SCRIPT`, `WHISPERX_MODEL`, `WHISPERX_DEVICE`, and `WHISPERX_COMPUTE_TYPE` for a local or bundled worker environment.

## macOS validation still required

On a macOS 13+ build host, validate microphone and Screen Recording TCC prompts, real CPAL sample callbacks, ScreenCaptureKit system-audio callbacks, permission revocation, pause/resume/stop, close while recording, bundle resources, signing, hardened runtime, and notarization. Linux cannot verify those behaviors.
