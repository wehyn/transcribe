# macOS capture validation checklist

The portable capture contract is implemented and the macOS source opens its CPAL microphone stream and ScreenCaptureKit audio stream only from `CaptureSource::open`, which is called by Record. The Linux host cannot execute this adapter.

Run on a macOS 13+ build host:

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cd apps/desktop/ui && npm install && npm run build
cd ../src-tauri && cargo tauri build
```

Before Record, inspect Activity Monitor and the app logs: no microphone stream, ScreenCaptureKit stream, audio callback, audio artifact, worker process, VAD, or note generation may exist. After Record, verify:

- microphone samples arrive in `microphone.pcm`;
- ScreenCaptureKit audio samples arrive in `system.pcm`;
- mixed samples arrive in `mixed.pcm`;
- timestamps are monotonic and normalized to the selected recording format;
- Pause and Stop close both native streams;
- permission revocation reports an error and does not restart capture;
- closing the window while recording seals or safely leaves the retained bundle;
- the worker receives `mixed.wav` only after capture stops;
- final transcript segments retain timestamp and alignment status.

The adapter uses `CMSampleBuffer::get_audio_buffer_list()` from `screencapturekit` 0.3.6 and converts each returned buffer's packed little-endian float bytes into normalized `f32` samples. Validate the actual ScreenCaptureKit sample format on macOS; the portable code still cannot prove permissions, callback delivery, channel layout, or end-to-end capture from Linux.
