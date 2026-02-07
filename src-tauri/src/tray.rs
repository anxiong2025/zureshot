//! Tray icon setup and menu handling for Zureshot.

use tauri::{
    image::Image,
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    AppHandle, Manager,
};

use crate::commands;
use crate::commands::RecordingState;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

const TRAY_ID: &str = "zureshot-tray";

/// Guard against double-quit: once set, further quit requests are ignored.
static QUITTING: AtomicBool = AtomicBool::new(false);

/// Load tray icon from bundled resources
fn load_tray_icon(app: &AppHandle, recording: bool) -> Result<Image<'static>, Box<dyn std::error::Error>> {
    let filename = if recording { "tray-recording.png" } else { "tray.png" };

    // Try resource dir first (bundled app)
    if let Ok(res_dir) = app.path().resource_dir() {
        let icon_path = res_dir.join("icons").join(filename);
        if icon_path.exists() {
            if let Ok(icon) = Image::from_path(&icon_path) {
                return Ok(icon);
            }
        }
    }

    // Fallback: try relative to executable (dev mode)
    let dev_path = std::path::PathBuf::from("icons").join(filename);
    if dev_path.exists() {
        if let Ok(icon) = Image::from_path(&dev_path) {
            return Ok(icon);
        }
    }

    // Final fallback: create a simple monochrome camera shape (22x22)
    let size: usize = 22;
    let mut rgba = vec![0u8; size * size * 4];
    // Simple filled rectangle as camera body
    for y in 6..16 {
        for x in 3..15 {
            let idx = (y * size + x) * 4;
            rgba[idx] = 0;
            rgba[idx + 1] = 0;
            rgba[idx + 2] = 0;
            rgba[idx + 3] = 255;
        }
    }
    // Lens triangle
    for y in 7..15 {
        let x_start = 16;
        let x_end = 16 + (4 - (y as i32 - 11).abs()) as usize;
        for x in x_start..x_end.min(size) {
            let idx = (y * size + x) * 4;
            rgba[idx] = 0;
            rgba[idx + 1] = 0;
            rgba[idx + 2] = 0;
            rgba[idx + 3] = 255;
        }
    }
    Ok(Image::new_owned(rgba, size as u32, size as u32))
}

/// Switch tray icon between normal and recording states
fn update_tray_icon(app: &AppHandle, recording: bool) {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        match load_tray_icon(app, recording) {
            Ok(icon) => {
                let _ = tray.set_icon(Some(icon));
            }
            Err(e) => eprintln!("[zureshot] Failed to update tray icon: {}", e),
        }
    }
}

/// Build the tray menu with correct enabled/disabled states
fn build_menu(app: &AppHandle, is_recording: bool) -> Result<Menu<tauri::Wry>, Box<dyn std::error::Error>> {
    let record_region = MenuItem::with_id(
        app,
        "record_region",
        "Record Region",
        !is_recording,
        Some("CmdOrCtrl+Shift+R"),
    )?;
    let stop_recording = MenuItem::with_id(
        app,
        "stop",
        "Stop Recording",
        is_recording,
        Some("CmdOrCtrl+Shift+S"),
    )?;
    let separator = MenuItem::with_id(app, "sep1", "────────────", false, None::<&str>)?;
    let open_recordings = MenuItem::with_id(
        app,
        "open_folder",
        "Open Recordings Folder",
        true,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(app, "quit", "Quit Zureshot", true, Some("CmdOrCtrl+Q"))?;

    let menu = Menu::with_items(
        app,
        &[
            &record_region,
            &stop_recording,
            &separator,
            &open_recordings,
            &quit,
        ],
    )?;
    Ok(menu)
}

/// Setup the system tray icon and menu
pub fn setup_tray(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let menu = build_menu(app, false)?;
    let icon = load_tray_icon(app, false)?;

    let _tray = TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon)
        .icon_as_template(false)
        .menu(&menu)
        .show_menu_on_left_click(true)
        .tooltip("Zureshot - Screen Recorder")
        .on_menu_event(move |app, event| {
            handle_menu_event(app, event.id.as_ref());
        })
        .build(app)?;

    Ok(())
}

