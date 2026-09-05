#![forbid(unsafe_code)]

mod capture;
mod note_schema;
mod notes;
mod session;
mod status;

pub use capture::{CaptureConfig, LanguageMode, TrackRole};
pub use note_schema::{
    DraftNotes, FinalNotes, NoteCitation, NoteDocument, NoteError, NoteItem, NoteSections,
    NoteStatus, TranscriptSegmentRange,
};
pub use notes::{
    default_capture, draft_from_transcript, final_from_transcript, serialize_document,
};
pub use session::{LiveSession, RecordingArtifact};
pub use status::{Meeting, MeetingStatus, SessionState};
