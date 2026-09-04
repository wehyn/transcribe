# AI Meeting Assistant and Note Taker Implementation Plan

> **For Hermes:** Use subagent-driven-development skill to implement this plan task-by-task.

**Goal:** Build a privacy-conscious meeting assistant that records microphone/system audio, transcribes meetings with WhisperX, produces word-level timestamps and speaker labels, and turns the transcript into editable, searchable notes with summaries and action items.

**Architecture:** Build the application itself as a lightweight local-first Rust desktop app. Rust owns the desktop shell, native audio capture, session state, durable recording, local database, live event delivery, transcript reconciliation, API, and job orchestration. Use Tauri 2 only as a thin desktop shell for the React/TypeScript presentation layer, keeping the UI bundle separate from the application runtime. WhisperX is the only non-Rust runtime exception and remains isolated in a separately pinned Python inference worker because WhisperX and its ML dependencies are Python-based. The Rust app communicates with that worker through a narrow local protocol and keeps the worker lifecycle, model loading, and resource usage controlled. Do not build a Python API/backend, Electron app, or browser-first product. The live path is **open app -> consent -> capture -> incremental WhisperX-backed transcript -> stop -> final WhisperX pass -> notes**.

**Tech Stack:** Rust stable with a Cargo workspace, Tokio, Axum only for the local in-process API/event layer, SQLx with SQLite, Serde, tracing, and small focused crates for domain/storage/capture/orchestration. Tauri 2 is a thin desktop shell; React/TypeScript is limited to the presentation layer and bundled assets, not the application backend. Native audio capture is implemented behind Rust traits and platform adapters. WhisperX is the only separate runtime: a separately pinned Python 3.10–3.12 inference worker using WhisperX, faster-whisper, forced alignment, and optional pyannote diarization. Store recordings/artifacts on the local filesystem and keep session/job state in SQLite. Use Cargo tests/clippy/fmt plus focused frontend/worker contract tests. Optimize for a small idle footprint, low startup overhead, bounded memory, and no always-running server/database/queue services in the local desktop MVP.

## Product scope and decisions

### MVP acceptance criteria

- User can create a meeting, select microphone/system audio, acknowledge consent, and see a clear **Record** button; no audio device is opened and no audio is processed before the user explicitly presses **Record**.
- Pressing **Record** is the sole action that starts capture/listening, creates the active session, and launches the on-demand WhisperX worker. Before that action, the app may show device capability/status only and must not read, buffer, transmit, or persist audio.
- During recording, captured audio is continuously written to a durable local recording while completed speech windows are sent to the WhisperX worker and incremental transcript updates appear in the UI.
- Pressing **Pause** stops audio capture and inference immediately; pressing **Resume** explicitly restarts them. Pressing **Stop** closes capture, seals the recording, stops inference, and begins finalization.
- Closing the app, revoking permission, or encountering a capture error must stop listening and terminate/clean up the active capture path; the app must not restart capture automatically.
- The live transcript shows provisional/final status, timestamps, and speaker labels when diarization is available; it remains usable when speaker labels are pending or unavailable.
- The user can recover from temporary worker/network failures without losing already-captured audio and see actionable health/latency state.
- When stopped, the system runs a final full-recording WhisperX pass, replacing provisional segments with the best validated transcript and marking any alignment/diarization limitations.
- Notes include a concise summary, decisions, action items, open questions, and cited transcript time ranges after the final transcript is ready; optionally show a clearly labeled live notes draft during the meeting.
- User can review/edit speaker names and transcript text without overwriting the raw recording or raw WhisperX artifacts.
- User can export transcript/notes as Markdown and JSON; audio export/download is protected by the same authorization as the meeting.
- Consent, recording status, retention, deletion, live latency, provisional/final state, and model limitations are visible to the user.
- Automated tests cover pre-record silence, explicit Record gating, pause/resume/stop, capture cleanup, streaming backpressure and reconnects, Rust/Python contracts, normalization, finalization, failure/retry behavior, and the principal UI flow; a real smoke test exercises the gated live chunk path and final WhisperX pass where model/GPU assets are available.

### Recommended scope boundaries

- The MVP is a live listening application, not an upload-first workflow: the primary path is **open app -> configure source -> acknowledge consent -> press Record -> see incremental transcript -> Pause/Resume or Stop -> finalize**. Audio upload is a secondary recovery/import feature only.
- **Explicit recording gate:** opening the app, creating a meeting, selecting a device, checking permissions/capabilities, or viewing the dashboard must never activate the microphone, system audio, VAD, recorder, inference worker, or audio buffers. Only the user's deliberate press of **Record** may transition the session into capture.
- Do not promise zero-latency or native streaming WhisperX. WhisperX's documented API is batch-oriented, so implement low-latency rolling-window inference: capture PCM frames only after Record, use VAD/utterance boundaries, submit short committed windows, display provisional text, and reconcile it with the final full-recording pass.
- Use a desktop-first Tauri capture path for reliable microphone and system audio. Keep browser `MediaRecorder` as a fallback/demo path, since browser system-audio capture varies by platform and usually requires an explicit share selection.
- Keep the first release single-user and local. Add accounts, team workspaces, calendar integrations, meeting bots, and multi-device sync after the live pipeline is reliable.
- Keep live diarization optional or explicitly experimental in the first cut. Speaker labels may be assigned after enough context is available and can be corrected during finalization.

