use meeting_domain::{
    DraftNotes, FinalNotes, NoteCitation, NoteDocument, NoteError, NoteItem, NoteSections,
    NoteStatus, TranscriptSegmentRange,
};

fn segments() -> Vec<TranscriptSegmentRange> {
    vec![
        TranscriptSegmentRange::new(0, 500_000).unwrap(),
        TranscriptSegmentRange::new(700_000, 1_200_000).unwrap(),
    ]
}

fn cited_item(text: &str, start_micros: u64, end_micros: u64) -> NoteItem {
    NoteItem::with_citations(
        text,
        vec![NoteCitation::new(start_micros, end_micros).unwrap()],
    )
}

fn sections() -> NoteSections {
    NoteSections {
        summary: cited_item("The team aligned on the launch plan.", 0, 500_000),
        decisions: vec![cited_item("Ship the beta next week.", 700_000, 1_200_000)],
        action_items: vec![cited_item(
            "Ari will prepare the release checklist.",
            700_000,
            1_200_000,
        )],
        open_questions: vec![cited_item(
            "Who owns the support rotation?",
            700_000,
            1_200_000,
        )],
    }
}

#[test]
fn draft_notes_are_editable_and_round_trip_all_sections() {
    let mut draft = DraftNotes::new(sections());

    assert_eq!(draft.status(), NoteStatus::Draft);
    draft.edit(NoteSections {
        summary: NoteItem::new("Updated live summary."),
        ..sections()
    });
    assert_eq!(draft.sections.summary.text, "Updated live summary.");

    let document = NoteDocument::Draft(draft);
    let encoded = serde_json::to_string(&document).unwrap();
    let decoded: NoteDocument = serde_json::from_str(&encoded).unwrap();

    assert_eq!(decoded, document);
    assert!(encoded.contains("\"status\":\"draft\""));
}

#[test]
fn final_notes_are_authoritative_and_citations_must_reference_final_segments() {
    let final_notes = FinalNotes::from_sections(sections(), &segments()).unwrap();

    assert_eq!(final_notes.status(), NoteStatus::Final);
    assert!(final_notes.is_authoritative());

    let mut document = NoteDocument::Final(final_notes);
    let error = document.edit(sections()).unwrap_err();
    assert_eq!(error, NoteError::FinalNotesImmutable);

    let invalid = NoteSections {
        summary: cited_item("This range is not in the transcript.", 500_001, 600_000),
        ..NoteSections::default()
    };
    assert_eq!(
        FinalNotes::from_sections(invalid, &segments()),
        Err(NoteError::CitationNotInTranscript {
            start_micros: 500_001,
            end_micros: 600_000,
        })
    );
}

#[test]
fn citations_require_positive_ranges_and_final_items_require_citations() {
    assert_eq!(
        NoteCitation::new(10, 10),
        Err(NoteError::InvalidCitationRange {
            start_micros: 10,
            end_micros: 10,
        })
    );
    assert_eq!(
        NoteCitation::new(20, 10),
        Err(NoteError::InvalidCitationRange {
            start_micros: 20,
            end_micros: 10,
        })
    );

    let uncited = NoteSections {
        summary: NoteItem::new("An uncited final claim."),
        ..NoteSections::default()
    };
    assert_eq!(
        FinalNotes::from_sections(uncited, &segments()),
        Err(NoteError::MissingCitation {
            section: "summary",
            item_index: None,
        })
    );
}

#[test]
fn empty_final_notes_can_be_created_for_an_empty_transcript() {
    let final_notes = FinalNotes::from_sections(NoteSections::default(), &[]).unwrap();

    assert_eq!(final_notes.sections, NoteSections::default());
    assert!(final_notes.is_authoritative());
}
