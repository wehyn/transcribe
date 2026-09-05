# WhisperX worker

This directory contains the only Python component in the application. Rust launches `whisperx_worker.py` on demand and exchanges versioned JSON-lines messages over stdin/stdout.

## Environment

`pyproject.toml` pins the worker to Python 3.11–3.13, WhisperX 3.7.4, and the CPU PyTorch 2.8.0 combination. `requirements-cpu.txt` is provided for pip-based installs. Install it in a dedicated environment; do not install these dependencies into the Rust/Tauri process.

## Protocol

The worker responds to:

- `hello` — protocol handshake
- `capabilities` — supported models/languages
- `transcribe_window` — low-latency provisional result
- `transcribe_recording` — authoritative transcription, alignment, and segment output
- `finalize` — session lifecycle acknowledgment
- `shutdown` — clean exit

The ML imports are lazy. Handshake, capabilities, empty windows, and shutdown do not load WhisperX or NumPy. The selected language is validated as `english`, `filipino`, or `taglish`; Taglish uses automatic language detection and falls back to segment-level timing where word alignment is unavailable.

## Checks

Run the protocol-only checks without model downloads:

```bash
python test_worker_protocol.py
python smoke_test.py
```

A real final-pass smoke test requires the pinned environment and a valid audio file. The Rust wrapper is:

```bash
cargo run -p whisperx-worker --bin whisperx-live-smoke -- \
  --audio tests/fixtures/short_meeting.wav \
  --window-seconds 4 \
  --overlap-seconds 1
```
