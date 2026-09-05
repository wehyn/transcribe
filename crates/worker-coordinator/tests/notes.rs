use whisperx_worker::{
    Citation, DraftNotes, FinalTranscript, MeetingNotes, NoteItem, NoteVersion, TranscriptSegment,
    generate_final_notes,
};

fn transcript() -> FinalTranscript {
    FinalTranscript {
        session_id: "session-1".into(),
        language: whisperx_worker::LanguageMode::Taglish,
        text: "We agreed to ship Friday. TODO: Alex sends the release notes. Open question: which timezone?".into(),
        segments: vec![
            TranscriptSegment {
                start_micros: 0,
                end_micros: 1_000_000,
                text: "We agreed to ship Friday.".into(),
                words: Vec::new(),
                speaker: Some("SPEAKER_00".into()),
                alignment_status: whisperx_worker::AlignmentStatus::Segment,
            },
            TranscriptSegment {
                start_micros: 1_000_000,
                end_micros: 2_000_000,
                text: "TODO: Alex sends the release notes.".into(),
                words: Vec::new(),
                speaker: Some("SPEAKER_01".into()),
                alignment_status: whisperx_worker::AlignmentStatus::Segment,
            },
            TranscriptSegment {
                start_micros: 2_000_000,
                end_micros: 3_000_000,
                text: "Open question: which timezone?".into(),
                words: Vec::new(),
                speaker: None,
                alignment_status: whisperx_worker::AlignmentStatus::Segment,
            },
        ],
    }
}

#[test]
fn draft_notes_are_editable_and_labeled_draft() {
    let mut draft = DraftNotes::new();
    draft.update_from_text("TODO: send the agenda");
    let mut edited = draft.notes().clone();
    edited.summary = "Edited summary".into();
    draft.edit(edited).unwrap();

    assert_eq!(draft.notes().version, NoteVersion::Draft);
    assert_eq!(draft.notes().summary, "Edited summary");
    assert_eq!(draft.notes().action_items[0].text, "TODO: send the agenda");
}

#[test]
fn final_notes_are_cited_to_authoritative_transcript_ranges() {
    let notes = generate_final_notes(&transcript()).unwrap();

    assert_eq!(notes.version, NoteVersion::Final);
    assert_eq!(notes.decisions.len(), 1);
    assert_eq!(notes.action_items.len(), 1);
    assert_eq!(notes.open_questions.len(), 1);
    assert_eq!(notes.action_items[0].citations[0].start_micros, 1_000_000);
    notes.validate_against(&transcript()).unwrap();
}

#[test]
fn invalid_citation_ranges_and_unmatched_citations_are_rejected() {
    assert_eq!(
        Citation::new(2, 1, None),
        Err(whisperx_worker::NotesError::InvalidCitationRange)
    );
    assert_eq!(
        Citation::new(2, 2, None),
        Err(whisperx_worker::NotesError::InvalidCitationRange)
    );
    let mut notes = MeetingNotes::empty(NoteVersion::Final);
    notes.decisions.push(NoteItem {
        text: "unsupported".into(),
        citations: vec![Citation::new(9, 10, None).unwrap()],
        completed: false,
    });

    assert_eq!(
        notes.validate_against(&transcript()),
        Err(whisperx_worker::NotesError::CitationNotInTranscript)
    );
}

#[test]
fn generated_final_notes_do_not_mutate_transcript() {
    let original = transcript();
    let _notes = generate_final_notes(&original).unwrap();

    assert_eq!(original.text, transcript().text);
    assert_eq!(original.segments.len(), 3);
}
