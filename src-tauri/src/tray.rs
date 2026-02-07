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

const TRAY_ID: &str = "zureshot-tray";

/// Create a simple red circle icon for the tray (22x22 RGBA)
fn create_tray_icon(app: &AppHandle) -> Result<Image<'static>, Box<dyn std::error::Error>> {
    // Try to load from file first
    let icon_path = app.path().resource_dir()?.join("icons/tray.png");
    if icon_path.exists() {
        if let Ok(icon) = Image::from_path(&icon_path) {
            return Ok(icon);
        }
    }

    // Fallback: create a simple RGBA image with a red circle
    let size: usize = 22;
    let mut rgba = vec![0u8; size * size * 4];
    let center = size as f32 / 2.0;
    let radius = 8.0;

    for y in 0..size {
        for x in 0..size {
            let dx = x as f32 - center;
            let dy = y as f32 - center;
            let dist = (dx * dx + dy * dy).sqrt();
            let idx = (y * size + x) * 4;

            if dist <= radius {
                // Red circle (macOS-style)
                rgba[idx] = 255; // R
                rgba[idx + 1] = 59; // G
                rgba[idx + 2] = 48; // B
                rgba[idx + 3] = 255; // A
            }
            // else: already zeroed (transparent)
        }
    }

    Ok(Image::new_owned(rgba, size as u32, size as u32))
}

/// Build the tray menu with correct enabled/disabled states
fn build_menu(app: &AppHandle, is_recording: bool) -> Result<Menu<tauri::Wry>, Box<dyn std::error::Error>> {
    let start_recording = MenuItem::with_id(
        app,
        "start",
        "📹 Start Recording",
        !is_recording,
        Some("CmdOrCtrl+Shift+R"),
    )?;
    let stop_recording = MenuItem::with_id(
        app,
        "stop",
        "⏹ Stop Recording",
        is_recording,
        Some("CmdOrCtrl+Shift+S"),
    )?;
    let separator = MenuItem::with_id(app, "sep1", "────────────", false, None::<&str>)?;
    let open_recordings = MenuItem::with_id(
        app,
        "open_folder",
        "📂 Open Recordings Folder",
        true,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(app, "quit", "Quit Zureshot", true, Some("CmdOrCtrl+Q"))?;

    let menu = Menu::with_items(
        app,
        &[
            &start_recording,
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
    let icon = create_tray_icon(app)?;

    let _tray = TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon)
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

/// Handle menu item clicks
fn handle_menu_event(app: &AppHandle, id: &str) {
    match id {
        "start" => {
            // Spawn on background thread — capture::create_and_start blocks
            // waiting for ObjC completion handlers that may need the main run loop
            let app = app.clone();
            std::thread::spawn(move || {
                match commands::do_start_recording(&app, None) {
                    Ok(path) => {
                        println!("[zureshot] Started via menu: {}", path);
                        update_menu_state(&app, true);
                    }
                    Err(e) => eprintln!("[zureshot] Start error: {}", e),
                }
            });
        }
        "stop" => {
            // CRITICAL: Must run on background thread!
            // finishWritingWithCompletionHandler and stopCaptureWithCompletionHandler
            // dispatch their callbacks via GCD. If we block the main thread waiting
            // for these callbacks (via mpsc::channel), and GCD needs the main thread
            // to deliver them, we get a deadlock → moov atom never written.
            //
            // Immediately update menu to prevent double-click
            update_menu_state(app, false);
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
            let downloads = dirs::download_dir()
                .or_else(dirs::home_dir)
                .unwrap_or_else(|| std::path::PathBuf::from("."));
            let _ = std::process::Command::new("open").arg(&downloads).spawn();
        }
        "quit" => {
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
                    match commands::do_stop_recording(&app) {
                        Ok(r) => println!(
                            "[zureshot] Recording finalized before quit: {} ({:.1}s)",
                            r.path, r.duration_secs
                        ),
                        Err(e) => eprintln!("[zureshot] Error finalizing on quit: {}", e),
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