/// Rebuild the tray menu to reflect current recording state
fn update_menu_state(app: &AppHandle, is_recording: bool) {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        match build_menu(app, is_recording) {
            Ok(menu) => {
                if let Err(e) = tray.set_menu(Some(menu)) {
                    eprintln!("[zureshot] Failed to update tray menu: {}", e);
                }
                // Update tooltip to show state
                let tooltip = if is_recording {
                    "Zureshot - 🔴 Recording..."
                } else {
                    "Zureshot - Screen Recorder"
                };
                let _ = tray.set_tooltip(Some(tooltip));
            }
            Err(e) => eprintln!("[zureshot] Failed to build menu: {}", e),
        }
    }
}

/// Called from commands.rs when recording stops (e.g. via the recording bar).
/// Resets tray icon and menu to idle state.
pub fn notify_recording_stopped(app: &AppHandle) {
    update_menu_state(app, false);
    update_tray_icon(app, false);
}

/// Called from commands.rs when recording starts.
/// Switches tray icon to recording state (red dot) and enables Stop menu.
pub fn notify_recording_started(app: &AppHandle) {
    update_menu_state(app, true);
    update_tray_icon(app, true);
}

/// Handle menu item clicks
fn handle_menu_event(app: &AppHandle, id: &str) {
    match id {
        "record_region" => {
            match commands::do_start_region_selection(app) {
                Ok(()) => println!("[zureshot] Region selector opened via menu"),
                Err(e) => eprintln!("[zureshot] Region selection error: {}", e),
            }
        }
        "stop" => {
            // CRITICAL: Must run on background thread!
            // finishWritingWithCompletionHandler and stopCaptureWithCompletionHandler
            // dispatch their callbacks via GCD. If we block the main thread waiting
            // for these callbacks (via mpsc::channel), and GCD needs the main thread
            // to deliver them, we get a deadlock → moov atom never written.
            //
            // Immediately update menu and icon to prevent double-click
            update_menu_state(app, false);
            update_tray_icon(app, false);
            let app = app.clone();
            std::thread::spawn(move || {
                match commands::do_stop_recording(&app) {
                    Ok(result) => {
                        println!(
                            "[zureshot] Stopped via menu: {} ({:.1}s, {:.1} MB)",
                            result.path,
                            result.duration_secs,
                            result.file_size_bytes as f64 / 1_048_576.0
                        );
                    }
                    Err(e) => eprintln!("[zureshot] Stop error: {}", e),
                }
            });
        }
        "open_folder" => {
            let base = dirs::download_dir()
                .or_else(dirs::home_dir)
                .unwrap_or_else(|| std::path::PathBuf::from("."));
            let zureshot_dir = base.join("Zureshot");
            let _ = std::fs::create_dir_all(&zureshot_dir);
            let _ = std::process::Command::new("open").arg(&zureshot_dir).spawn();
        }
        "quit" => {
            // Guard: ignore if already quitting (prevents double-click race)
            if QUITTING.swap(true, Ordering::SeqCst) {
                return; // Already in quit sequence
            }

            // Close any open windows (region-selector, recording-bar, overlay)
            // before starting the quit sequence. These windows don't hold
            // critical state — they just need to be torn down cleanly.
            for label in ["region-selector", "recording-bar", "recording-overlay"] {
                if let Some(win) = app.get_webview_window(label) {
                    let _ = win.destroy();
                }
            }

            // If recording is in progress, finalize it before quitting.
            // Must run on a background thread — GCD completion handlers
            // may need the main run loop to deliver callbacks.
            let is_recording = {
                let state = app.state::<Mutex<RecordingState>>();
                state.lock().map(|r| r.is_recording).unwrap_or(false)
            };
            if is_recording {
                let app = app.clone();
                std::thread::spawn(move || {
                    // Use catch_unwind so app.exit(0) always runs even if
                    // do_stop_recording panics (e.g. mutex poisoned).
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        commands::do_stop_recording(&app)
                    }));
                    match result {
                        Ok(Ok(r)) => println!(
                            "[zureshot] Recording finalized before quit: {} ({:.1}s)",
                            r.path, r.duration_secs
                        ),
                        Ok(Err(e)) => eprintln!("[zureshot] Error finalizing on quit: {}", e),
                        Err(_) => eprintln!("[zureshot] PANIC during finalize on quit"),
                    }
                    app.exit(0);
                });
            } else {
                app.exit(0);
            }
        }
        _ => {}
    }
}
