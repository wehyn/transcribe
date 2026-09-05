use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{FinalTranscript, TranscriptSegment};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoteVersion {
    Draft,
    Final,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Citation {
    pub start_micros: u64,
    pub end_micros: u64,
    pub quote: Option<String>,
}

impl Citation {
    pub fn new(
        start_micros: u64,
        end_micros: u64,
        quote: Option<String>,
    ) -> Result<Self, NotesError> {
        if start_micros >= end_micros {
            return Err(NotesError::InvalidCitationRange);
        }
        Ok(Self {
            start_micros,
            end_micros,
            quote,
        })
    }

    pub fn covers(&self, segment: &TranscriptSegment) -> bool {
        self.start_micros <= segment.start_micros && self.end_micros >= segment.end_micros
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteItem {
    pub text: String,
    pub citations: Vec<Citation>,
    pub completed: bool,
}

impl NoteItem {
    pub fn draft(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            citations: Vec::new(),
            completed: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeetingNotes {
    pub version: NoteVersion,
    pub summary: String,
    pub decisions: Vec<NoteItem>,
    pub action_items: Vec<NoteItem>,
    pub open_questions: Vec<NoteItem>,
}

impl MeetingNotes {
    pub fn empty(version: NoteVersion) -> Self {
        Self {
            version,
            summary: String::new(),
            decisions: Vec::new(),
            action_items: Vec::new(),
            open_questions: Vec::new(),
        }
    }

    pub fn validate_against(&self, transcript: &FinalTranscript) -> Result<(), NotesError> {
        let segments = &transcript.segments;
        for item in self
            .decisions
            .iter()
            .chain(self.action_items.iter())
            .chain(self.open_questions.iter())
        {
            for citation in &item.citations {
                if !segments.iter().any(|segment| citation.covers(segment)) {
                    return Err(NotesError::CitationNotInTranscript);
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum NotesError {
    #[error("citation range must have a positive duration")]
    InvalidCitationRange,
    #[error("citation does not cover a final transcript segment")]
    CitationNotInTranscript,
}

#[derive(Debug)]
pub struct DraftNotes {
    notes: MeetingNotes,
    last_transcript: String,
}

impl DraftNotes {
    pub fn new() -> Self {
        Self {
            notes: MeetingNotes::empty(NoteVersion::Draft),
            last_transcript: String::new(),
        }
    }

    pub fn update_from_text(&mut self, text: impl Into<String>) -> &MeetingNotes {
        let text = text.into();
        if text == self.last_transcript {
            return &self.notes;
        }
        self.last_transcript = text.clone();
        self.notes.summary = summarize_text(&text);
        self.notes.action_items = detect_items(&text, &["todo", "action", "follow up", "assign"]);
        self.notes.decisions = detect_items(&text, &["decide", "decision", "agreed"]);
        self.notes.open_questions = detect_items(&text, &["question", "unknown", "open"]);
        &self.notes
    }

    pub fn notes(&self) -> &MeetingNotes {
        &self.notes
    }

    pub fn edit(&mut self, notes: MeetingNotes) -> Result<(), NotesError> {
        if notes.version != NoteVersion::Draft {
            return Err(NotesError::CitationNotInTranscript);
        }
        self.notes = notes;
        Ok(())
    }
}

impl Default for DraftNotes {
    fn default() -> Self {
        Self::new()
    }
}

pub fn generate_final_notes(transcript: &FinalTranscript) -> Result<MeetingNotes, NotesError> {
    let mut notes = MeetingNotes::empty(NoteVersion::Final);
    notes.summary = summarize_text(&transcript.text);
    let mut seen = HashMap::new();
    for segment in &transcript.segments {
        let lower = segment.text.to_lowercase();
        let bucket = if contains_any(&lower, &["todo", "action", "follow up", "assign"]) {
            Some(&mut notes.action_items)
        } else if contains_any(&lower, &["decide", "decision", "agreed"]) {
            Some(&mut notes.decisions)
        } else if contains_any(&lower, &["question", "unknown", "open"]) {
            Some(&mut notes.open_questions)
        } else {
            None
        };
        #[allow(clippy::collapsible_if)]
        if let Some(items) = bucket {
            if seen.insert(segment.start_micros, true).is_none() {
                items.push(NoteItem {
                    text: segment.text.clone(),
                    citations: vec![Citation::new(
                        segment.start_micros,
                        segment.end_micros,
                        Some(segment.text.clone()),
                    )?],
                    completed: false,
                });
            }
        }
    }
    notes.validate_against(transcript)?;
    Ok(notes)
}

fn summarize_text(text: &str) -> String {
    text.split_whitespace()
        .take(40)
        .collect::<Vec<_>>()
        .join(" ")
}

fn detect_items(text: &str, markers: &[&str]) -> Vec<NoteItem> {
    text.lines()
        .filter(|line| contains_any(&line.to_lowercase(), markers))
        .map(|line| NoteItem::draft(line.trim()))
        .collect()
}

fn contains_any(text: &str, markers: &[&str]) -> bool {
    markers.iter().any(|marker| text.contains(marker))
}
