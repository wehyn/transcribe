use crate::{AudioWindow, TranscriptResult};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveTranscript {
    pub session_id: String,
    pub text: String,
    pub segments: Vec<TranscriptResult>,
}

#[derive(Debug, Default)]
pub struct TranscriptReconciler {
    windows: BTreeMap<u64, TranscriptResult>,
}

impl TranscriptReconciler {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn accept(&mut self, result: TranscriptResult) {
        self.windows
            .entry(result.sequence)
            .and_modify(|current| {
                if current.provisional && !result.provisional {
                    *current = result.clone();
                }
            })
            .or_insert(result);
    }

    pub fn snapshot(&self) -> Option<LiveTranscript> {
        let segments: Vec<_> = self.windows.values().cloned().collect();
        let first = segments.first()?;
        let text = segments
            .iter()
            .map(|segment| segment.text.trim())
            .filter(|segment| !segment.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        Some(LiveTranscript {
            session_id: first.session_id.clone(),
            text,
            segments,
        })
    }

    pub fn finalize(
        &mut self,
        results: impl IntoIterator<Item = TranscriptResult>,
    ) -> LiveTranscript {
        self.windows.clear();
        for result in results {
            self.accept(result);
        }
        self.snapshot().unwrap_or_else(|| LiveTranscript {
            session_id: String::new(),
            text: String::new(),
            segments: Vec::new(),
        })
    }

    pub fn len(&self) -> usize {
        self.windows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.windows.is_empty()
    }
}

pub fn result_for(window: &AudioWindow, text: &str, provisional: bool) -> TranscriptResult {
    TranscriptResult {
        session_id: window.session_id.clone(),
        sequence: window.sequence,
        start_micros: window.start_micros,
        end_micros: window.end_micros,
        text: text.to_string(),
        words: Vec::new(),
        language: window.language,
        provisional,
    }
}
