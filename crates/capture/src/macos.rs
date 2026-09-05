use std::sync::Arc;
use std::time::Instant;

use super::{
    AudioFrame, AudioSinkHandle, CaptureCapabilities, CaptureSource, CaptureTrack, SourceError,
};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream, StreamConfig};
use meeting_domain::CaptureConfig;
use screencapturekit::prelude::*;

#[derive(Clone)]
struct FrameSink {
    sink: AudioSinkHandle,
    started_at: Instant,
}

impl FrameSink {
    fn emit(&self, track: CaptureTrack, sample_rate: u32, channels: u16, samples: &[f32]) {
        let timestamp_micros = self.started_at.elapsed().as_micros() as u64;
        self.sink.on_audio_frame(AudioFrame {
            track,
            sample_rate,
            channels,
            timestamp_micros,
            samples,
        });
    }
}

struct MicrophoneCapture {
    _stream: Stream,
}

struct SystemAudioHandler {
    frame_sink: FrameSink,
    sample_rate: u32,
    channels: u16,
}

impl SCStreamOutputTrait for SystemAudioHandler {
    fn did_output_sample_buffer(&self, sample: CMSampleBuffer, kind: SCStreamOutputType) {
        if kind != SCStreamOutputType::Audio {
            return;
        }
        if let Some(samples) = audio_samples(&sample) {
            self.frame_sink.emit(
                CaptureTrack::System,
                self.sample_rate,
                self.channels,
                &samples,
            );
        }
    }
}

pub struct MacOsCaptureSource {
    microphone_open: bool,
    system_audio_open: bool,
    microphone: Option<MicrophoneCapture>,
    system_stream: Option<SCStream>,
    sink: Option<AudioSinkHandle>,
}

impl Default for MacOsCaptureSource {
    fn default() -> Self {
        Self {
            microphone_open: false,
            system_audio_open: false,
            microphone: None,
            system_stream: None,
            sink: None,
        }
    }
}

impl MacOsCaptureSource {
    pub fn new() -> Self {
        Self::default()
    }

    fn microphone_available() -> bool {
        cpal::default_host().default_input_device().is_some()
    }

    fn system_audio_available() -> bool {
        SCShareableContent::get()
            .map(|content| !content.displays().is_empty())
            .unwrap_or(false)
    }

    fn open_microphone(&mut self, sink: FrameSink) -> Result<(), SourceError> {
        let device = cpal::default_host()
            .default_input_device()
            .ok_or(SourceError::MicrophoneUnavailable)?;
        let supported = device
            .default_input_config()
            .map_err(|_| SourceError::MicrophoneUnavailable)?;
        let sample_rate = supported.sample_rate().0;
        let channels = supported.channels();
        let stream_config: StreamConfig = supported.config();
        let error_sink = sink.sink.clone();
        let stream = match supported.sample_format() {
            SampleFormat::F32 => device.build_input_stream(
                &stream_config,
                move |data: &[f32], _| {
                    sink.emit(CaptureTrack::Microphone, sample_rate, channels, data)
                },
                move |error| {
                    error_sink.on_capture_error(CaptureTrack::Microphone, &error.to_string())
                },
                None,
            ),
            SampleFormat::I16 => {
                let sink = sink.clone();
                device.build_input_stream(
                    &stream_config,
                    move |data: &[i16], _| {
                        let samples: Vec<f32> = data
                            .iter()
                            .map(|value| *value as f32 / i16::MAX as f32)
                            .collect();
                        sink.emit(CaptureTrack::Microphone, sample_rate, channels, &samples);
                    },
                    move |error| {
                        error_sink.on_capture_error(CaptureTrack::Microphone, &error.to_string())
                    },
                    None,
                )
            }
            SampleFormat::U16 => {
                let sink = sink.clone();
                device.build_input_stream(
                    &stream_config,
                    move |data: &[u16], _| {
                        let samples: Vec<f32> = data
                            .iter()
                            .map(|value| (*value as f32 / u16::MAX as f32) * 2.0 - 1.0)
                            .collect();
                        sink.emit(CaptureTrack::Microphone, sample_rate, channels, &samples);
                    },
                    move |error| {
                        error_sink.on_capture_error(CaptureTrack::Microphone, &error.to_string())
                    },
                    None,
                )
            }
            _ => return Err(SourceError::MicrophoneUnavailable),
        }
        .map_err(|_| SourceError::MicrophoneUnavailable)?;
        stream
            .play()
            .map_err(|_| SourceError::MicrophoneUnavailable)?;
        self.microphone = Some(MicrophoneCapture { _stream: stream });
        Ok(())
    }

