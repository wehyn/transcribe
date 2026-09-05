use crate::{AudioWindow, LanguageMode};
use std::collections::VecDeque;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowConfig {
    pub window_micros: u64,
    pub overlap_micros: u64,
    pub max_pending_windows: usize,
}

impl WindowConfig {
    pub fn new(window_seconds: u64, overlap_seconds: u64, max_pending_windows: usize) -> Self {
        assert!(window_seconds > overlap_seconds);
        assert!(max_pending_windows > 0);
        Self {
            window_micros: window_seconds * 1_000_000,
            overlap_micros: overlap_seconds * 1_000_000,
            max_pending_windows,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WindowError {
    Backpressure,
    NonMonotonicTimestamp,
    InvalidPcm,
}

#[derive(Debug)]
pub struct RollingWindowBuffer {
    config: WindowConfig,
    session_id: String,
    sample_rate: u32,
    channels: u16,
    language: LanguageMode,
    next_sequence: u64,
    pending: VecDeque<AudioWindow>,
    samples: Vec<u8>,
    start_micros: Option<u64>,
    last_end_micros: Option<u64>,
}

impl RollingWindowBuffer {
    pub fn new(
        config: WindowConfig,
        session_id: impl Into<String>,
        sample_rate: u32,
        channels: u16,
        language: LanguageMode,
    ) -> Self {
        Self {
            config,
            session_id: session_id.into(),
            sample_rate,
            channels,
            language,
            next_sequence: 0,
            pending: VecDeque::new(),
            samples: Vec::new(),
            start_micros: None,
            last_end_micros: None,
        }
    }

    pub fn push_pcm(
        &mut self,
        timestamp_micros: u64,
        pcm_f32_le: &[u8],
    ) -> Result<(), WindowError> {
        if !pcm_f32_le
            .len()
            .is_multiple_of(4 * usize::from(self.channels))
        {
            return Err(WindowError::InvalidPcm);
        }
        let frame_count = pcm_f32_le.len() as u64 / 4 / u64::from(self.channels);
        let duration_micros = frame_count * 1_000_000 / u64::from(self.sample_rate);
        let end_micros = timestamp_micros + duration_micros;

        if self
            .last_end_micros
            .is_some_and(|last_end| timestamp_micros < last_end)
        {
            return Err(WindowError::NonMonotonicTimestamp);
        }
        self.last_end_micros = Some(end_micros);
        self.start_micros.get_or_insert(timestamp_micros);
        self.samples.extend_from_slice(pcm_f32_le);

        if end_micros - self.start_micros.unwrap_or(end_micros) >= self.config.window_micros {
            self.emit_window(end_micros)?;
        }
        Ok(())
    }

    pub fn pop_window(&mut self) -> Option<AudioWindow> {
        self.pending.pop_front()
    }

    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }

    fn emit_window(&mut self, end_micros: u64) -> Result<(), WindowError> {
        if self.pending.len() >= self.config.max_pending_windows {
            return Err(WindowError::Backpressure);
        }
        let start_micros = self.start_micros.expect("window start exists");
        self.pending.push_back(AudioWindow {
            session_id: self.session_id.clone(),
            sequence: self.next_sequence,
            start_micros,
            end_micros,
            sample_rate: self.sample_rate,
            channels: self.channels,
            pcm_f32_le: self.samples.clone(),
            language: self.language,
        });
        self.next_sequence += 1;
        self.start_micros = Some(end_micros.saturating_sub(self.config.overlap_micros));
        let overlap_bytes = self.config.overlap_micros * u64::from(self.sample_rate) / 1_000_000
            * u64::from(self.channels)
            * 4;
        let keep_from = self.samples.len().saturating_sub(overlap_bytes as usize);
        self.samples = self.samples[keep_from..].to_vec();
        Ok(())
    }
}