### Open decisions to resolve before implementation

1. Target desktop platforms and capture scope: microphone only, or microphone plus system audio; define the first supported OS.
2. Local-only privacy requirement vs hosted multi-user deployment.
3. Target hardware and latency budget; select the rolling-window model, chunk/overlap duration, VAD, queue depth, and CPU/GPU fallback after measuring on representative meetings.
4. LLM provider for live/final notes: local model, OpenAI-compatible endpoint, or hosted API; define whether transcript content may leave the machine.
5. Supported languages and whether live alignment/diarization are enabled by default. Final alignment requires language-specific models; diarization requires pyannote model access.
6. Retention policy, including whether the raw recording remains after finalization and how pause intervals are handled.
7. Live transcript policy: how provisional text is visually distinguished, how much rollback/reconciliation is allowed, and whether live notes are MVP or post-meeting only.

## Technical design

### MVP implementation principles

- **Lightweight runtime:** one Rust/Tauri desktop process for the product, with SQLite/local files; no Electron, Python API, remote backend, or always-on server/database/queue services in the MVP.
- **Explicit Record gate:** opening the app, creating a meeting, selecting a device, checking permissions/capabilities, or viewing the dashboard must never activate the microphone, system audio, VAD, recorder, inference worker, or audio buffers. Only a deliberate user press of **Record** may transition the session into capture.
- **On-demand ML:** one separately pinned Python WhisperX worker, spawned only after Record and terminated after Stop/finalization or an idle timeout. It is an inference subprocess, not the app architecture.
- **Rust ownership:** Rust owns native capture, session state, persistence, event delivery, transcript reconciliation, finalization, notes orchestration, exports, retention, and security. The React/TypeScript bundle is presentation only.
- **No implicit restart:** after Pause, Stop, app close, permission revocation, capture failure, or worker crash, the app must not reopen an audio device, resume VAD, restart the worker, or continue listening without another explicit **Record** press.

### Rust ownership and inference boundary

Rust is the product runtime and owns the application lifecycle, domain logic, native capture, persistence, artifact storage, local HTTP/SSE/WebSocket events, job/session coordination, exports, retention, logging, and desktop integration. Keep the local MVP as one lightweight Rust/Tauri process plus a WhisperX worker spawned only when a meeting starts; do not require a permanently running API server, Redis, PostgreSQL, or queue service. The React/TypeScript layer is presentation code bundled by Tauri, not a backend. WhisperX remains the required ML engine, but it runs in a separately pinned Python process because the official WhisperX runtime and its faster-whisper, alignment, and pyannote dependencies are Python-based. Keep that boundary narrow and versioned: Rust sends a JSON job/window manifest and receives validated JSON results and structured errors. Do not spread Python/WhisperX imports into Rust domain, API, or storage crates.

### Live processing pipeline

1. **Capture:** Tauri/Rust capture adapters are constructed and opened only inside the explicit `Record` command handler. Before that command, the app may enumerate capability metadata if needed but must not obtain an audio stream or receive frames. On Record, acquire microphone audio and, where supported, system/loopback audio; convert frames to a known PCM format, timestamp them with a monotonic clock, and fan them out to both the durable recorder and live inference buffer.
2. **Durable recording:** Rust appends each accepted frame only after Record succeeds to a crash-safe temporary recording, periodically flushes it, and atomically seals it on Stop. Capture failures must not silently produce a partial meeting; surface dropped-frame counts and recoverable errors.
3. **Voice activity and utterance windows:** A Rust VAD component starts only after Record and stops on Pause/Stop/error. It detects speech boundaries, maintains a bounded overlap, and emits committed utterance windows rather than arbitrary fixed slices.
4. **Live WhisperX transcription:** The Rust coordinator starts the WhisperX worker only after Record, keeps the model warm for the active session, and sends committed windows to it. The worker returns `provisional` or `committed` results with a session sequence number.
5. **Incremental reconciliation:** Rust deduplicates results by session/window sequence, tracks acknowledged audio offsets, and replaces the provisional tail when a newer overlapping window arrives. The UI labels provisional text and tolerates corrections. Persist live segments separately from final transcript versions.
6. **Live events:** Axum exposes a local WebSocket or SSE stream for transcript deltas, capture health, latency, queue depth, and worker errors. The event stream may be connected before Record for UI status, but it must never carry audio or cause capture; reconnects replay events from a session cursor.
7. **Optional live speakers:** Do not block live transcription on diarization. If enabled, send sufficiently long committed windows to the WhisperX diarization stage and update speaker labels asynchronously; final speaker assignment occurs during finalization.
8. **Finalization:** On Stop, close capture first, seal the recording, terminate or hand off the worker according to the session lifecycle, and run one full-recording WhisperX pass with alignment and optional diarization. Rust validates and stores the final version, reconciles/removes provisional segments, and only then generates final cited notes.
9. **No implicit restart:** If the app closes, permission is revoked, capture fails, or the worker crashes, Rust transitions the session to a stopped/recoverable/error state and does not reopen devices or restart listening without another explicit Record action.

### Core entities

