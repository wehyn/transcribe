#![forbid(unsafe_code)]

mod commands;

fn main() {
    tauri::Builder::default()
        .manage(std::sync::Mutex::new(commands::DesktopState::default()))
        .invoke_handler(tauri::generate_handler![
            commands::capabilities,
            commands::create_session,
            commands::accept_consent,
            commands::record,
            commands::pause,
            commands::resume,
            commands::stop,
            commands::export_meeting,
            commands::delete_meeting,
            commands::shutdown,
            commands::session_state,
            commands::model_status,
            commands::model_manifest,
            commands::model_recover,
            commands::download_model,
            commands::cancel_model_download,
            commands::remove_model,
        ])
        .on_window_event(|window, event| {
            if matches!(event, tauri::WindowEvent::CloseRequested { .. }) {
                let _ = window.emit("session-close-requested", ());
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running Meeting Notes");
}
