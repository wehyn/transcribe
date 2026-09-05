use serde::{Deserialize, Serialize};
use thiserror::Error;

/// A time range occupied by one normalized transcript segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranscriptSegmentRange {
    pub start_micros: u64,
    pub end_micros: u64,
}

impl TranscriptSegmentRange {
    pub fn new(start_micros: u64, end_micros: u64) -> Result<Self, NoteError> {
        if start_micros >= end_micros {
            return Err(NoteError::InvalidTimeRange {
                start_micros,
                end_micros,
            });
        }

        Ok(Self {
            start_micros,
            end_micros,
        })
    }

    fn contains(&self, citation: &NoteCitation) -> bool {
        self.start_micros <= citation.start_micros && citation.end_micros <= self.end_micros
    }
}

/// A source range attached to a note item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteCitation {
    pub start_micros: u64,
    pub end_micros: u64,
}

impl NoteCitation {
    pub fn new(start_micros: u64, end_micros: u64) -> Result<Self, NoteError> {
        if start_micros >= end_micros {
            return Err(NoteError::InvalidCitationRange {
                start_micros,
                end_micros,
            });
        }

        Ok(Self {
            start_micros,
            end_micros,
        })
    }
}

/// One editable note claim and the transcript ranges supporting it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteItem {
    pub text: String,
    #[serde(default)]
    pub citations: Vec<NoteCitation>,
}

impl NoteItem {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            citations: Vec::new(),
        }
    }

    pub fn with_citations(text: impl Into<String>, citations: Vec<NoteCitation>) -> Self {
        Self {
            text: text.into(),
            citations,
        }
    }
}

/// The four note sections shared by live drafts and final notes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteSections {
    pub summary: NoteItem,
    #[serde(default)]
    pub decisions: Vec<NoteItem>,
    #[serde(default)]
    pub action_items: Vec<NoteItem>,
    #[serde(default)]
    pub open_questions: Vec<NoteItem>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoteStatus {
    Draft,
    Final,
}

/// Notes that can be updated while a transcript is still provisional.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DraftNotes {
    pub sections: NoteSections,
}

impl DraftNotes {
    pub fn new(sections: NoteSections) -> Self {
        Self { sections }
    }

    pub fn edit(&mut self, sections: NoteSections) {
        self.sections = sections;
    }

    pub fn status(&self) -> NoteStatus {
        NoteStatus::Draft
    }
}

/// Notes generated from an authoritative transcript and validated against its segments.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FinalNotes {
    pub sections: NoteSections,
}

impl FinalNotes {
    pub fn from_sections(
        sections: NoteSections,
        transcript_segments: &[TranscriptSegmentRange],
    ) -> Result<Self, NoteError> {
        sections.validate(transcript_segments)?;
        Ok(Self { sections })
    }

    pub fn status(&self) -> NoteStatus {
        NoteStatus::Final
    }

    pub fn is_authoritative(&self) -> bool {
        true
    }
}

/// A serialized note version whose status keeps drafts separate from final notes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", content = "notes", rename_all = "snake_case")]
pub enum NoteDocument {
    Draft(DraftNotes),
    Final(FinalNotes),
}

impl NoteDocument {
    pub fn status(&self) -> NoteStatus {
        match self {
            Self::Draft(notes) => notes.status(),
            Self::Final(notes) => notes.status(),
        }
    }

    pub fn is_authoritative(&self) -> bool {
        matches!(self, Self::Final(_))
    }

    pub fn edit(&mut self, sections: NoteSections) -> Result<(), NoteError> {
        match self {
            Self::Draft(notes) => {
                notes.edit(sections);
                Ok(())
            }
            Self::Final(_) => Err(NoteError::FinalNotesImmutable),
        }
    }
}

impl NoteSections {
    fn validate(&self, transcript_segments: &[TranscriptSegmentRange]) -> Result<(), NoteError> {
        validate_item("summary", None, &self.summary, transcript_segments)?;
        validate_items("decisions", &self.decisions, transcript_segments)?;
        validate_items("action_items", &self.action_items, transcript_segments)?;
        validate_items("open_questions", &self.open_questions, transcript_segments)?;
        Ok(())
    }
}

fn validate_items(
    section: &'static str,
    items: &[NoteItem],
    transcript_segments: &[TranscriptSegmentRange],
) -> Result<(), NoteError> {
    for (item_index, item) in items.iter().enumerate() {
        validate_item(section, Some(item_index), item, transcript_segments)?;
    }
    Ok(())
}

fn validate_item(
    section: &'static str,
    item_index: Option<usize>,
    item: &NoteItem,
    transcript_segments: &[TranscriptSegmentRange],
) -> Result<(), NoteError> {
    for citation in &item.citations {
        if citation.start_micros >= citation.end_micros {
            return Err(NoteError::InvalidCitationRange {
                start_micros: citation.start_micros,
                end_micros: citation.end_micros,
            });
        }
        if !transcript_segments
            .iter()
            .any(|segment| segment.contains(citation))
        {
            return Err(NoteError::CitationNotInTranscript {
                start_micros: citation.start_micros,
                end_micros: citation.end_micros,
            });
        }
    }

    if !item.text.trim().is_empty() && item.citations.is_empty() {
        return Err(NoteError::MissingCitation {
            section,
            item_index,
        });
    }

    Ok(())
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum NoteError {
    #[error("invalid time range {start_micros}..{end_micros}")]
    InvalidTimeRange { start_micros: u64, end_micros: u64 },
    #[error("invalid citation range {start_micros}..{end_micros}")]
    InvalidCitationRange { start_micros: u64, end_micros: u64 },
    #[error("citation range {start_micros}..{end_micros} is not in the transcript")]
    CitationNotInTranscript { start_micros: u64, end_micros: u64 },
    #[error("{section} note item {item_index:?} is missing a citation")]
    MissingCitation {
        section: &'static str,
        item_index: Option<usize>,
    },
    #[error("final notes are authoritative and cannot be edited")]
    FinalNotesImmutable,
}
