//! Tray icon setup and menu handling for Zureshot.

use tauri::{
    image::Image,
    menu::{CheckMenuItem, Menu, MenuItem},
    tray::TrayIconBuilder,
    AppHandle, Emitter, Manager,
};
use tauri_plugin_updater::UpdaterExt;

use crate::commands;
use crate::commands::RecordingState;
use std::path::PathBuf;
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

// ── Settings persistence ──────────────────────────────────────────────

fn settings_path(app: &AppHandle) -> PathBuf {
    let dir = app
        .path()
        .app_config_dir()
        .unwrap_or_else(|_| PathBuf::from("."));
    let _ = std::fs::create_dir_all(&dir);
    dir.join("settings.json")
}

fn get_auto_update_enabled(app: &AppHandle) -> bool {
    let path = settings_path(app);
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v["auto_update"].as_bool())
        .unwrap_or(true)
}

fn set_auto_update_enabled(app: &AppHandle, enabled: bool) {
    let path = settings_path(app);
    let mut settings: serde_json::Value = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    settings["auto_update"] = serde_json::json!(enabled);
    let _ = std::fs::write(&path, serde_json::to_string_pretty(&settings).unwrap());
}

// ── Native macOS dialogs ─────────────────────────────────────────────

fn show_confirm_dialog(title: &str, message: &str, accept: &str, cancel: &str) -> bool {
    let script = format!(
        "display dialog \"{}\" buttons {{\"{}\" , \"{}\"}} default button \"{}\" with title \"{}\"",
        message, cancel, accept, accept, title
    );
    std::process::Command::new("osascript")
        .args(["-e", &script])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains(accept))
        .unwrap_or(false)
}

fn show_info_dialog(title: &str, message: &str) {
    let script = format!(
        "display dialog \"{}\" buttons {{\"OK\"}} default button \"OK\" with title \"{}\"",
        message, title
    );
    let _ = std::process::Command::new("osascript")
        .args(["-e", &script])
        .output();
}

// ── Tray menu ────────────────────────────────────────────────────────

/// Build the tray menu with correct enabled/disabled states
fn build_menu(app: &AppHandle, is_recording: bool) -> Result<Menu<tauri::Wry>, Box<dyn std::error::Error>> {
    let screenshot_region = MenuItem::with_id(
        app,
        "screenshot_region",
        "Screenshot Region",
        !is_recording,
        Some("CmdOrCtrl+Shift+A"),
    )?;
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
    let check_update = MenuItem::with_id(
        app,
        "check_update",
        "Check for Updates…",
        true,
        None::<&str>,
    )?;
    let auto_update = CheckMenuItem::with_id(
        app,
        "auto_update",
        "Auto Check for Updates",
        true,
        get_auto_update_enabled(app),
        None::<&str>,
    )?;
    let separator2 = MenuItem::with_id(app, "sep2", "────────────", false, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Zureshot", true, Some("CmdOrCtrl+Q"))?;

    let menu = Menu::with_items(
        app,
        &[
            &screenshot_region,
            &record_region,
            &stop_recording,
            &separator,
            &open_recordings,
            &check_update,
            &auto_update,
            &separator2,
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

/// Check for updates.
///
/// * `interactive` – `true` when the user clicks "Check for Updates\u2026".
///   Shows a dialog regardless of outcome (update found *or* already up to
///   date). In non-interactive mode (startup auto-check) we only prompt
///   when an update is actually available.
async fn check_for_updates(
    app: &AppHandle,
    interactive: bool,
) -> Result<Option<String>, Box<dyn std::error::Error>> {
    let updater = app.updater_builder().build()?;

    match updater.check().await? {
        Some(update) => {
            let version = update.version.clone();
            println!("[zureshot] Update found: v{}", version);
            let _ = app.emit("update-available", &version);

            // Ask the user whether they want to install now
            let ver = version.clone();
            let confirmed = tokio::task::spawn_blocking(move || {
                show_confirm_dialog(
                    "Zureshot Update",
                    &format!(
                        "A new version (v{}) is available. Update now?",
                        ver
                    ),
                    "Update Now",
                    "Later",
                )
            })
            .await
            .unwrap_or(false);

            if !confirmed {
                println!("[zureshot] User declined update v{}", version);
                return Ok(Some(version));
            }

            // Download and install
            let mut downloaded: u64 = 0;
            update
                .download_and_install(
                    |chunk_length, content_length| {
                        downloaded += chunk_length as u64;
                        let progress = content_length
                            .map(|total| (downloaded as f64 / total as f64 * 100.0) as u32)
                            .unwrap_or(0);
                        println!("[zureshot] Downloading update: {}%", progress);
                    },
                    || {
                        println!("[zureshot] Download complete, installing...");
                    },
                )
                .await?;

            println!("[zureshot] Update installed, restarting...");
            app.restart();
        }
        None => {
            println!("[zureshot] App is up to date");
            let _ = app.emit("update-not-available", ());

            if interactive {
                tokio::task::spawn_blocking(|| {
                    show_info_dialog(
                        "Zureshot Update",
                        "You're running the latest version.",
                    );
                })
                .await
                .ok();
            }

            Ok(None)
        }
    }
}

/// Auto-check for updates on app startup (respects user preference).
pub fn auto_check_update(app: &AppHandle) {
    if !get_auto_update_enabled(app) {
        println!("[zureshot] Auto-update check disabled by user");
        return;
    }

    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        // Wait 5 seconds after startup before checking
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        match check_for_updates(&app, false).await {
            Ok(Some(v)) => println!("[zureshot] Auto-check: update v{} available", v),
            Ok(None) => println!("[zureshot] Auto-check: up to date"),
            Err(e) => eprintln!("[zureshot] Auto-update check failed: {}", e),
        }
    });
}

/// Handle menu item clicks
fn handle_menu_event(app: &AppHandle, id: &str) {
    match id {
        "screenshot_region" => {
            match commands::do_start_screenshot_selection(app) {
                Ok(()) => println!("[zureshot] Screenshot region selector opened via menu"),
                Err(e) => eprintln!("[zureshot] Screenshot region selection error: {}", e),
            }
        }
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
        "check_update" => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                match check_for_updates(&app, true).await {
                    Ok(Some(v)) => println!("[zureshot] Check result: v{}", v),
                    Ok(None) => println!("[zureshot] Already up to date"),
                    Err(e) => {
                        eprintln!("[zureshot] Update check failed: {}", e);
                        let msg = format!("Failed to check for updates: {}", e);
                        tokio::task::spawn_blocking(move || {
                            show_info_dialog("Zureshot Update", &msg);
                        });
                    }
                }
            });
        }
        "auto_update" => {
            let current = get_auto_update_enabled(app);
            let new_val = !current;
            set_auto_update_enabled(app, new_val);
            println!(
                "[zureshot] Auto-update check {}",
                if new_val { "enabled" } else { "disabled" }
            );
            // Rebuild menu so the checkmark reflects the new state
            let is_recording = {
                let state = app.state::<Mutex<RecordingState>>();
                state.lock().map(|r| r.is_recording).unwrap_or(false)
            };
            update_menu_state(app, is_recording);
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
