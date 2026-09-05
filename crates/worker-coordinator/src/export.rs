use std::path::Path;

use crate::{Citation, FinalTranscript, MeetingNotes, NoteItem, NoteVersion};

pub fn markdown(notes: &MeetingNotes, transcript: Option<&FinalTranscript>) -> String {
    let label = match notes.version {
        NoteVersion::Draft => "Draft",
        NoteVersion::Final => "Final",
    };
    let mut output = format!(
        "# Meeting Notes ({label})\n\n## Summary\n\n{}\n",
        notes.summary
    );
    append_items(&mut output, "Decisions", &notes.decisions, transcript);
    append_items(&mut output, "Action items", &notes.action_items, transcript);
    append_items(
        &mut output,
        "Open questions",
        &notes.open_questions,
        transcript,
    );
    output
}

pub fn json(notes: &MeetingNotes) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(notes)
}

pub fn export_files(
    notes: &MeetingNotes,
    transcript: Option<&FinalTranscript>,
    destination: impl AsRef<Path>,
) -> std::io::Result<()> {
    let destination = destination.as_ref();
    std::fs::create_dir_all(destination)?;
    std::fs::write(destination.join("notes.md"), markdown(notes, transcript))?;
    std::fs::write(
        destination.join("notes.json"),
        json(notes).map_err(std::io::Error::other)?,
    )?;
    if let Some(transcript) = transcript {
        std::fs::write(
            destination.join("transcript.json"),
            serde_json::to_string_pretty(transcript).map_err(std::io::Error::other)?,
        )?;
    }
    Ok(())
}

fn append_items(
    output: &mut String,
    heading: &str,
    items: &[NoteItem],
    transcript: Option<&FinalTranscript>,
) {
    output.push_str(&format!("\n## {heading}\n\n"));
    if items.is_empty() {
        output.push_str("_None recorded._\n");
        return;
    }
    for item in items {
        output.push_str(&format!(
            "- {}{}\n",
            item.text,
            citation_suffix(&item.citations, transcript)
        ));
    }
}

fn citation_suffix(citations: &[Citation], transcript: Option<&FinalTranscript>) -> String {
    citations
        .iter()
        .filter(|citation| {
            transcript.is_none_or(|transcript| {
                transcript.segments.iter().any(|segment| {
                    citation.start_micros <= segment.start_micros
                        && citation.end_micros >= segment.end_micros
                })
            })
        })
        .map(|citation| {
            format!(
                " [`{}–{}`]",
                format_time(citation.start_micros),
                format_time(citation.end_micros)
            )
        })
        .collect()
}

fn format_time(micros: u64) -> String {
    let seconds = micros / 1_000_000;
    format!("{:02}:{:02}", seconds / 60, seconds % 60)
}