    fn open_system_audio(&mut self, sink: FrameSink) -> Result<(), SourceError> {
        let content = SCShareableContent::get().map_err(|_| SourceError::SystemAudioUnavailable)?;
        let display = content
            .displays()
            .into_iter()
            .next()
            .ok_or(SourceError::SystemAudioUnavailable)?;
        let filter = SCContentFilter::new().with_display_excluding_windows(&display, &[]);
        let configuration = SCStreamConfiguration::new()
            .set_captures_audio(true)
            .map_err(|_| SourceError::SystemAudioUnavailable)?
            .set_sample_rate(48_000)
            .map_err(|_| SourceError::SystemAudioUnavailable)?
            .set_channel_count(2)
            .map_err(|_| SourceError::SystemAudioUnavailable)?;
        let mut stream = SCStream::new(&filter, &configuration);
        stream.add_output_handler(
            SystemAudioHandler {
                frame_sink: sink,
                sample_rate: 48_000,
                channels: 2,
            },
            SCStreamOutputType::Audio,
        );
        stream
            .start_capture()
            .map_err(|_| SourceError::SystemAudioUnavailable)?;
        self.system_stream = Some(stream);
        Ok(())
    }
}

impl CaptureSource for MacOsCaptureSource {
    fn capabilities(&self) -> CaptureCapabilities {
        CaptureCapabilities {
            microphone_available: Self::microphone_available(),
            system_audio_available: Self::system_audio_available(),
        }
    }

    fn open(&mut self, config: &CaptureConfig, sink: AudioSinkHandle) -> Result<(), SourceError> {
        if self.is_open() {
            return Err(SourceError::AlreadyOpen);
        }
        if config.microphone && !Self::microphone_available() {
            return Err(SourceError::MicrophoneUnavailable);
        }
        if config.system_audio && !Self::system_audio_available() {
            return Err(SourceError::SystemAudioUnavailable);
        }
        let frame_sink = FrameSink {
            sink: Arc::clone(&sink),
            started_at: Instant::now(),
        };
        self.sink = Some(sink);
        if config.microphone {
            self.open_microphone(frame_sink.clone())?;
            self.microphone_open = true;
        }
        if config.system_audio {
            if let Err(error) = self.open_system_audio(frame_sink) {
                self.close();
                return Err(error);
            }
            self.system_audio_open = true;
        }
        Ok(())
    }

    fn close(&mut self) {
        if let Some(stream) = self.system_stream.take() {
            let _ = stream.stop_capture();
        }
        self.microphone.take();
        self.microphone_open = false;
        self.system_audio_open = false;
        self.sink = None;
    }

    fn is_open(&self) -> bool {
        self.microphone_open || self.system_audio_open
    }
}

fn audio_samples(_sample: &CMSampleBuffer) -> Option<Vec<f32>> {
    let buffers = _sample.get_audio_buffer_list().ok()?;
    let mut samples = Vec::new();
    for index in 0..buffers.num_buffers() {
        let buffer = buffers.get(index)?;
        let bytes = buffer.data();
        let (chunks, remainder) = bytes.as_chunks::<4>();
        if !remainder.is_empty() {
            return None;
        }
        samples.extend(chunks.iter().map(|chunk| f32::from_le_bytes(*chunk)));
    }
    Some(samples)
}
