// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![allow(non_snake_case)]

mod commands;
mod platform;
mod tray;

use commands::{RecordingState, ScrollCaptureStateWrapper};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Manager;

/// When true, the app should actually exit (set by the quit menu handler).
pub static SHOULD_EXIT: AtomicBool = AtomicBool::new(false);

#[cfg(target_os = "macos")]
use platform::macos::camera::NativeCameraState;
#[cfg(target_os = "macos")]
use platform::macos::mouse_tracker::MouseTrackerState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            // Initialize recording state
            app.manage(Mutex::new(RecordingState::default()));

            // Initialize scroll capture state
            app.manage(Mutex::new(ScrollCaptureStateWrapper::default()));

            // Initialize native camera state
            #[cfg(target_os = "macos")]
            app.manage(Mutex::new(NativeCameraState::default()));

            // Initialize mouse tracker state for editor auto-zoom
            #[cfg(target_os = "macos")]
            app.manage(Mutex::new(MouseTrackerState::default()));

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

            // Auto-check for updates on startup
            tray::auto_check_update(app.handle());

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
            commands::start_screenshot_selection,
            commands::take_screenshot,
            commands::screenshot_to_clipboard,
            commands::copy_image_data_to_clipboard,
            commands::save_annotated_and_pin,
            commands::close_pin_window,
            commands::save_screenshot,
            commands::copy_screenshot,
            commands::dismiss_screenshot,
            commands::open_camera_overlay,
            commands::open_camera_overlay_with_options,
            commands::close_camera_overlay,
            commands::toggle_camera_overlay,
            commands::move_camera_overlay,
            commands::list_native_camera_devices,
            commands::start_native_camera,
            commands::stop_native_camera,
            // Pin + OCR commands
            commands::pin_screenshot,
            commands::ocr_screenshot,
            // Scroll capture commands
            commands::start_scroll_screenshot_selection,
            commands::start_scroll_capture,
            commands::scroll_capture_tick,
            commands::finish_scroll_capture,
            commands::cancel_scroll_capture,
            // Video editor commands
            commands::open_video_editor,
            commands::get_video_metadata,
            commands::generate_timeline_thumbnails,
            commands::generate_waveform,
            commands::trim_video,
            commands::render_preview_frame,
            commands::start_export,
            commands::get_mouse_track,
            commands::suggest_zoom_keyframes,
            commands::log_debug,
        ])
        // Tray-only app: use .build() + .run() to intercept ExitRequested.
        // This prevents Tauri from quitting when all windows are destroyed.
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app_handle, event| {
            if let tauri::RunEvent::ExitRequested { api, .. } = event {
                // Only prevent implicit exits (e.g. all windows closed).
                // When SHOULD_EXIT is true, let the app actually quit.
                if !SHOULD_EXIT.load(Ordering::SeqCst) {
                    api.prevent_exit();
                }
            }
        });
}