- `Meeting`: id, owner_id (nullable for local MVP), title, status, started_at, ended_at, language, created_at, deleted_at.
- `LiveSession`: id, meeting_id, status, capture_config_json, audio_format_json, started_at, paused_at, stopped_at, last_audio_offset_ms, last_event_sequence, dropped_frames, created_at.
- `Recording`: id, meeting_id, live_session_id, storage_key, original_filename, mime_type, byte_size, duration_seconds, sha256, capture_source, created_at, sealed_at.
- `AudioWindow`: id, live_session_id, sequence, start_offset_ms, end_offset_ms, storage_key, status, submitted_at, completed_at, retry_count.
- `ProcessingJob`: id, meeting_id, recording_id, live_session_id, type, status, attempts, progress, error_code, error_message, worker_version, started_at, finished_at.
- `TranscriptVersion`: id, meeting_id, recording_id, status, provisional, language, raw_artifact_key, config_json, model_metadata_json, created_at.
- `TranscriptSegment`: id, transcript_version_id, live_session_id, window_sequence, ordinal, start_seconds, end_seconds, text, segment_status, speaker_id, confidence, edited_text.
- `TranscriptWord`: id, segment_id, ordinal, start_seconds, end_seconds, word, confidence, speaker_id.
- `Speaker`: id, transcript_version_id, stable_label, display_name, color.
- `NoteVersion`: id, meeting_id, transcript_version_id, status, summary, decisions_json, action_items_json, open_questions_json, model_metadata_json, created_at, edited_at.
- `NoteCitation`: id, note_version_id, section, item_index, segment_start, segment_end, quote.
- `LiveEvent`: id, live_session_id, sequence, event_type, payload_json, created_at.

Use immutable recording and processing versions and separate editable fields/overlays. Live windows/events are session-scoped and deduplicated by `(live_session_id, sequence)`. Deletion must cascade or tombstone all associated audio, raw artifacts, normalized records, notes, live events, and job logs according to the retention policy.

### API shape

- `POST /api/meetings` — create a meeting and capture configuration.
- `GET /api/meetings` and `GET /api/meetings/:id` — list/detail with live/final status and available artifacts.
- `POST /api/meetings/:id/live-sessions` — create a live capture session after consent/config validation; this creates no audio stream.
- `POST /api/live-sessions/:id/start` — reserved for the explicit **Record** action; only this command opens capture, starts buffering, and launches WhisperX.
- `POST /api/live-sessions/:id/pause`, `POST /api/live-sessions/:id/resume`, `POST /api/live-sessions/:id/stop` — control the session; Pause closes/halts capture and inference, Resume explicitly reopens them, and Stop seals audio and starts finalization.
- `GET /api/live-sessions/:id/capabilities` — return device/capture capability metadata only; must not open or sample an audio device.
- `WS /api/live-sessions/:id/events` or `GET /api/live-sessions/:id/events` via SSE — stream transcript deltas, capture health, latency, queue depth, and worker errors; support a `Last-Event-ID`/cursor for replay. Connecting to this stream must never start capture.
- `POST /api/live-sessions/:id/audio-windows` — internal/local capture-window submission endpoint after Record; enforce sequence and offset idempotency.
- `GET /api/jobs/:id` — finalization or recovery job status/progress/error.
- `GET /api/meetings/:id/transcript?version=live|final` — live provisional or final normalized transcript with pagination/search parameters.
- `PATCH /api/transcript-segments/:id` — edit final text/speaker display name overlay; retain audit metadata.
- `GET /api/meetings/:id/notes` and `PATCH /api/meetings/:id/notes` — retrieve/edit generated notes.
- `POST /api/meetings/:id/notes/regenerate` — enqueue bounded regeneration from a selected final transcript version.
- `GET /api/meetings/:id/export?format=markdown|json` — export editable transcript and notes.
- `DELETE /api/meetings/:id` — request deletion and verify storage cleanup.

Every endpoint must authorize against the meeting owner/workspace, validate IDs and payloads, enforce session sequencing, and avoid returning storage credentials or secrets.

### Frontend screens

1. **Dashboard:** meeting list, search/filter, live/processing/ready status, duration, last updated, and prominent `New meeting`/`Record` CTA. Loading this screen opens no audio devices.
2. **New meeting:** title, microphone/system audio source selection, consent acknowledgement, language/model/diarization options, and a capability check that reads metadata only. The primary action is a clearly labeled **Record** button; upload is secondary recovery/import.
3. **Pre-record state:** show selected source and permissions/capabilities without an active stream, timer, VAD, audio meter, recorder, or WhisperX worker.
4. **Live meeting view:** after **Record** is pressed, show a prominent recording indicator, elapsed timer, pause/resume/stop, current audio source/level, live transcript with provisional styling, latency/connection state, speaker labels when available, and recovery messaging.
5. **Finalization view:** stage-level progress (seal, transcribe, align, diarize, reconcile, notes), live transcript retained while final processing runs, retry action, and actionable failure text.
6. **Meeting workspace:** final notes/transcript/audio player in a responsive split view; clicking a citation seeks playback; transcript search and speaker filter; edit controls; export/delete.
7. **Settings:** capture permissions, model/latency defaults, storage/retention, privacy/provider status, hardware diagnostics, and Hugging Face token setup instructions without displaying the token.

Accessibility requirements: keyboard-accessible **Record**, **Pause**, **Resume**, and **Stop** controls, visible focus, status announcements for recording/pause/stop/reconnect/finalization, captions/transcript readable without color alone, a non-color recording indicator, and confirmation before destructive deletion.

### WhisperX operational constraints

