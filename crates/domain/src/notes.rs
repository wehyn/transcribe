use crate::{
    CaptureConfig, DraftNotes, FinalNotes, LanguageMode, NoteCitation, NoteDocument, NoteError,
    NoteItem, NoteSections, TranscriptSegmentRange,
};

pub fn draft_from_transcript(text: impl Into<String>) -> DraftNotes {
    DraftNotes::new(NoteSections {
        summary: NoteItem::new(text),
        ..NoteSections::default()
    })
}

pub fn final_from_transcript(
    summary: impl Into<String>,
    segments: &[TranscriptSegmentRange],
) -> Result<FinalNotes, NoteError> {
    FinalNotes::from_sections(
        NoteSections {
            summary: NoteItem::with_citations(
                summary,
                segments
                    .first()
                    .copied()
                    .into_iter()
                    .map(|range| NoteCitation::new(range.start_micros, range.end_micros))
                    .collect::<Result<Vec<_>, _>>()?,
            ),
            ..NoteSections::default()
        },
        segments,
    )
}

pub fn default_capture(language: LanguageMode) -> CaptureConfig {
    CaptureConfig::dual_source(language)
}

pub fn serialize_document(document: &NoteDocument) -> String {
    serde_json::to_string_pretty(document).expect("note document is serializable")
}
