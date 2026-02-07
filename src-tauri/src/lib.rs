// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![allow(non_snake_case)]

mod capture;
mod commands;
mod tray;
mod writer;

use commands::RecordingState;
use std::sync::Mutex;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            // Initialize recording state
            app.manage(Mutex::new(RecordingState::default()));

            // Setup tray icon
            tray::setup_tray(app.handle())?;

            // Hide the Dock icon — pure menu-bar app
            #[cfg(target_os = "macos")]
            {
                use tauri::ActivationPolicy;
                app.set_activation_policy(ActivationPolicy::Accessory);
            }

            println!("[zureshot] App started, tray icon ready");
            println!("[zureshot] Click tray icon or use menu to start recording");

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::start_recording,
            commands::stop_recording,
            commands::get_recording_status,
            commands::reveal_in_finder,
            commands::get_recordings_dir,
            commands::start_region_selection,
            commands::confirm_region_selection,
            commands::cancel_region_selection,
            commands::pause_recording,
            commands::resume_recording,
        ])
        // Tray-only app: use .build() + .run() to intercept ExitRequested.
        // This prevents Tauri from quitting when all windows are destroyed.
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app_handle, event| {
            if let tauri::RunEvent::ExitRequested { api, .. } = event {
                // Keep the app alive — only explicit app.exit(0) can quit
                api.prevent_exit();
            }
        });
}