- Pin a tested WhisperX/PyTorch/Python combination in the worker image; do not use an unbounded dependency install in production.
- Prefer the current WhisperX API documented in its repository at implementation time. WhisperX provides batched transcription, forced alignment, and pyannote-based diarization, but its primary API is batch-oriented rather than a native streaming API. The live design must therefore use warm-worker rolling windows for responsiveness and a final full-recording WhisperX pass for authoritative output.
- **Live latency path:** keep the WhisperX model loaded, use a benchmarked small/fast model and `device=cuda`/`compute_type=float16` on GPU or `device=cpu`/`compute_type=int8` on CPU, bound window size/overlap/concurrency, and expose measured end-to-end latency. Do not claim real-time performance until the smoke test measures it on target hardware.
- **Final accuracy path:** after stop, run the complete recording through WhisperX alignment and optional diarization. Final results supersede provisional live segments; preserve both versions for debugging and reconciliation.
- GPU path: use a CUDA image, bounded batch size, model cache volume, and memory cleanup between final transcription/alignment/diarization stages.
- Diarization requires user-provided Hugging Face access and acceptance of the relevant pyannote model terms. Handle missing token/model access as a configuration error for that optional stage, not as a reason to lose the live transcript.
- WhisperX limitations remain visible: overlapping speech is difficult, diarization is imperfect, and words outside an alignment model's dictionary may lack word timings.
- Job/session execution must be idempotent, resumable by stage where practical, cancellable before starting a new heavy stage, and protected against duplicate live-window processing.
- Treat all transcript text and generated notes as untrusted content in the UI; escape rendered Markdown/HTML and never execute model-produced markup.

---

## Step-by-step implementation plan

Each implementation task follows strict TDD: write one focused failing test, run it to observe the expected failure, implement the smallest behavior, rerun the focused test, then run the relevant suite. Commit each logical workstream separately; do not combine unrelated features.

### Task 1: Establish the lightweight Rust workspace and on-demand worker environment

**Objective:** Create a Rust-first desktop workspace and thin UI shell, with an isolated WhisperX worker that is launched only for meetings.

**Files:**
- Create: `Cargo.toml` (workspace manifest)
- Create: `crates/domain/`, `crates/application/`, `crates/storage/`, `crates/capture/`, `crates/api/`, `crates/worker-coordinator/`, `apps/desktop/src-tauri/`
- Create: `apps/web/`, `services/whisperx-worker/`, `packages/contracts/`, `tests/fixtures/`
- Create: `README.md`, `LICENSE`, `.gitignore`, `rust-toolchain.toml`, `.env.example`
- Create: `docker-compose.gpu.yml`, `Makefile` or `justfile`

**Steps:**
1. Initialize the Cargo workspace and make each Rust crate compile with a minimal public health/version function.
2. Add the small React/TypeScript UI shell and Tauri 2 configuration; keep all application logic in Rust.
3. Add the Python worker directory with a locked Python 3.10–3.12 environment and protocol-only contract; do not add a Python API/backend.
4. Pin Rust, Node, Python, and the WhisperX/PyTorch/CUDA worker image strategy; document the on-demand worker lifecycle and model cache.
5. Add `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test --workspace`, frontend checks, and worker contract checks.
6. Verify the Rust app compiles, the Tauri shell launches, the worker contract passes without loading models, and no worker/server/database/queue process starts before an explicit meeting start.
7. Commit: `chore: scaffold lightweight rust meeting assistant workspace`.

### Task 2: Define Rust domain types, schema, and lifecycle states

**Objective:** Make meetings, recordings, jobs, transcript versions, and note versions representable with explicit status transitions in Rust.

**Files:**
- Create: `crates/domain/src/meeting.rs`, `recording.rs`, `processing.rs`, `transcript.rs`, `notes.rs`
- Create: `crates/domain/src/status.rs`, `lib.rs`
- Create: `crates/domain/tests/status.rs`
- Create: `crates/storage/migrations/001_initial_schema.sql`

**Steps:**
1. Test allowed transitions such as `created -> recording -> processing -> ready` and terminal `failed/deleted` behavior with Rust integration tests.
2. Test that IDs, timestamps, durations, and immutable artifact references are validated and that Serde JSON round trips are stable.
3. Implement domain types, error enums, and SQLx migrations with indexes on owner, status, meeting, and timestamps.
4. Run `cargo test -p domain` and apply migrations against a disposable SQLite database.
5. Commit: `feat: define rust meeting processing domain`.

### Task 3: Implement Rust live capture, durable recording, and explicit Record gating

**Objective:** Capture the meeting only after the user presses **Record**, continuously persist audio, and expose a recoverable session state machine.

**Files:**
- Create: `crates/capture/src/lib.rs`, `source.rs`, `pcm.rs`, `vad.rs`
- Create: `apps/desktop/src-tauri/src/capture.rs`, `commands.rs`
- Create: `crates/application/src/live_sessions.rs`, `recordings.rs`
- Create: `crates/storage/src/artifact_store.rs`, `local.rs`, `hash.rs`
- Create: `crates/capture/tests/session.rs`, `crates/application/tests/live_sessions.rs`

