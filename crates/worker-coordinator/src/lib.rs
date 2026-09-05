#![forbid(unsafe_code)]

mod coordinator;
mod export;
mod notes;
mod process;
mod protocol;
mod reconcile;
mod windowing;
mod worker_config;

pub use coordinator::{
    CoordinatorError, JsonLinesWorker, LivePipeline, WorkerCoordinator, WorkerTransport,
    provisional_result,
};
pub use export::{export_files, json as notes_json, markdown as notes_markdown};
pub use notes::{
    Citation, DraftNotes, MeetingNotes, NoteItem, NoteVersion, NotesError, generate_final_notes,
};
pub use process::{
    WorkerProcess, bundled_worker_resource_root, bundled_worker_root, copy_worker_script,
    resolve_worker_config,
};
pub use protocol::{
    AlignmentStatus, AudioWindow, FinalTranscript, LanguageMode, PROTOCOL_VERSION, ProtocolError,
    TranscriptResult, TranscriptSegment, TranscriptWord, WorkerCapabilities, WorkerRequest,
    WorkerResponse, decode_request, decode_response, encode_request, encode_response,
};
pub use reconcile::{LiveTranscript, TranscriptReconciler, result_for};
pub use windowing::{RollingWindowBuffer, WindowConfig, WindowError};
pub use worker_config::{WorkerConfig, WorkerConfigError, WorkerLaunchError};
