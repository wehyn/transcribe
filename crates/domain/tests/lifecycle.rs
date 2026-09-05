use meeting_domain::{CaptureConfig, LanguageMode, LiveSession, SessionState};

#[test]
fn a_new_session_is_created_and_requires_consent_before_recording() {
    let config = CaptureConfig::dual_source(LanguageMode::English);
    let session = LiveSession::new("Weekly sync", config);

    assert_eq!(session.state, SessionState::Created);
    assert!(!session.consent_accepted);
    assert!(session.started_at.is_none());
    assert!(session.recordings.is_empty());
}

#[test]
fn lifecycle_accepts_only_explicit_record_pause_resume_and_stop_transitions() {
    assert!(SessionState::Created.can_transition_to(SessionState::Starting));
    assert!(!SessionState::Created.can_transition_to(SessionState::Listening));
    assert!(SessionState::Starting.can_transition_to(SessionState::Listening));
    assert!(SessionState::Listening.can_transition_to(SessionState::Paused));
    assert!(SessionState::Paused.can_transition_to(SessionState::Starting));
    assert!(SessionState::Listening.can_transition_to(SessionState::Stopping));
    assert!(SessionState::Stopping.can_transition_to(SessionState::Sealed));
    assert!(!SessionState::Sealed.can_transition_to(SessionState::Listening));
}

#[test]
fn taglish_is_a_first_class_language_mode() {
    let config = CaptureConfig::dual_source(LanguageMode::Taglish);
    assert_eq!(config.language, LanguageMode::Taglish);
}