**Steps:**
1. Write tests first for the hard privacy gate: creating a meeting, selecting a source, capability checks, opening the pre-record screen, and subscribing to events must produce zero capture calls, frames, audio buffers, recordings, worker starts, or persisted audio.
2. Test that only an explicit Record command can transition (`created -> starting -> listening`), and that consent is required; test Pause/Resume/Stop, invalid transitions, and recovery after temporary device errors.
3. Test PCM format conversion, monotonic offsets, dropped-frame accounting, bounded buffers, pause behavior, and cleanup after close/permission revocation/errors.
4. Implement a `CaptureSource` trait with adapters constructed/opened only inside the Record handler; capability inspection must be metadata-only and must not open/sample an audio device.
5. Implement a crash-safe append-only recording writer that starts only after capture succeeds, periodically flushes/checkpoints, and atomically seals on Stop.
6. Implement Rust/Tauri commands for metadata-only capability check, explicit Record/start, Pause, Resume, Stop, audio-level health only while recording, and session recovery without automatic restart.
7. Run capture/application tests and a manual short microphone capture; verify no device/worker activity before pressing Record; commit: `feat: add rust live capture sessions with explicit record gate`.

### Task 4: Add the Rust live-window/VAD pipeline and backpressure control

**Objective:** Convert incoming audio frames into bounded speech windows ready for low-latency WhisperX inference while keeping capture lightweight and lossless.

**Files:**
- Create: `crates/capture/src/windowing.rs`, `backpressure.rs`
- Create: `crates/application/src/live_windows.rs`
- Create: `crates/capture/tests/windowing.rs`
- Create: `crates/application/tests/live_windows.rs`

**Steps:**
1. Test that the VAD/windowing pipeline remains completely idle before Record and after Pause/Stop/error; no frames may enter its buffers before an active capture session exists.
2. Test speech start/end detection, silence trimming, overlap handling, sequence numbering, pause gaps, and bounded queue behavior after Record.
3. Implement VAD/windowing with small bounded buffers and configurable frame size, minimum speech duration, maximum window duration, overlap, and queue capacity. Prefer a lightweight Rust VAD implementation or isolate VAD independently from the Python WhisperX worker.
4. Persist accepted window metadata and audio artifacts before submission; use `(session_id, sequence)` as the idempotency key.
5. Drop or defer inference windows under load according to an explicit policy and surface latency, queue depth, and dropped-window status to the UI; never silently lose audio from the durable recording.
6. Run deterministic PCM fixture tests and a real microphone windowing smoke test; commit: `feat: add lightweight live audio windows and backpressure`.

### Task 5: Add the Rust live event stream and session APIs

**Objective:** Let the UI control sessions and receive replayable live transcript/health events without allowing event connections to start capture.

**Files:**
- Create: `packages/contracts/openapi.yaml`
- Create: `crates/api/src/lib.rs`, `router.rs`, `state.rs`, `handlers/meetings.rs`, `handlers/live_sessions.rs`, `events.rs`
- Create: `crates/api/tests/live_sessions.rs`, `events.rs`
- Create: `crates/storage/src/event_repository.rs`

**Steps:**
1. Write Axum tests for meeting/session creation, consent validation, metadata-only capabilities, start/pause/resume/stop, event ordering, cursor replay, reconnects, duplicate commands, and the invariant that no endpoint except explicit Record/start can invoke capture.
2. Implement the local WebSocket or SSE stream, persist a bounded event cursor, and publish transcript deltas, capture health, live latency, queue depth, and worker errors. Connecting to the stream must never start capture.
3. Implement stable error codes, request limits, session ownership checks, and idempotent command handling; reject start requests that are not tied to the explicit Record action/state transition.
4. Verify a test client can connect before Record without receiving audio or triggering capture, then receive each post-Record event once from its cursor after disconnect/reconnect; commit: `feat: add rust live session API and events`.

### Task 6: Implement the isolated WhisperX streaming-window worker and Rust coordinator

**Objective:** Run mandatory WhisperX on committed live windows with a warm model process, but launch it only after the user presses **Record**.

**Files:**
- Create: `services/whisperx-worker/pyproject.toml` or locked requirements file
- Create: `services/whisperx-worker/src/worker.py`, `protocol.py`, `whisperx_pipeline.py`
- Create: `crates/worker-coordinator/src/protocol.rs`, `whisperx.rs`, `config.rs`, `live.rs`
- Create: `services/whisperx-worker/tests/test_protocol.py`, `test_streaming_pipeline.py`
- Create: `crates/worker-coordinator/tests/whisperx.rs`, `live.rs`
- Modify: worker Dockerfiles and GPU profile

**Steps:**
1. Write Python protocol tests and Rust coordinator tests first for pre-Record non-start behavior, explicit worker startup, long-lived worker health, protocol versioning, manifest validation, window sequence/offsets, device/compute/batch configuration, timeouts, malformed output, and error mapping; run them red.
2. Implement a long-lived JSON-lines or HTTP worker contract: Rust sends window PCM/artifact reference, absolute offsets, overlap, model settings, and sequence; Python returns provisional/committed text, relative timestamps, model metadata, and structured errors.
3. Keep WhisperX imports inside the Python worker; load the configured small/fast model once per process after Record starts and call WhisperX transcription for each bounded window. This is a rolling-window compatibility layer around WhisperX's batch API, not a claim that WhisperX natively streams.
4. Implement Rust process supervision, warm-worker health, bounded concurrency, ordering/deduplication, timeout/restart behavior, cancellation, and artifact verification. A crash/restart must not automatically reopen capture or resume listening.
5. Add a real live smoke command that proves no worker starts before Record, then captures/reads short sequential windows only after Record, loads WhisperX once, processes windows through the Rust coordinator, and reports measured first-result and steady-state latency.
6. Pin dependencies and record exact tested versions/device in the lockfiles and README.
7. Run Rust/Python contract tests and the gated live smoke test on the available worker profile; commit: `feat: stream live windows through whisperx after record`.
### Task 7: Add live transcript reconciliation and final WhisperX processing

