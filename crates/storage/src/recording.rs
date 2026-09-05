use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackRole {
    Microphone,
    System,
    Mixed,
}

impl TrackRole {
    fn filename(self) -> &'static str {
        match self {
            Self::Microphone => "microphone.pcm",
            Self::System => "system.pcm",
            Self::Mixed => "mixed.pcm",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioFormat {
    pub sample_rate: u32,
    pub channels: u16,
}

impl Default for AudioFormat {
    fn default() -> Self {
        Self {
            sample_rate: 48_000,
            channels: 1,
        }
    }
}

impl AudioFormat {
    pub fn pcm_f32(sample_rate: u32, channels: u16) -> Self {
        Self {
            sample_rate,
            channels,
        }
    }
}

#[derive(Debug, Error)]
pub enum RecordingError {
    #[error("recording directory could not be created: {0}")]
    CreateDirectory(#[source] io::Error),
    #[error("recording track could not be opened: {0}")]
    Open(#[source] io::Error),
    #[error("recording track could not be written: {0}")]
    Write(#[source] io::Error),
    #[error("recording track could not be sealed: {0}")]
    Seal(#[source] io::Error),
    #[error("recording manifest could not be written: {0}")]
    Manifest(#[source] io::Error),
    #[error("recording track could not be converted to WAV: {0}")]
    Wav(#[source] io::Error),
    #[error("track format does not match the recording format")]
    FormatMismatch,
    #[error("recording has already been sealed")]
    AlreadySealed,
}

pub struct RecordingBundle {
    root: PathBuf,
    format: AudioFormat,
    microphone: File,
    system: File,
    mixed: File,
    sealed: bool,
}

impl RecordingBundle {
    pub fn create(root: impl AsRef<Path>) -> Result<Self, RecordingError> {
        Self::create_with_format(root, AudioFormat::default())
    }

    pub fn create_with_format(
        root: impl AsRef<Path>,
        format: AudioFormat,
    ) -> Result<Self, RecordingError> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(&root).map_err(RecordingError::CreateDirectory)?;

        Ok(Self {
            microphone: open_track(&root, TrackRole::Microphone)?,
            system: open_track(&root, TrackRole::System)?,
            mixed: open_track(&root, TrackRole::Mixed)?,
            root,
            format,
            sealed: false,
        })
    }

    pub fn format(&self) -> AudioFormat {
        self.format
    }

    pub fn append(&mut self, role: TrackRole, samples: &[f32]) -> Result<(), RecordingError> {
        if self.sealed {
            return Err(RecordingError::AlreadySealed);
        }
        let file = match role {
            TrackRole::Microphone => &mut self.microphone,
            TrackRole::System => &mut self.system,
            TrackRole::Mixed => &mut self.mixed,
        };
        for sample in samples {
            file.write_all(&sample.to_le_bytes())
                .map_err(RecordingError::Write)?;
        }
        Ok(())
    }

    pub fn append_frame(
        &mut self,
        role: TrackRole,
        format: AudioFormat,
        samples: &[f32],
    ) -> Result<(), RecordingError> {
        if format != self.format {
            return Err(RecordingError::FormatMismatch);
        }
        self.append(role, samples)
    }

    pub fn flush(&mut self) -> Result<(), RecordingError> {
        if self.sealed {
            return Err(RecordingError::AlreadySealed);
        }
        self.microphone.flush().map_err(RecordingError::Write)?;
        self.system.flush().map_err(RecordingError::Write)?;
        self.mixed.flush().map_err(RecordingError::Write)?;
        Ok(())
    }

    pub fn seal(mut self) -> Result<SealedRecordingBundle, RecordingError> {
        self.flush()?;
        self.microphone.sync_all().map_err(RecordingError::Seal)?;
        self.system.sync_all().map_err(RecordingError::Seal)?;
        self.mixed.sync_all().map_err(RecordingError::Seal)?;
        write_manifest(&self.root, self.format).map_err(RecordingError::Manifest)?;
        self.sealed = true;

        Ok(SealedRecordingBundle {
            root: self.root.clone(),
            format: self.format,
        })
    }
}

pub struct SealedRecordingBundle {
    root: PathBuf,
    format: AudioFormat,
}

impl SealedRecordingBundle {
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn format(&self) -> AudioFormat {
        self.format
    }

    pub fn track_path(&self, role: TrackRole) -> PathBuf {
        self.root.join(role.filename())
    }

    pub fn wav_path(&self) -> PathBuf {
        self.root.join("mixed.wav")
    }

    pub fn materialize_mixed_wav(&self) -> Result<PathBuf, RecordingError> {
        let pcm = fs::read(self.track_path(TrackRole::Mixed)).map_err(RecordingError::Wav)?;
        let path = self.wav_path();
        write_pcm_wav(&path, self.format, &pcm).map_err(RecordingError::Wav)?;
        Ok(path)
    }

    pub fn whisperx_audio_path(&self) -> Result<PathBuf, RecordingError> {
        self.materialize_mixed_wav()
    }

    pub fn retained(&self) -> bool {
        self.track_path(TrackRole::Microphone).exists()
            && self.track_path(TrackRole::System).exists()
            && self.track_path(TrackRole::Mixed).exists()
            && self.root.join("manifest.json").exists()
    }
}

impl Drop for RecordingBundle {
    fn drop(&mut self) {
        if !self.sealed {
            let _ = self.flush();
        }
    }
}

fn open_track(root: &Path, role: TrackRole) -> Result<File, RecordingError> {
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(root.join(role.filename()))
        .map_err(RecordingError::Open)
}

fn write_manifest(root: &Path, format: AudioFormat) -> io::Result<()> {
    let manifest = format!(
        "{{\"format\":{{\"sample_rate\":{},\"channels\":{}}},\"tracks\":[\"microphone.pcm\",\"system.pcm\",\"mixed.pcm\"]}}\n",
        format.sample_rate, format.channels
    );
    fs::write(root.join("manifest.json"), manifest)
}

#[allow(clippy::chunks_exact_to_as_chunks)]
fn write_pcm_wav(path: &Path, format: AudioFormat, pcm_f32_le: &[u8]) -> io::Result<()> {
    if !pcm_f32_le.len().is_multiple_of(4) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "f32 PCM has an incomplete sample",
        ));
    }
    if format.sample_rate == 0 || format.channels == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "WAV format must have a non-zero sample rate and channel count",
        ));
    }
    let sample_count = pcm_f32_le.len() / 4;
    let mut pcm_i16_le = Vec::with_capacity(sample_count * 2);
    for chunk in pcm_f32_le.chunks_exact(4) {
        let sample = f32::from_le_bytes(chunk.try_into().expect("four-byte chunk"));
        let sample = if sample.is_finite() {
            sample.clamp(-1.0, 1.0)
        } else {
            0.0
        };
        let integer = if sample <= -1.0 {
            i16::MIN
        } else {
            (sample * f32::from(i16::MAX)).round() as i16
        };
        pcm_i16_le.extend_from_slice(&integer.to_le_bytes());
    }

    let block_align = format
        .channels
        .checked_mul(2)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "channel count overflow"))?;
    let byte_rate = format
        .sample_rate
        .checked_mul(u32::from(block_align))
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "sample rate overflow"))?;
    let data_len = u32::try_from(pcm_i16_le.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "WAV data is too large"))?;
    let riff_len = 36_u32
        .checked_add(data_len)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "WAV is too large"))?;

    let mut wav = Vec::with_capacity(44 + pcm_i16_le.len());
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&riff_len.to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16_u32.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&format.channels.to_le_bytes());
    wav.extend_from_slice(&format.sample_rate.to_le_bytes());
    wav.extend_from_slice(&byte_rate.to_le_bytes());
    wav.extend_from_slice(&block_align.to_le_bytes());
    wav.extend_from_slice(&16_u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    wav.extend_from_slice(&pcm_i16_le);
    fs::write(path, wav)
}