**Objective:** Show responsive provisional transcript text during capture and produce an authoritative aligned/diarized final version after stop.

**Files:**
- Create: `crates/application/src/live_transcript.rs`, `finalization.rs`
- Create: `crates/worker-coordinator/src/alignment.rs`, `diarization.rs`
- Create: `crates/domain/tests/transcript.rs`, `finalization.rs`
- Create: `crates/application/tests/live_transcript.rs`, `finalization.rs`
- Create: `crates/storage/migrations/002_live_transcript_tables.sql`, `003_final_transcript_tables.sql`
- Modify: `services/whisperx-worker/src/protocol.py`, `whisperx_pipeline.py`

**Steps:**
1. Test Rust reconciliation of overlapping windows, provisional-tail replacement, duplicate/out-of-order results, absolute timestamp conversion, gaps, and final-version replacement.
2. Implement live segment persistence/events separately from immutable final transcript versions; label provisional vs committed text and retain source window references.
3. On stop, seal the audio and run the full recording through WhisperX transcription, `whisperx.load_align_model`/`whisperx.align`, and optional diarization/`whisperx.assign_word_speakers`.
4. Normalize and validate the final output in Rust, preserve raw JSON, mark partial alignment/diarization limitations, and generate notes only from the final version.
5. Run deterministic reconciliation tests and a real finalization smoke test; commit: `feat: finalize live meetings with whisperx alignment`.

### Task 8: Add optional live speaker diarization and Rust speaker editing

**Objective:** Add speaker labels without blocking the live transcript and allow user-friendly names without corrupting raw results.

**Files:**
- Modify: `services/whisperx-worker/src/whisperx_pipeline.py`, `protocol.py`
- Create: `crates/worker-coordinator/src/diarization.rs`
- Create: `crates/api/src/handlers/speakers.rs`
- Create: `crates/worker-coordinator/tests/diarization.rs`
- Create: `crates/api/tests/speakers.rs`
- Modify: `.env.example`, settings, and worker deployment docs

**Steps:**
1. Test missing token, denied model access, min/max speaker validation, delayed speaker updates, speaker assignment, and redacted logging.
2. Implement optional window/final diarization in Python with a runtime-injected token, followed by `whisperx.assign_word_speakers` for final output.
3. Implement Rust speaker repositories and API handlers; keep display names in a separate editable overlay.
4. Ensure missing/failed diarization leaves the live and final transcript usable with a visible status.
5. Run unit/contract tests and a token-enabled fixture smoke test if credentials are configured; commit: `feat: add optional live speaker diarization`.

### Task 9: Implement the live meeting UI and final transcript workspace

**Objective:** Let a user start a live meeting, monitor incremental transcript updates, control capture, and review the finalized transcript.

**Files:**
- Create: `apps/web/src/routes/home.tsx`, `routes/new-meeting.tsx`, `routes/live-meeting.tsx`, `routes/meeting.tsx`
- Create: `apps/web/src/components/Recorder.tsx`, `LiveTranscript.tsx`, `MeetingStatus.tsx`, `ProcessingProgress.tsx`, `Transcript.tsx`, `AudioPlayer.tsx`, `SpeakerEditor.tsx`
- Create: `apps/web/tests/live-meeting.spec.ts`, `meeting-workspace.spec.ts`
- Modify: `apps/desktop/src-tauri/src/commands.rs` and capture adapters if desktop capture is selected

**Steps:**
1. Test live UI state transitions for permission denied, unsupported capture, start/pause/resume/stop, event reconnect, stale cursor, worker latency, and processing failure.
2. Implement the live meeting screen with explicit consent, recording indicator, elapsed timer, source/level status, provisional transcript styling, live latency/connection state, and keyboard-accessible controls.
3. Subscribe to Axum WebSocket/SSE events with cursor replay; apply sequence-aware transcript reconciliation without duplicate lines.
4. Render finalization progress while retaining the live transcript; switch to the authoritative final transcript and notes when ready.
5. Add final transcript search/editing, speaker naming, playback seeking, and safe rendering.
6. Use Playwright to verify the main flow: create -> consent -> start -> receive transcript deltas -> pause/resume -> stop -> finalization -> final transcript.
7. Commit: `feat: add live meeting and transcript workspace`.

### Task 10: Generate structured AI notes from the finalized live transcript

**Objective:** Convert the final WhisperX transcript into validated, editable notes that link claims to source time ranges.

**Files:**
- Create: `crates/application/src/notes.rs`, `llm.rs`
- Create: `crates/domain/src/note_schema.rs`
- Create: `crates/worker-coordinator/src/notes.rs`
- Create: `crates/application/tests/notes.rs`
- Create: `crates/api/src/handlers/notes.rs`
- Create: `crates/api/tests/notes.rs`

**Steps:**
1. Test that note generation is triggered only for a finalized transcript, not provisional live segments; also test prompt length limits, structured JSON validation, malformed output repair/retry, empty transcript handling, and citation range validation in Rust.
2. Implement an `LlmProvider` trait with one configured OpenAI-compatible/local provider and a deterministic fake for tests; require the model to separate observed facts from inferred suggestions.
3. Store notes as a version tied to the final transcript and keep user edits separate from generated output. If live note drafts are included, label them as provisional and never export them as final notes.
4. Add regenerate endpoint with idempotency and bounded retries; do not log full transcript/prompt by default.
5. Run Rust tests with the fake provider and an opt-in live-provider smoke test; commit: `feat: generate cited notes from live meetings`.

### Task 11: Build recording and processing UI flows with Rust commands/API

**Objective:** Create a live meeting and ensure audio/transcription begins only from an explicit **Record** action.

**Files:**
- Create: `apps/web/src/routes/home.tsx`, `routes/new-meeting.tsx`, `routes/live-meeting.tsx`
- Create: `apps/web/src/components/Recorder.tsx`, `LiveTranscript.tsx`, `MeetingStatus.tsx`, `ProcessingProgress.tsx`
- Create: `apps/web/tests/recording-flow.spec.ts`, `live-meeting.spec.ts`
- Modify: `apps/desktop/src-tauri/src/commands.rs` and capture adapters if desktop capture is selected

**Steps:**
1. Test that the dashboard, new-meeting screen, source selection, capability check, event connection, and pre-record screen never request audio permission, open an audio device, create frames, start VAD, launch WhisperX, or persist audio.
2. Test that clicking **Record** is the sole path to start capture/processing; test consent, explicit **Pause**, **Resume**, and **Stop**, plus event reconnect, stale cursor, worker latency, and processing failure.
3. Implement the pre-record screen with a clearly labeled **Record** button, metadata-only capability state, consent acknowledgement, and no active audio meter/timer/buffer.
4. Implement the Tauri/Rust capture path for desktop microphone/system audio where supported; keep browser MediaRecorder as a fallback/demo path and retain explicit consent acknowledgement.
5. Render live transcript deltas, provisional styling, capture health, queue depth, latency, reconnect state, and finalization progress from Axum WebSocket/SSE events.
6. Ensure already-captured audio remains recoverable if the worker fails; expose retry/restart actions without restarting capture unless the user presses **Record** again.
7. Run Playwright with mocked capture/events and a manual desktop test with a real short live meeting; commit: `feat: add rust live meeting flow with explicit record gate`.

### Task 12: Add Rust exports, deletion, retention, and privacy controls

**Objective:** Make user data portable and deletable while preventing accidental leakage.

**Files:**
- Create: `crates/application/src/export.rs`, `retention.rs`
- Create: `crates/api/src/handlers/export.rs`, `retention.rs`
- Create: `crates/application/tests/export.rs`, `retention.rs`
- Modify: frontend settings/detail pages and privacy copy

**Steps:**
1. Test Markdown/JSON exports, escaping, stable ordering, unauthorized access, and deletion cleanup across database and storage.
2. Implement exports from edited views with original/version metadata; implement explicit deletion confirmation and retention worker in Rust.
3. Add provider/privacy disclosure, diarization token guidance, consent copy, and recording indicator.
4. Run end-to-end deletion then verify the meeting, storage keys, transcript artifacts, and notes are gone; commit: `feat: add rust privacy export and retention controls`.

### Task 13: Harden the lightweight Rust desktop deployment and WhisperX worker

**Objective:** Keep the Rust desktop app small and safe while making the on-demand WhisperX worker reproducible.

**Files:**
- Modify: Rust/Tauri config, worker Dockerfiles, `docker-compose.gpu.yml`, `.env.example`
- Create: `deploy/healthcheck.sh`, `docs/operations.md`, `docs/privacy.md`
- Create: `crates/api/tests/security/authorization.rs`

**Steps:**
1. Test authorization boundaries, capture permissions, resource limits, subprocess path validation, worker lifecycle, and secret redaction.
2. Keep the desktop binary free of ML/Python dependencies; start the WhisperX process only when a meeting starts, reuse its warm model during that session, terminate it after an idle timeout/session end, and never run Redis/PostgreSQL/queue services for the local MVP.
3. Keep the default Rust/Tauri install lightweight: compile release binaries with size optimization, avoid unnecessary plugins, lazy-load the UI's heavier views, and bound CPU/RAM/VRAM/window/concurrency settings. Add non-root worker containers, restricted local IPC/network access, isolated model/artifact volumes, and secure headers for any local HTTP surface.
4. Document HF model terms, consent/legal review, backup/restore, model cache warm-up, GPU diagnostics, CPU fallback, Rust/Python protocol compatibility, worker startup cost, and data retention.
5. Verify Tauri app size/startup baseline, `docker compose -f docker-compose.gpu.yml config`, worker spawn/stop behavior, migration startup, CPU smoke path, GPU path if hardware is present, and no secrets in logs.
6. Commit: `chore: harden lightweight rust desktop deployment`.

### Task 14: Run the complete live Rust/WhisperX acceptance suite and document limitations

**Objective:** Prove the implemented product meets the live-meeting MVP acceptance criteria, especially the explicit Record privacy gate, and report environment-dependent gaps honestly.

**Files:**
- Modify: `README.md`, `docs/operations.md`, `docs/limitations.md`
- Create: `tests/e2e/live-meeting.spec.ts`

**Steps:**
1. Run `cargo fmt --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, `cargo test --workspace`, frontend typecheck/lint/unit tests, Python protocol tests, and Playwright tests.
2. Run a real gated live smoke test through the Rust coordinator: prove no audio device/frames/VAD/worker activity before Record; press Record, capture sequential windows, keep WhisperX warm, measure first-result and steady-state latency, verify event sequencing/reconnect; press Stop, then run the final full-recording WhisperX alignment pass.
3. Verify the full primary flow: create -> configure without listening -> consent -> press Record -> live transcript deltas -> Pause/Resume -> reconnect -> Stop -> seal -> final WhisperX transcript -> notes -> edit -> export -> delete.
4. Check that worker failure does not lose durable audio, recovery does not duplicate segments, finalization supersedes provisional text, a capture error does not auto-restart listening, accessibility works, and hostile Markdown/HTML is safely rendered.
5. Document unsupported capture platforms, rolling-window latency and accuracy tradeoffs, alignment/diarization limitations, model requirements, protocol version, and skipped checks with reasons.
6. Commit: `test: verify explicit-record live rust whisperx flow`.

---

## Validation commands

The local MVP should run as one Tauri/Rust desktop application that launches the WhisperX worker on demand. Use the repository's final scripts once created; the expected shape is:

```bash
# Rust static checks and tests
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace

# Frontend and Python worker contract checks
npm --prefix apps/web run build
npm --prefix apps/web run lint
python -m pytest services/whisperx-worker/tests -q
npx playwright test

# Lightweight local app
cargo tauri dev
# In a separate terminal, inspect the app's local health/diagnostic command only if exposed
cargo run -p worker-coordinator --bin whisperx-live-smoke -- --audio tests/fixtures/short_meeting.wav --window-seconds <window> --overlap-seconds <overlap>

# GPU worker image only when GPU deployment is selected
docker compose -f docker-compose.gpu.yml config
```

The live smoke test must report first-result latency, steady-state latency, queue depth/dropped windows, event sequence continuity, worker startup cost, worker shutdown, and finalization status. Do not claim native streaming support: the live path is a Rust-managed rolling-window compatibility layer around WhisperX's batch API, followed by a final full-recording WhisperX pass. The real test must import WhisperX in the on-demand isolated worker, load the selected model, process sequential windows through the Rust coordinator, and produce validated provisional and final output. If the host lacks model access, CUDA, or a suitable fixture, report that exact blocker and keep the CPU/adapter tests separate.

---

## Risks and mitigations

- **Capture limitations:** Browser system-audio APIs vary by browser/OS and can omit audio. Make the MVP desktop-first with an explicit Tauri/Rust capability check, microphone support, and platform-specific loopback adapters; retain browser capture/upload only as fallback paths.
- **WhisperX streaming limitation:** WhisperX's primary API is batch-oriented, not native streaming. Use warm rolling-window inference for live feedback and a final full-recording pass for authoritative output. Clearly label provisional text and measure the actual latency/accuracy tradeoff.
- **Live audio reliability:** A worker outage or queue overload must not lose the durable recording. Separate the capture/write path from inference, bound memory, checkpoint to disk, expose dropped-window metrics, and allow replay of unprocessed windows.
- **WhisperX compatibility:** Rust should not try to reimplement WhisperX or embed its Python dependency graph. Python, Torch, CUDA, CTranslate2, PyAV, and pyannote versions are coupled; isolate, pin, and test the worker image plus its Rust/Python protocol.
- **Rust/Python boundary failures:** Treat timeouts, worker crashes, malformed JSON, protocol mismatches, and missing artifacts as typed session/job failures; use protocol versioning, bounded output, retries only for transient errors, and fixture-based contract tests.
- **VRAM and latency:** Large models may exceed available memory. Use a small/fast live model, explicit device/compute settings, bounded windows/concurrency, model cache, CPU fallback, and report measured first-result/steady-state latency.
- **Alignment gaps:** Unsupported languages or dictionary-missing tokens may lack word timestamps. Preserve segment-level timings and label partial alignment.
- **Diarization quality:** Overlapping speech and imperfect speaker clustering are known limitations. Do not block live transcription on diarization; allow corrections and use final-pass diarization for authoritative labels.
- **Privacy/legal:** Obtain explicit consent appropriate to jurisdiction, show recording state to participants where required, protect artifacts, and make deletion meaningful.
- **LLM hallucination:** Generate final notes only from the finalized transcript; enforce structured output, source citations, confidence/unknown markers, and editable notes.
- **Long meetings:** Append audio incrementally, checkpoint live windows, bound transcript/event history, and make final processing resumable.
- **Untrusted model output:** Escape UI rendering and validate exports to prevent stored XSS or malicious links.

## Handoff

Plan updated at `.hermes/plans/2026-09-04_122410-ai-meeting-assistant.md`. The privacy rule is explicit: **the app only listens after the user presses Record**. Before that, it may display metadata-only device capabilities, but it must not open or sample audio devices, request/use an active audio stream, buffer frames, run VAD, start WhisperX, or persist audio. Pause, Stop, errors, permission revocation, and app close all stop listening, and capture never restarts automatically. Rust remains the lightweight application runtime; WhisperX starts on demand only after Record.
