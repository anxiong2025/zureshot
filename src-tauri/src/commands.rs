//! Tauri commands for screen recording.
//!
//! These functions are exposed to the frontend via Tauri's IPC mechanism.

use crate::platform;
use crate::platform::{CaptureRegion, RecordingQuality, StartRecordingConfig};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, WebviewWindowBuilder, WebviewUrl};

/// Recording state shared across commands
pub struct RecordingState {
    /// Platform-specific recording handle (owns stream, encoder, etc.)
    pub handle: Option<platform::imp::RecordingHandle>,
    pub output_path: Option<String>,
    pub is_recording: bool,
    pub is_paused: bool,
    pub start_time: Option<std::time::Instant>,
    /// Accumulated pause duration (subtracted from wall-clock elapsed)
    pub pause_accumulated: std::time::Duration,
    /// When the current pause started (None if not paused)
    pub pause_start: Option<std::time::Instant>,
    pub region: Option<CaptureRegion>,
    pub quality: RecordingQuality,
    /// Output format: "video" (MP4) or "gif" (record MP4, convert to GIF on stop)
    pub output_format: String,
}

impl Default for RecordingState {
    fn default() -> Self {
        Self {
            handle: None,
            output_path: None,
            is_recording: false,
            is_paused: false,
            start_time: None,
            pause_accumulated: std::time::Duration::ZERO,
            pause_start: None,
            region: None,
            quality: RecordingQuality::Standard,
            output_format: "video".to_string(),
        }
    }
}

// SAFETY: RecordingState contains platform-specific objects that are thread-safe.
// We wrap it in a Mutex for interior mutability.
unsafe impl Send for RecordingState {}
unsafe impl Sync for RecordingState {}

/// Recording status sent to frontend
#[derive(Clone, Serialize, Deserialize)]
pub struct RecordingStatus {
    pub is_recording: bool,
    pub is_paused: bool,
    pub duration_secs: f64,
    pub output_path: Option<String>,
    pub quality: String,
}

/// Result of stopping a recording
#[derive(Clone, Serialize, Deserialize)]
pub struct RecordingResult {
    pub path: String,
    pub duration_secs: f64,
    pub file_size_bytes: u64,
}

/// GIF recording constraints (industry standard, matching CleanShot X)
const GIF_MAX_DURATION_SECS: f64 = 30.0;
const GIF_MAX_WIDTH: usize = 640;
const GIF_FPS: u32 = 15;

/// Payload emitted with `recording-started` event
#[derive(Clone, Serialize, Deserialize)]
pub struct RecordingStartedPayload {
    pub path: String,
    pub region: Option<CaptureRegion>,
    pub format: String,
    /// Max duration in seconds (0 = unlimited)
    pub max_duration: f64,
}

/// Core logic to start recording (called from both tray and commands)
pub fn do_start_recording(
    app: &AppHandle,
    output_path: Option<String>,
    region: Option<CaptureRegion>,
    quality: RecordingQuality,
    capture_system_audio: bool,
    capture_microphone: bool,
    output_format: Option<String>,
) -> Result<String, String> {
    let state: tauri::State<'_, Mutex<RecordingState>> = app.state();
    let mut recording = state.lock().map_err(|e: std::sync::PoisonError<_>| e.to_string())?;

    if recording.is_recording {
        return Err("Recording already in progress".to_string());
    }

    // Generate output path if not provided
    let path = output_path.unwrap_or_else(|| {
        let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
        let base = dirs::download_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        let zureshot_dir = base.join("Zureshot");
        let _ = std::fs::create_dir_all(&zureshot_dir);
        zureshot_dir
            .join(format!("zureshot_{}.mp4", timestamp))
            .to_string_lossy()
            .to_string()
    });

    println!("[zureshot] Starting recording to: {}", path);

    // Delegate all platform-specific setup to the platform layer
    let config = StartRecordingConfig {
        output_path: path.clone(),
        region: region.clone(),
        quality,
        capture_system_audio,
        capture_microphone,
    };
    let handle = platform::imp::start_recording(app, config)?;

    // Update state
    recording.handle = Some(handle);
    recording.output_path = Some(path.clone());
    recording.is_recording = true;
    recording.is_paused = false;
    recording.start_time = Some(std::time::Instant::now());
    recording.pause_accumulated = std::time::Duration::ZERO;
    recording.pause_start = None;
    recording.region = region.clone();
    recording.quality = quality;
    recording.output_format = output_format.unwrap_or_else(|| "video".to_string());

    // Start mouse tracking for editor auto-zoom (macOS only)
    #[cfg(target_os = "macos")]
    {
        if let Some(tracker_state) = app.try_state::<Mutex<platform::macos::mouse_tracker::MouseTrackerState>>() {
            if let Ok(tracker) = tracker_state.lock() {
                platform::macos::mouse_tracker::start_mouse_tracking(&tracker);
                println!("[zureshot] Mouse tracking started for auto-zoom");
            }
        }
    }

    // Switch tray icon to recording state (red dot + Stop enabled)
    crate::tray::notify_recording_started(app);

    // Emit event to frontend with region info and format
    let fmt = recording.output_format.clone();
    let max_dur = if fmt == "gif" { GIF_MAX_DURATION_SECS } else { 0.0 };
    let payload = RecordingStartedPayload {
        path: path.clone(),
        region,
        format: fmt,
        max_duration: max_dur,
    };
    let _ = app.emit("recording-started", &payload);

    Ok(path)
}

/// Core logic to stop recording (called from both tray and commands)
pub fn do_stop_recording(app: &AppHandle) -> Result<RecordingResult, String> {
    // Extract all recording state while holding the mutex, then release it
    // BEFORE any blocking operations.
    let (handle, output_path, duration, output_format) = {
        let state: tauri::State<'_, Mutex<RecordingState>> = app.state();
        let mut recording = state.lock().map_err(|e: std::sync::PoisonError<_>| e.to_string())?;

        if !recording.is_recording {
            return Err("No recording in progress".to_string());
        }

        let duration = recording
            .start_time
            .map(|t: std::time::Instant| {
                let wall = t.elapsed();
                let paused = recording.pause_accumulated
                    + recording.pause_start.map(|ps| ps.elapsed()).unwrap_or_default();
                (wall - paused).as_secs_f64()
            })
            .unwrap_or(0.0);

        let handle = recording.handle.take();
        let output_path = recording.output_path.take().unwrap_or_default();
        let output_format = std::mem::replace(&mut recording.output_format, "video".to_string());
        recording.is_recording = false;
        recording.is_paused = false;
        recording.start_time = None;
        recording.pause_accumulated = std::time::Duration::ZERO;
        recording.pause_start = None;
        recording.region = None;
        recording.quality = RecordingQuality::Standard;

        (handle, output_path, duration, output_format)
    }; // ← mutex released here

    println!("[zureshot] Stopping recording after {:.1}s", duration);

    // Stop mouse tracking and save track data
    #[cfg(target_os = "macos")]
    {
        if let Some(tracker_state) = app.try_state::<Mutex<platform::macos::mouse_tracker::MouseTrackerState>>() {
            if let Ok(tracker) = tracker_state.lock() {
                let track = platform::macos::mouse_tracker::stop_mouse_tracking(&tracker);
                if !track.samples.is_empty() {
                    let _ = platform::macos::mouse_tracker::save_mouse_track(&output_path, &track);
                }
            }
        }
    }

    // Close the recording bar and dim overlay windows
    if let Some(win) = app.get_webview_window("recording-bar") {
        let _ = win.destroy();
    }
    if let Some(win) = app.get_webview_window("recording-overlay") {
        let _ = win.destroy();
    }
    // Close camera bubble if open
    if let Some(win) = app.get_webview_window("camera-overlay") {
        let _ = app.emit("camera-overlay-close", ());
        let _ = win.destroy();
    }

    // Stop capture and finalize file (platform-specific)
    if let Some(ref handle) = handle {
        handle.stop_capture();
    }

    // Brief pause to let the capture pipeline fully drain
    std::thread::sleep(std::time::Duration::from_millis(200));

    // Finalize output file
    if let Some(ref handle) = handle {
        handle.finalize();
    }

    // If format is GIF, convert MP4 → GIF using ffmpeg with palette optimization
    let final_path = if output_format == "gif" {
        let gif_path = output_path.replace(".mp4", ".gif");
        println!("[zureshot] Converting MP4 to GIF: {} → {}", output_path, gif_path);

        // Two-pass palette-optimized GIF for high quality:
        // - Cap width at 640px (scale down large regions for reasonable file size)
        // - fps=15 balances file size and smoothness
        // - lanczos scaling for sharpness
        // - palettegen+paletteuse for optimal color dithering
        let vf = format!(
            "fps={},scale='min({},iw)':-1:flags=lanczos,split[s0][s1];[s0]palettegen[p];[s1][p]paletteuse",
            GIF_FPS, GIF_MAX_WIDTH
        );
        let ffmpeg_result = std::process::Command::new("ffmpeg")
            .args([
                "-i", &output_path,
                "-t", &format!("{}", GIF_MAX_DURATION_SECS),
                "-vf", &vf,
                "-y",
                &gif_path,
            ])
            .output();

        match ffmpeg_result {
            Ok(output) if output.status.success() => {
                println!("[zureshot] GIF conversion successful");
                // Delete the temporary MP4
                let _ = std::fs::remove_file(&output_path);
                gif_path
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                eprintln!("[zureshot] ffmpeg conversion failed: {}", stderr);
                // Keep the MP4 as fallback
                output_path
            }
            Err(e) => {
                eprintln!("[zureshot] ffmpeg not found or failed to run: {}", e);
                eprintln!("[zureshot] Install ffmpeg with: brew install ffmpeg");
                // Keep the MP4 as fallback
                output_path
            }
        }
    } else {
        output_path
    };

    let file_size = std::fs::metadata(&final_path).map(|m| m.len()).unwrap_or(0);

    let result = RecordingResult {
        path: final_path.clone(),
        duration_secs: duration,
        file_size_bytes: file_size,
    };

    // Emit event to frontend with result
    let _ = app.emit("recording-stopped", &result);

    println!(
        "[zureshot] Recording complete: {} ({:.1}s, {:.1} MB)",
        final_path,
        duration,
        file_size as f64 / 1_048_576.0
    );

    // Update tray menu to reflect stopped state
    // (handles case where stop was triggered from recording bar, not tray)
    crate::tray::notify_recording_stopped(app);

    // Auto-open video editor for MP4 recordings (not GIF)
    if output_format != "gif" {
        let app_clone = app.clone();
        let path_clone = final_path.clone();
        // Slight delay to let the UI settle before opening editor
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(500));
            if let Err(e) = do_open_video_editor(&app_clone, &path_clone) {
                eprintln!("[zureshot] Failed to open video editor: {}", e);
            }
        });
    }

    Ok(result)
}

/// Start screen recording (Tauri command - called from frontend)
#[tauri::command]
pub async fn start_recording(
    app: AppHandle,
    _state: tauri::State<'_, Mutex<RecordingState>>,
    output_path: Option<String>,
) -> Result<(), String> {
    // CRITICAL: Must run on a dedicated OS thread, not the Tokio async runtime.
    // do_start_recording() blocks on GCD completion handlers via mpsc::channel.
    // Running this on a Tokio worker thread can deadlock because GCD may not
    // deliver callbacks to Tokio-managed threads on macOS.
    let app_clone = app.clone();
    tokio::task::spawn_blocking(move || {
        do_start_recording(&app_clone, output_path, None, RecordingQuality::Standard, false, false, None).map(|_| ())
    })
    .await
    .map_err(|e| format!("Task join error: {e}"))?
}

/// Stop screen recording (Tauri command - called from frontend)
#[tauri::command]
pub async fn stop_recording(
    app: AppHandle,
    _state: tauri::State<'_, Mutex<RecordingState>>,
) -> Result<RecordingResult, String> {
    // CRITICAL: Must run on a dedicated OS thread, not the Tokio async runtime.
    // do_stop_recording() blocks on GCD completion handlers (finishWriting,
    // stopCapture) via mpsc::channel. If this runs on a Tokio worker thread,
    // GCD may not deliver callbacks → deadlock → moov atom never written
    // → unplayable MP4.
    let app_clone = app.clone();
    tokio::task::spawn_blocking(move || do_stop_recording(&app_clone))
        .await
        .map_err(|e| format!("Task join error: {e}"))?
}

/// Get current recording status
#[tauri::command]
pub fn get_recording_status(
    state: tauri::State<'_, Mutex<RecordingState>>,
) -> Result<RecordingStatus, String> {
    let recording = state.lock().map_err(|e| e.to_string())?;

    Ok(RecordingStatus {
        is_recording: recording.is_recording,
        is_paused: recording.is_paused,
        duration_secs: recording
            .start_time
            .map(|t| {
                let wall = t.elapsed();
                let paused = recording.pause_accumulated
                    + recording.pause_start.map(|ps| ps.elapsed()).unwrap_or_default();
                (wall - paused).as_secs_f64()
            })
            .unwrap_or(0.0),
        output_path: recording.output_path.clone(),
        quality: format!("{:?}", recording.quality),
    })
}

/// Open the recorded file in the system file manager
#[tauri::command]
pub async fn reveal_in_finder(path: String) -> Result<(), String> {
    platform::imp::reveal_file(&path)
}

/// Get the default recordings directory
#[tauri::command]
pub fn get_recordings_dir() -> String {
    let base = dirs::download_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let zureshot_dir = base.join("Zureshot");
    let _ = std::fs::create_dir_all(&zureshot_dir);
    zureshot_dir.to_string_lossy().to_string()
}

/// Core logic to open the region selector overlay (callable from both tray and commands)
pub fn do_start_region_selection(app: &AppHandle) -> Result<(), String> {
    // Check if already recording
    {
        let state: tauri::State<'_, Mutex<RecordingState>> = app.state();
        let recording = state.lock().map_err(|e| e.to_string())?;
        if recording.is_recording {
            return Err("Recording already in progress".to_string());
        }
    }

    // If a region-selector window already exists, just show + focus it
    if let Some(win) = app.get_webview_window("region-selector") {
        let _ = win.show();
        let _ = win.set_focus();
        return Ok(());
    }

    // Get display info for sizing the overlay
    let monitor = app
        .primary_monitor()
        .map_err(|e| format!("Failed to get monitor: {}", e))?
        .ok_or("No primary monitor found")?;
    let phys_size = monitor.size();
    let scale = monitor.scale_factor();
    let position = monitor.position();

    // Tauri's inner_size() expects LOGICAL pixels, not physical.
    // On a Retina display (scale=2.0), physical 2880×1800 → logical 1440×900.
    let logical_w = phys_size.width as f64 / scale;
    let logical_h = phys_size.height as f64 / scale;

    println!(
        "[zureshot] Region selector: physical={}×{}, scale={}, logical={:.0}×{:.0}, pos=({},{})",
        phys_size.width, phys_size.height, scale, logical_w, logical_h,
        position.x, position.y
    );

    // Create fullscreen transparent overlay window.
    // transparent(true) is safe now that the Svelte component mounts correctly
    // (the earlier invisibility was caused by an SSR resolution bug, not by
    // the transparency itself).
    let window = WebviewWindowBuilder::new(
        app,
        "region-selector",
        WebviewUrl::App("region-selector.html".into()),
    )
    .title("Region Selector")
    .inner_size(logical_w, logical_h)
    .position(position.x as f64 / scale, position.y as f64 / scale)
    .transparent(true)
    .decorations(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .resizable(false)
    .build()
    .map_err(|e| format!("Failed to create region selector window: {}", e))?;

    // Explicitly show and focus — tray-only macOS apps may not auto-focus new windows
    let _ = window.show();
    let _ = window.set_focus();

    println!("[zureshot] Region selector window created and focused");

    Ok(())
}

/// Open the region selector overlay window (Tauri command)
#[tauri::command]
pub async fn start_region_selection(app: AppHandle) -> Result<(), String> {
    do_start_region_selection(&app)
}

/// Confirm region selection and start recording with the selected region
#[tauri::command]
pub fn confirm_region_selection(
    app: AppHandle,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    quality: Option<String>,
    system_audio: Option<bool>,
    microphone: Option<bool>,
    format: Option<String>,
    camera: Option<bool>,
    camera_device_id: Option<String>,
    camera_shape: Option<String>,
    camera_size: Option<String>,
) -> Result<(), String> {
    // Hide the region selector (don't destroy — we're inside its IPC call).
    if let Some(win) = app.get_webview_window("region-selector") {
        let _ = win.hide();
    }

    let region = CaptureRegion {
        x,
        y,
        width,
        height,
    };

    let q = match quality.as_deref() {
        Some("high") => RecordingQuality::High,
        _ => RecordingQuality::Standard,
    };

    let sys_audio = system_audio.unwrap_or(false);
    let mic = microphone.unwrap_or(false);
    let output_format = format.unwrap_or_else(|| "video".to_string());
    let camera_enabled = camera.unwrap_or(false);
    let cam_device_id = camera_device_id.clone();
    let cam_shape = camera_shape.unwrap_or_else(|| "circle".to_string());
    let cam_size = camera_size.unwrap_or_else(|| "medium".to_string());

    let region_for_bar = region.clone();
    let region_for_overlay = region.clone();
    let app_clone = app.clone();
    std::thread::spawn(move || {
        // Small delay to let the region selector fully disappear
        std::thread::sleep(std::time::Duration::from_millis(300));

        // Now safe to destroy region-selector
        if let Some(win) = app_clone.get_webview_window("region-selector") {
            let _ = win.destroy();
        }

        match do_start_recording(&app_clone, None, Some(region), q, sys_audio, mic, Some(output_format)) {
            Ok(_) => {
                // Open the dim overlay and floating control bar
                let _ = do_open_recording_overlay(&app_clone, &region_for_overlay);
                let _ = do_open_recording_bar(&app_clone, Some(&region_for_bar));

                // Open camera bubble if user enabled it
                if camera_enabled {
                    let _ = do_open_camera_overlay_with_options(
                        &app_clone,
                        &cam_shape,
                        &cam_size,
                        cam_device_id.as_deref(),
                        Some(&region_for_overlay),
                    );
                }

                // Brief delay for windows to register with WindowServer,
                // then refresh the stream filter to exclude them from capture
                std::thread::sleep(std::time::Duration::from_millis(150));
                let _ = refresh_stream_exclusion(&app_clone);

                // Send region coordinates to the overlay for the dim effect
                let _ = app_clone.emit("recording-region", &region_for_overlay);
            }
            Err(e) => eprintln!("[zureshot] Start error: {}", e),
        }
    });

    Ok(())
}

/// Cancel region selection without starting recording
#[tauri::command]
pub async fn cancel_region_selection(app: AppHandle) -> Result<(), String> {
    if let Some(win) = app.get_webview_window("region-selector") {
        let _ = win.destroy();
    }
    // Clean up frozen screen preview if exists
    let preview_path = std::env::temp_dir().join("zureshot_screen_preview.png");
    let _ = std::fs::remove_file(&preview_path);
    Ok(())
}

/// Pause the current recording (frames will be dropped, timer pauses)
#[tauri::command]
pub fn pause_recording(
    state: tauri::State<'_, Mutex<RecordingState>>,
) -> Result<(), String> {
    let mut recording = state.lock().map_err(|e| e.to_string())?;

    if !recording.is_recording {
        return Err("No recording in progress".to_string());
    }
    if recording.is_paused {
        return Err("Recording is already paused".to_string());
    }

    // Set the atomic flag so the capture delegate drops frames
    if let Some(ref handle) = recording.handle {
        handle.pause();
    }

    recording.is_paused = true;
    recording.pause_start = Some(std::time::Instant::now());
    println!("[zureshot] Recording paused");
    Ok(())
}

/// Resume a paused recording
#[tauri::command]
pub fn resume_recording(
    state: tauri::State<'_, Mutex<RecordingState>>,
) -> Result<(), String> {
    let mut recording = state.lock().map_err(|e| e.to_string())?;

    if !recording.is_recording {
        return Err("No recording in progress".to_string());
    }
    if !recording.is_paused {
        return Err("Recording is not paused".to_string());
    }

    // Clear the atomic flag so frames start being written again
    if let Some(ref handle) = recording.handle {
        handle.resume();
    }

    // Accumulate this pause duration
    if let Some(ps) = recording.pause_start.take() {
        recording.pause_accumulated += ps.elapsed();
    }
    recording.is_paused = false;
    println!("[zureshot] Recording resumed");
    Ok(())
}

/// Open the floating recording control bar.
/// Called after recording starts to give the user stop/pause controls.
pub fn do_open_recording_bar(app: &AppHandle, region: Option<&CaptureRegion>) -> Result<(), String> {
    // If already open, just focus it
    if let Some(win) = app.get_webview_window("recording-bar") {
        let _ = win.show();
        let _ = win.set_focus();
        return Ok(());
    }

    // Bar dimensions (logical pixels)
    let bar_width = 220.0;
    let bar_height = 48.0;

    // Position: smart placement based on region vs screen
    let monitor = app
        .primary_monitor()
        .map_err(|e| format!("Failed to get monitor: {}", e))?
        .ok_or("No primary monitor found")?;
    let scale = monitor.scale_factor();
    let phys_size = monitor.size();
    let screen_w = phys_size.width as f64 / scale;
    let screen_h = phys_size.height as f64 / scale;

    let (pos_x, pos_y) = if let Some(rgn) = region {
        // Check if region is effectively fullscreen (covers >90% of screen)
        let is_fullscreen = rgn.width >= screen_w * 0.9 && rgn.height >= screen_h * 0.9;

        if is_fullscreen {
            // Fullscreen: bottom-center, above the Dock area
            ((screen_w - bar_width) / 2.0, screen_h - bar_height - 80.0)
        } else {
            // Custom region: find the side with the most available space
            let space_below = screen_h - (rgn.y + rgn.height);
            let space_above = rgn.y;
            let space_right = screen_w - (rgn.x + rgn.width);
            let space_left = rgn.x;

            let gap = 16.0;
            let cx = rgn.x + rgn.width / 2.0 - bar_width / 2.0;

            if space_below >= bar_height + gap + 20.0 {
                // Below the region (preferred)
                let x = cx.max(8.0).min(screen_w - bar_width - 8.0);
                (x, rgn.y + rgn.height + gap)
            } else if space_above >= bar_height + gap + 20.0 {
                // Above the region
                let x = cx.max(8.0).min(screen_w - bar_width - 8.0);
                (x, rgn.y - bar_height - gap)
            } else if space_right >= bar_width + gap + 20.0 {
                // Right of the region
                let y = (rgn.y + rgn.height / 2.0 - bar_height / 2.0)
                    .max(8.0).min(screen_h - bar_height - 8.0);
                (rgn.x + rgn.width + gap, y)
            } else if space_left >= bar_width + gap + 20.0 {
                // Left of the region
                let y = (rgn.y + rgn.height / 2.0 - bar_height / 2.0)
                    .max(8.0).min(screen_h - bar_height - 8.0);
                (rgn.x - bar_width - gap, y)
            } else {
                // No good space outside: place inside at bottom-center of region
                let x = cx.max(8.0).min(screen_w - bar_width - 8.0);
                let y = (rgn.y + rgn.height - bar_height - gap)
                    .max(8.0).min(screen_h - bar_height - 8.0);
                (x, y)
            }
        }
    } else {
        // No region at all: bottom-center
        ((screen_w - bar_width) / 2.0, screen_h - bar_height - 80.0)
    };

    let window = WebviewWindowBuilder::new(
        app,
        "recording-bar",
        WebviewUrl::App("recording-bar.html".into()),
    )
    .title("Recording")
    .inner_size(bar_width, bar_height)
    .position(pos_x, pos_y)
    .transparent(true)
    .decorations(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .resizable(false)
    .build()
    .map_err(|e| format!("Failed to create recording bar: {}", e))?;

    let _ = window.show();
    let _ = window.set_focus();

    println!("[zureshot] Recording bar opened at ({:.0}, {:.0})", pos_x, pos_y);
    Ok(())
}

/// Open a fullscreen transparent dim overlay that darkens the non-recorded area.
/// The overlay is click-through (ignores cursor events) so the user can still
/// interact with apps underneath. Only used for region recording.
pub fn do_open_recording_overlay(app: &AppHandle, region: &CaptureRegion) -> Result<(), String> {
    // If already open, just show it
    if let Some(win) = app.get_webview_window("recording-overlay") {
        let _ = win.show();
        return Ok(());
    }

    let monitor = app
        .primary_monitor()
        .map_err(|e| format!("Failed to get monitor: {}", e))?
        .ok_or("No primary monitor found")?;
    let scale = monitor.scale_factor();
    let phys_size = monitor.size();
    let position = monitor.position();
    let logical_w = phys_size.width as f64 / scale;
    let logical_h = phys_size.height as f64 / scale;

    let window = WebviewWindowBuilder::new(
        app,
        "recording-overlay",
        WebviewUrl::App("recording-overlay.html".into()),
    )
    .title("Recording Overlay")
    .inner_size(logical_w, logical_h)
    .position(position.x as f64 / scale, position.y as f64 / scale)
    .transparent(true)
    .decorations(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .resizable(false)
    .build()
    .map_err(|e| format!("Failed to create recording overlay: {}", e))?;

    // Make overlay completely click-through so it doesn't intercept mouse events
    let _ = window.set_ignore_cursor_events(true);
    let _ = window.show();

    println!(
        "[zureshot] Recording overlay opened for region ({:.0},{:.0} {:.0}x{:.0})",
        region.x, region.y, region.width, region.height
    );
    Ok(())
}

/// Refresh the stream content filter to exclude our app windows from capture.
/// Each platform handles this differently (macOS: SCStream filter, Linux: no-op).
pub fn refresh_stream_exclusion(app: &AppHandle) -> Result<(), String> {
    let state: tauri::State<'_, Mutex<RecordingState>> = app.state();
    let recording = state.lock().map_err(|e| e.to_string())?;
    if let Some(ref handle) = recording.handle {
        handle.refresh_exclusion(app)
    } else {
        Err("No active recording to update".into())
    }
}

// ════════════════════════════════════════════════════════════════════════
//  Camera bubble commands
// ════════════════════════════════════════════════════════════════════════

/// Camera bubble size presets (logical pixels)
fn camera_size_dimensions(size: &str, shape: &str) -> (f64, f64) {
    let base = match size {
        "small" => 100.0,
        "medium" => 180.0,
        "large" => 260.0,
        "huge" => 360.0,
        _ => 180.0,
    };
    match shape {
        "circle" | "square" => (base, base),
        "rectangle" => (base * 1.5, base),        // 3:2 landscape
        "vertical" => (base, base * 1.5),          // 2:3 portrait
        _ => (base, base),
    }
}

/// Move the camera overlay window by a delta, and update its bounds.
/// This is called from the frontend during region drag for real-time following.
#[tauri::command]
pub async fn move_camera_overlay(
    app: AppHandle,
    dx: f64,
    dy: f64,
    bound_x: f64,
    bound_y: f64,
    bound_w: f64,
    bound_h: f64,
) -> Result<(), String> {
    if let Some(win) = app.get_webview_window("camera-overlay") {
        let scale = win.scale_factor().unwrap_or(1.0);
        let pos = win.outer_position().map_err(|e| e.to_string())?;
        let logical_x = pos.x as f64 / scale + dx;
        let logical_y = pos.y as f64 / scale + dy;

        // Clamp within new bounds
        let size = win.outer_size().map_err(|e| e.to_string())?;
        let win_w = size.width as f64 / scale;
        let win_h = size.height as f64 / scale;
        let min_x = bound_x + 4.0;
        let min_y = bound_y + 4.0;
        let max_x = (bound_x + bound_w - win_w - 4.0).max(min_x);
        let max_y = (bound_y + bound_h - win_h - 4.0).max(min_y);
        let clamped_x = logical_x.max(min_x).min(max_x);
        let clamped_y = logical_y.max(min_y).min(max_y);

        let _ = win.set_position(tauri::LogicalPosition::new(clamped_x, clamped_y));
    }
    Ok(())
}

/// List camera devices via native AVFoundation (works with Continuity Camera)
#[tauri::command]
pub async fn list_native_camera_devices() -> Result<Vec<serde_json::Value>, String> {
    #[cfg(target_os = "macos")]
    {
        Ok(platform::macos::camera::list_camera_devices()
            .into_iter()
            .map(|d| serde_json::to_value(d).unwrap_or_default())
            .collect())
    }
    #[cfg(not(target_os = "macos"))]
    Err("Native camera is not supported on this platform yet".into())
}

/// Start native camera capture (ffmpeg-based, emits JPEG frames as events)
#[tauri::command]
pub async fn start_native_camera(
    app: AppHandle,
    device_id: String,
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let state: tauri::State<'_, Mutex<platform::macos::camera::NativeCameraState>> = app.state();
        let camera_state = state.lock().map_err(|e| e.to_string())?;
        platform::macos::camera::start_native_camera_stream(&app, &device_id, &camera_state)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, device_id);
        Err("Native camera is not supported on this platform yet".into())
    }
}

/// Stop native camera capture
#[tauri::command]
pub async fn stop_native_camera(
    app: AppHandle,
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let state: tauri::State<'_, Mutex<platform::macos::camera::NativeCameraState>> = app.state();
        let camera_state = state.lock().map_err(|e| e.to_string())?;
        platform::macos::camera::stop_native_camera_stream(&camera_state);
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        Err("Native camera is not supported on this platform yet".into())
    }
}

/// Payload emitted with `camera-overlay-settings` event
#[derive(Clone, Serialize, Deserialize)]
pub struct CameraOverlaySettings {
    pub shape: String,
    pub size: String,
    pub device_id: Option<String>,
    pub bound_x: Option<f64>,
    pub bound_y: Option<f64>,
    pub bound_w: Option<f64>,
    pub bound_h: Option<f64>,
}

/// Open the camera bubble overlay window with options.
pub fn do_open_camera_overlay_with_options(
    app: &AppHandle,
    shape: &str,
    size: &str,
    device_id: Option<&str>,
    bound_region: Option<&CaptureRegion>,
) -> Result<(), String> {
    // If already open, just emit new settings and refocus
    if let Some(win) = app.get_webview_window("camera-overlay") {
        let bounds = bound_region.map(|r| (r.x, r.y, r.width, r.height));
        let settings = CameraOverlaySettings {
            shape: shape.to_string(),
            size: size.to_string(),
            device_id: device_id.map(|s| s.to_string()),
            bound_x: bounds.map(|b| b.0),
            bound_y: bounds.map(|b| b.1),
            bound_w: bounds.map(|b| b.2),
            bound_h: bounds.map(|b| b.3),
        };
        let _ = app.emit("camera-overlay-settings", &settings);
        let _ = win.show();
        let _ = win.set_focus();
        return Ok(());
    }

    let (content_w, content_h) = camera_size_dimensions(size, shape);
    let padding = 16.0;
    let win_w = content_w + padding;
    let win_h = content_h + padding;

    // Position: bottom-right corner with margin
    let monitor = app
        .primary_monitor()
        .map_err(|e| format!("Failed to get monitor: {}", e))?
        .ok_or("No primary monitor found")?;
    let scale = monitor.scale_factor();
    let phys_size = monitor.size();
    let screen_w = phys_size.width as f64 / scale;
    let screen_h = phys_size.height as f64 / scale;

    let (bounds_x, bounds_y, bounds_w, bounds_h) = if let Some(r) = bound_region {
        (r.x, r.y, r.width, r.height)
    } else {
        (0.0, 0.0, screen_w, screen_h)
    };

    let margin = 12.0;
    let min_x = bounds_x + 4.0;
    let min_y = bounds_y + 4.0;
    let max_x = (bounds_x + bounds_w - win_w - 4.0).max(min_x);
    let max_y = (bounds_y + bounds_h - win_h - 4.0).max(min_y);
    let pos_x = (bounds_x + bounds_w - win_w - margin).max(min_x).min(max_x);
    let pos_y = (bounds_y + bounds_h - win_h - margin).max(min_y).min(max_y);

    // Pass initial settings via URL query params
    let url = format!(
        "camera-overlay.html?shape={}&size={}&deviceId={}&boundX={:.3}&boundY={:.3}&boundW={:.3}&boundH={:.3}",
        shape,
        size,
        device_id.unwrap_or(""),
        bounds_x,
        bounds_y,
        bounds_w,
        bounds_h
    );

    let builder = WebviewWindowBuilder::new(
        app,
        "camera-overlay",
        WebviewUrl::App(url.into()),
    )
    .title("Camera")
    .inner_size(win_w, win_h)
    .position(pos_x, pos_y)
    .transparent(true)
    .decorations(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .resizable(false);

    #[cfg(target_os = "macos")]
    let builder = builder; // No parent — parent windows can't be independently dragged on macOS

    let window = builder
        .build()
        .map_err(|e| format!("Failed to create camera overlay: {}", e))?;

    // On macOS, set NSWindow level above region-selector so camera is visible
    // but NOT a child window (child windows can't be dragged independently).
    // NSFloatingWindowLevel = 3, we use 4 to be one level above region-selector.
    #[cfg(target_os = "macos")]
    {
        use objc2::msg_send;
        if let Ok(ns_win) = window.ns_window() {
            let ns_window: *mut objc2::runtime::AnyObject = ns_win.cast();
            unsafe {
                let _: () = msg_send![ns_window, setLevel: 4_i64];
            }
            println!("[zureshot] Camera overlay NSWindow level set to 4 (above region-selector)");
        }
    }

    let _ = window.set_ignore_cursor_events(false);
    let _ = window.show();
    let _ = window.set_focus();

    println!(
        "[zureshot] Camera bubble opened: shape={}, size={} ({:.0}×{:.0}) at ({:.0},{:.0})",
        shape, size, content_w, content_h, pos_x, pos_y
    );

    // If currently recording, refresh stream exclusion
    {
        let state: tauri::State<'_, Mutex<RecordingState>> = app.state();
        let is_recording = state.lock().map(|r| r.is_recording).unwrap_or(false);
        if is_recording {
            std::thread::sleep(std::time::Duration::from_millis(150));
            let _ = refresh_stream_exclusion(app);
        }
    }

    Ok(())
}

/// Open the camera bubble overlay window (default: circle, medium).
pub fn do_open_camera_overlay(app: &AppHandle) -> Result<(), String> {
    let region = {
        let state: tauri::State<'_, Mutex<RecordingState>> = app.state();
        state.lock().ok().and_then(|r| r.region.clone())
    };
    do_open_camera_overlay_with_options(app, "circle", "medium", None, region.as_ref())
}

/// Close the camera bubble overlay window.
pub fn do_close_camera_overlay(app: &AppHandle) -> Result<(), String> {
    // Stop native camera if running
    #[cfg(target_os = "macos")]
    {
        if let Some(state) = app.try_state::<Mutex<platform::macos::camera::NativeCameraState>>() {
            if let Ok(camera_state) = state.lock() {
                platform::macos::camera::stop_native_camera_stream(&camera_state);
            }
        }
    }
    if let Some(win) = app.get_webview_window("camera-overlay") {
        // Emit close event so the Svelte component can stop the camera stream
        let _ = app.emit("camera-overlay-close", ());
        // Brief delay to allow cleanup
        std::thread::sleep(std::time::Duration::from_millis(50));
        let _ = win.destroy();
        println!("[zureshot] Camera bubble closed");
    }
    Ok(())
}

/// Tauri command: open camera bubble
#[tauri::command]
pub async fn open_camera_overlay(app: AppHandle) -> Result<(), String> {
    do_open_camera_overlay(&app)
}

/// Tauri command: open camera bubble with options (shape, size, device)
#[tauri::command]
pub async fn open_camera_overlay_with_options(
    app: AppHandle,
    shape: Option<String>,
    size: Option<String>,
    device_id: Option<String>,
    bound_x: Option<f64>,
    bound_y: Option<f64>,
    bound_w: Option<f64>,
    bound_h: Option<f64>,
) -> Result<(), String> {
    let bounds_region = match (bound_x, bound_y, bound_w, bound_h) {
        (Some(x), Some(y), Some(width), Some(height)) if width > 0.0 && height > 0.0 => {
            Some(CaptureRegion { x, y, width, height })
        }
        _ => None,
    };

    println!(
        "[zureshot] open_camera_overlay_with_options called: shape={:?}, size={:?}, device_id={:?}, bounds={:?}",
        shape,
        size,
        device_id,
        bounds_region
            .as_ref()
            .map(|b| format!("{:.0},{:.0} {:.0}x{:.0}", b.x, b.y, b.width, b.height))
    );
    do_open_camera_overlay_with_options(
        &app,
        &shape.unwrap_or_else(|| "circle".to_string()),
        &size.unwrap_or_else(|| "medium".to_string()),
        device_id.as_deref(),
        bounds_region.as_ref(),
    )
}

/// Tauri command: close camera bubble
#[tauri::command]
pub async fn close_camera_overlay(app: AppHandle) -> Result<(), String> {
    do_close_camera_overlay(&app)
}

/// Tauri command: toggle camera bubble (open if closed, close if open)
#[tauri::command]
pub async fn toggle_camera_overlay(app: AppHandle) -> Result<bool, String> {
    if app.get_webview_window("camera-overlay").is_some() {
        do_close_camera_overlay(&app)?;
        Ok(false) // now closed
    } else {
        do_open_camera_overlay(&app)?;
        Ok(true) // now open
    }
}

// ════════════════════════════════════════════════════════════════════════
//  Screenshot commands
// ════════════════════════════════════════════════════════════════════════

/// Result of a screenshot capture
#[derive(Clone, Serialize, Deserialize)]
pub struct ScreenshotResult {
    pub path: String,
    pub width: usize,
    pub height: usize,
    pub file_size_bytes: u64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub image_base64: String,
}

/// Open the region selector in screenshot mode.
pub fn do_start_screenshot_selection(app: &AppHandle) -> Result<(), String> {
    // If a region-selector window already exists, just show + focus it
    if let Some(win) = app.get_webview_window("region-selector") {
        let _ = win.show();
        let _ = win.set_focus();
        return Ok(());
    }

    let monitor = app
        .primary_monitor()
        .map_err(|e| format!("Failed to get monitor: {}", e))?
        .ok_or("No primary monitor found")?;
    let phys_size = monitor.size();
    let scale = monitor.scale_factor();
    let position = monitor.position();
    let logical_w = phys_size.width as f64 / scale;
    let logical_h = phys_size.height as f64 / scale;

    // Capture full screen BEFORE showing the overlay (freeze the screen)
    let preview_path = std::env::temp_dir()
        .join("zureshot_screen_preview.png")
        .to_string_lossy()
        .to_string();
    let _ = platform::imp::take_screenshot_region(0.0, 0.0, logical_w, logical_h, &preview_path);

    // Pass mode + preview path via URL query parameters
    let url = format!(
        "region-selector.html?mode=screenshot&preview={}",
        urlencoding::encode(&preview_path)
    );

    let window = WebviewWindowBuilder::new(
        app,
        "region-selector",
        WebviewUrl::App(url.into()),
    )
    .title("Screenshot Region")
    .inner_size(logical_w, logical_h)
    .position(position.x as f64 / scale, position.y as f64 / scale)
    .transparent(true)
    .decorations(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .resizable(false)
    .build()
    .map_err(|e| format!("Failed to create region selector window: {}", e))?;

    let _ = window.show();
    let _ = window.set_focus();

    println!("[zureshot] Screenshot region selector opened (with frozen preview)");
    Ok(())
}

/// Tauri command: open screenshot region selector
#[tauri::command]
pub async fn start_screenshot_selection(app: AppHandle) -> Result<(), String> {
    do_start_screenshot_selection(&app)
}

/// Tauri command: take a screenshot of the selected region
#[tauri::command]
pub async fn take_screenshot(
    app: AppHandle,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<ScreenshotResult, String> {
    // Close region selector immediately
    if let Some(win) = app.get_webview_window("region-selector") {
        let _ = win.hide();
        // Destroy after a tiny delay so the window disappears from the capture
        let app2 = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(100));
            if let Some(win) = app2.get_webview_window("region-selector") {
                let _ = win.destroy();
            }
        });
    }

    // Small delay to ensure region-selector is fully hidden
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Generate temp file path
    let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
    let base = dirs::download_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let zureshot_dir = base.join("Zureshot");
    let _ = std::fs::create_dir_all(&zureshot_dir);
    let temp_path = zureshot_dir
        .join(format!(".zureshot_screenshot_{}.png", timestamp))
        .to_string_lossy()
        .to_string();

    // Capture
    let (img_w, img_h, file_size) = platform::imp::take_screenshot_region(x, y, width, height, &temp_path)?;

    // Read file and encode as base64 for preview
    let file_bytes = std::fs::read(&temp_path)
        .map_err(|e| format!("Failed to read screenshot file: {}", e))?;
    let image_b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &file_bytes);

    let result = ScreenshotResult {
        path: temp_path.clone(),
        width: img_w,
        height: img_h,
        file_size_bytes: file_size,
        image_base64: image_b64,
    };

    println!(
        "[zureshot] Screenshot taken: {}x{} ({:.1} KB) → {}",
        img_w, img_h,
        file_size as f64 / 1024.0,
        temp_path
    );

    // Show screenshot preview window
    let is_new = do_open_screenshot_preview(&app).unwrap_or(false);

    // If window was just created, wait for the webview to load and register listeners
    if is_new {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }

    // Emit event to the preview window
    let _ = app.emit("screenshot-taken", &result);

    Ok(result)
}

/// Tauri command: capture a region and copy directly to clipboard (no preview window)
#[tauri::command]
pub async fn screenshot_to_clipboard(
    app: AppHandle,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<(), String> {
    // Hide region selector
    if let Some(win) = app.get_webview_window("region-selector") {
        let _ = win.hide();
    }
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Capture to temp file
    let temp_path = std::env::temp_dir()
        .join("zureshot_clipboard_capture.png")
        .to_string_lossy()
        .to_string();
    platform::imp::take_screenshot_region(x, y, width, height, &temp_path)?;

    // Copy to clipboard
    platform::imp::copy_image_to_clipboard(&temp_path)?;

    // Clean up
    let _ = std::fs::remove_file(&temp_path);

    // Destroy region selector
    if let Some(win) = app.get_webview_window("region-selector") {
        let _ = win.destroy();
    }

    // Clean up frozen preview
    let preview_path = std::env::temp_dir().join("zureshot_screen_preview.png");
    let _ = std::fs::remove_file(&preview_path);

    println!("[zureshot] Screenshot captured and copied to clipboard");
    Ok(())
}

/// Tauri command: copy base64-encoded PNG image data to clipboard
#[tauri::command]
pub async fn copy_image_data_to_clipboard(app: AppHandle, data: String) -> Result<(), String> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&data)
        .map_err(|e| format!("Invalid base64: {}", e))?;

    let temp_path = std::env::temp_dir()
        .join("zureshot_annotated.png")
        .to_string_lossy()
        .to_string();
    std::fs::write(&temp_path, &bytes)
        .map_err(|e| format!("Failed to write temp file: {}", e))?;

    platform::imp::copy_image_to_clipboard(&temp_path)?;

    let _ = std::fs::remove_file(&temp_path);

    // Clean up frozen preview
    let preview_path = std::env::temp_dir().join("zureshot_screen_preview.png");
    let _ = std::fs::remove_file(&preview_path);

    // Destroy region selector
    if let Some(win) = app.get_webview_window("region-selector") {
        let _ = win.destroy();
    }

    println!("[zureshot] Annotated screenshot copied to clipboard");
    Ok(())
}

/// Tauri command: close a pinned screenshot window by its label
#[tauri::command]
pub async fn close_pin_window(app: AppHandle, label: String) -> Result<(), String> {
    if let Some(win) = app.get_webview_window(&label) {
        let _ = win.destroy();
        println!("[zureshot] Pin window closed: {}", label);
    }
    Ok(())
}

/// Tauri command: save annotated screenshot to file and pin to desktop
#[tauri::command]
pub async fn save_annotated_and_pin(
    app: AppHandle,
    data: String,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<String, String> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&data)
        .map_err(|e| format!("Invalid base64: {}", e))?;

    let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
    let base = dirs::download_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let zureshot_dir = base.join("Zureshot");
    let _ = std::fs::create_dir_all(&zureshot_dir);
    let save_path = zureshot_dir
        .join(format!("screenshot_{}.png", timestamp))
        .to_string_lossy()
        .to_string();

    std::fs::write(&save_path, &bytes)
        .map_err(|e| format!("Failed to write screenshot: {}", e))?;

    // Clean up frozen preview
    let preview_path = std::env::temp_dir().join("zureshot_screen_preview.png");
    let _ = std::fs::remove_file(&preview_path);

    // Destroy region selector
    if let Some(win) = app.get_webview_window("region-selector") {
        let _ = win.destroy();
    }

    // Pin it (reuse pin_screenshot logic)
    let pin_id = PIN_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let window_label = format!("pin-{}", pin_id);
    let url = format!(
        "pinned-screenshot.html?path={}&label={}",
        urlencoding::encode(&save_path),
        urlencoding::encode(&window_label)
    );

    let window = WebviewWindowBuilder::new(
        &app,
        &window_label,
        WebviewUrl::App(url.into()),
    )
    .title("Pinned")
    .inner_size(width, height)
    .min_inner_size(120.0, 80.0)
    .position(x, y)
    .transparent(true)
    .decorations(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .resizable(true)
    .build()
    .map_err(|e| format!("Failed to create pinned window: {}", e))?;

    let _ = window.show();
    println!("[zureshot] Screenshot pinned at ({},{}) {}x{}: {} (window: {})", x, y, width, height, save_path, window_label);
    Ok(window_label)
}

/// Tauri command: save screenshot to permanent location (move from temp)
#[tauri::command]
pub async fn save_screenshot(path: String) -> Result<String, String> {
    let src = std::path::Path::new(&path);
    if !src.exists() {
        return Err("Screenshot file not found".into());
    }

    // Rename from hidden temp file (.zureshot_screenshot_...) to final name
    let filename = src
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let final_name = filename.trim_start_matches('.');
    let dest = src.parent().unwrap().join(final_name);

    std::fs::rename(&path, &dest).map_err(|e| format!("Failed to save screenshot: {}", e))?;

    let dest_str = dest.to_string_lossy().to_string();
    println!("[zureshot] Screenshot saved: {}", dest_str);
    Ok(dest_str)
}

/// Tauri command: copy screenshot to clipboard
#[tauri::command]
pub async fn copy_screenshot(path: String) -> Result<(), String> {
    let src = std::path::Path::new(&path);
    if !src.exists() {
        return Err("Screenshot file not found".into());
    }

    platform::imp::copy_image_to_clipboard(&path)?;

    // Clean up temp file after copying
    let _ = std::fs::remove_file(&path);
    println!("[zureshot] Screenshot copied to clipboard and temp file removed");
    Ok(())
}

/// Tauri command: dismiss screenshot (delete temp file)
#[tauri::command]
pub async fn dismiss_screenshot(path: String) -> Result<(), String> {
    let _ = std::fs::remove_file(&path);
    println!("[zureshot] Screenshot dismissed, temp file removed");
    Ok(())
}

/// Open the screenshot preview window (bottom-left floating)
fn do_open_screenshot_preview(app: &AppHandle) -> Result<bool, String> {
    if let Some(win) = app.get_webview_window("screenshot-preview") {
        let _ = win.show();
        let _ = win.set_focus();
        return Ok(false); // Window already existed
    }

    let window = WebviewWindowBuilder::new(
        app,
        "screenshot-preview",
        WebviewUrl::App("screenshot-preview.html".into()),
    )
    .title("Screenshot")
    .inner_size(300.0, 180.0)
    .position(20.0, 600.0) // Will be repositioned by frontend
    .transparent(true)
    .decorations(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .resizable(false)
    .build()
    .map_err(|e| format!("Failed to create screenshot preview: {}", e))?;

    let _ = window.show();
    let _ = window.set_focus();

    Ok(true) // Newly created
}

// ════════════════════════════════════════════════════════════════════════
//  Pin Screenshot to Desktop
// ════════════════════════════════════════════════════════════════════════

/// Counter for unique pinned window IDs
static PIN_COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Pin a screenshot to the desktop as an always-on-top floating window.
#[tauri::command]
pub async fn pin_screenshot(app: AppHandle, path: String) -> Result<String, String> {
    // First save the screenshot to a permanent location if it's a temp file
    let permanent_path = if path.contains(".zureshot_screenshot_") {
        // It's a temp file — save it permanently first
        let src = std::path::Path::new(&path);
        if !src.exists() {
            return Err("Screenshot file not found".into());
        }
        let filename = src
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .replace(".zureshot_screenshot_", "screenshot_");
        let dest = src.parent().unwrap().join(&filename);
        std::fs::copy(&path, &dest)
            .map_err(|e| format!("Failed to copy screenshot: {}", e))?;
        dest.to_string_lossy().to_string()
    } else {
        path.clone()
    };

    let pin_id = PIN_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let window_label = format!("pin-{}", pin_id);

    let url = format!(
        "pinned-screenshot.html?path={}&label={}",
        urlencoding::encode(&permanent_path),
        urlencoding::encode(&window_label)
    );

    let window = WebviewWindowBuilder::new(
        &app,
        &window_label,
        WebviewUrl::App(url.into()),
    )
    .title("Pinned")
    .inner_size(320.0, 220.0)
    .min_inner_size(120.0, 80.0)
    .position(100.0 + (pin_id as f64 * 30.0), 100.0 + (pin_id as f64 * 30.0))
    .transparent(true)
    .decorations(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .resizable(true)
    .build()
    .map_err(|e| format!("Failed to create pinned window: {}", e))?;

    let _ = window.show();

    println!("[zureshot] Screenshot pinned: {} (window: {})", permanent_path, window_label);
    Ok(window_label)
}

// ════════════════════════════════════════════════════════════════════════
//  OCR Text Recognition
// ════════════════════════════════════════════════════════════════════════

/// OCR result returned to frontend
#[derive(Clone, Serialize, Deserialize)]
pub struct OcrResponse {
    pub full_text: String,
    pub block_count: usize,
}

/// Recognize text in a screenshot image.
#[tauri::command]
pub async fn ocr_screenshot(_path: String) -> Result<OcrResponse, String> {
    #[cfg(target_os = "macos")]
    {
        let result = tokio::task::spawn_blocking(move || {
            crate::platform::macos::ocr::recognize_text(&_path)
        })
        .await
        .map_err(|e| format!("Task join error: {}", e))??;

        Ok(OcrResponse {
            block_count: result.blocks.len(),
            full_text: result.full_text,
        })
    }

    #[cfg(not(target_os = "macos"))]
    Err("OCR is not supported on this platform yet".into())
}

// ════════════════════════════════════════════════════════════════════════
//  Scrolling Long Screenshot
// ════════════════════════════════════════════════════════════════════════

/// State for the scroll capture session (managed by Tauri)
pub struct ScrollCaptureStateWrapper {
    #[cfg(target_os = "macos")]
    pub session: Option<crate::platform::macos::scroll_capture::ScrollCaptureSession>,
    #[cfg(not(target_os = "macos"))]
    pub session: Option<()>,
}

impl Default for ScrollCaptureStateWrapper {
    fn default() -> Self {
        Self { session: None }
    }
}

// SAFETY: Session is only accessed through Mutex
unsafe impl Send for ScrollCaptureStateWrapper {}
unsafe impl Sync for ScrollCaptureStateWrapper {}

#[derive(Clone, Serialize, Deserialize)]
pub struct ScrollCaptureStatus {
    pub frame_count: usize,
    pub total_height: usize,
    pub width: usize,
    pub new_content: bool,
}

/// Open region selector in scroll-screenshot mode.
pub fn do_start_scroll_screenshot_selection(app: &AppHandle) -> Result<(), String> {
    if let Some(win) = app.get_webview_window("region-selector") {
        let _ = win.show();
        let _ = win.set_focus();
        return Ok(());
    }

    let monitor = app
        .primary_monitor()
        .map_err(|e| format!("Failed to get monitor: {}", e))?
        .ok_or("No primary monitor found")?;
    let phys_size = monitor.size();
    let scale = monitor.scale_factor();
    let position = monitor.position();
    let logical_w = phys_size.width as f64 / scale;
    let logical_h = phys_size.height as f64 / scale;

    let window = WebviewWindowBuilder::new(
        app,
        "region-selector",
        WebviewUrl::App("region-selector.html?mode=scroll-screenshot".into()),
    )
    .title("Scroll Screenshot Region")
    .inner_size(logical_w, logical_h)
    .position(position.x as f64 / scale, position.y as f64 / scale)
    .transparent(true)
    .decorations(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .resizable(false)
    .build()
    .map_err(|e| format!("Failed to create region selector window: {}", e))?;

    let _ = window.show();
    let _ = window.set_focus();

    println!("[zureshot] Scroll screenshot region selector opened");
    Ok(())
}

#[tauri::command]
pub async fn start_scroll_screenshot_selection(app: AppHandle) -> Result<(), String> {
    do_start_scroll_screenshot_selection(&app)
}

/// Start a scroll capture session: capture the first frame and open the control bar.
#[tauri::command]
pub async fn start_scroll_capture(
    app: AppHandle,
    _x: f64,
    _y: f64,
    _width: f64,
    _height: f64,
) -> Result<ScrollCaptureStatus, String> {
    // Close region selector
    if let Some(win) = app.get_webview_window("region-selector") {
        let _ = win.destroy();
    }

    #[cfg(target_os = "macos")]
    {
        use crate::platform::macos::scroll_capture::ScrollCaptureSession;

        let session = ScrollCaptureSession::new(_x, _y, _width, _height)?;
        let status = ScrollCaptureStatus {
            frame_count: session.frame_count(),
            total_height: session.total_height(),
            width: session.width(),
            new_content: true,
        };

        let state: tauri::State<'_, Mutex<ScrollCaptureStateWrapper>> = app.state();
        let mut guard = state.lock().map_err(|e| e.to_string())?;
        guard.session = Some(session);

        // Open scroll capture bar
        do_open_scroll_capture_bar(&app)?;

        Ok(status)
    }

    #[cfg(not(target_os = "macos"))]
    Err("Scroll capture is not supported on this platform yet".into())
}

/// Capture one frame and stitch if new content detected.
#[tauri::command]
pub async fn scroll_capture_tick(
    _app: AppHandle,
) -> Result<ScrollCaptureStatus, String> {
    #[cfg(target_os = "macos")]
    {
        let state: tauri::State<'_, Mutex<ScrollCaptureStateWrapper>> = _app.state();
        let mut guard = state.lock().map_err(|e| e.to_string())?;
        let session = guard.session.as_mut().ok_or("No scroll capture session active")?;

        let new_content = session.capture_frame()?;

        Ok(ScrollCaptureStatus {
            frame_count: session.frame_count(),
            total_height: session.total_height(),
            width: session.width(),
            new_content,
        })
    }

    #[cfg(not(target_os = "macos"))]
    Err("Scroll capture is not supported on this platform yet".into())
}

/// Finish scroll capture: stitch and save as PNG.
#[tauri::command]
pub async fn finish_scroll_capture(
    app: AppHandle,
) -> Result<ScreenshotResult, String> {
    // Close the scroll capture bar
    if let Some(win) = app.get_webview_window("scroll-capture-bar") {
        let _ = win.destroy();
    }

    #[cfg(target_os = "macos")]
    {
        let state: tauri::State<'_, Mutex<ScrollCaptureStateWrapper>> = app.state();
        let mut guard = state.lock().map_err(|e| e.to_string())?;
        let session = guard.session.take().ok_or("No scroll capture session active")?;

        let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
        let base = dirs::download_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        let zureshot_dir = base.join("Zureshot");
        let _ = std::fs::create_dir_all(&zureshot_dir);
        let output_path = zureshot_dir
            .join(format!("scroll_screenshot_{}.png", timestamp))
            .to_string_lossy()
            .to_string();

        let (width, height, file_size) = session.finish(&output_path)?;

        // Read file for base64 preview
        let image_base64 = std::fs::read(&output_path)
            .map(|bytes| {
                use base64::Engine;
                base64::engine::general_purpose::STANDARD.encode(&bytes)
            })
            .unwrap_or_default();

        let result = ScreenshotResult {
            path: output_path.clone(),
            width,
            height,
            file_size_bytes: file_size,
            image_base64,
        };

        println!(
            "[zureshot] Scroll screenshot saved: {}x{} ({:.1} KB) -> {}",
            width, height, file_size as f64 / 1024.0, output_path
        );

        // Emit event for screenshot preview
        let _ = app.emit("screenshot-taken", &result);

        // Open screenshot preview
        let _ = do_open_screenshot_preview(&app);

        Ok(result)
    }

    #[cfg(not(target_os = "macos"))]
    Err("Scroll capture is not supported on this platform yet".into())
}

/// Cancel scroll capture without saving.
#[tauri::command]
pub async fn cancel_scroll_capture(
    app: AppHandle,
) -> Result<(), String> {
    if let Some(win) = app.get_webview_window("scroll-capture-bar") {
        let _ = win.destroy();
    }

    let state: tauri::State<'_, Mutex<ScrollCaptureStateWrapper>> = app.state();
    let mut guard = state.lock().map_err(|e| e.to_string())?;
    guard.session = None;

    println!("[zureshot] Scroll capture cancelled");
    Ok(())
}

/// Open the scroll capture floating control bar (bottom-center of screen)
fn do_open_scroll_capture_bar(app: &AppHandle) -> Result<(), String> {
    if let Some(win) = app.get_webview_window("scroll-capture-bar") {
        let _ = win.show();
        let _ = win.set_focus();
        return Ok(());
    }

    // Position at bottom-center of primary monitor
    let (pos_x, pos_y) = if let Ok(Some(monitor)) = app.primary_monitor() {
        let scale = monitor.scale_factor();
        let screen_w = monitor.size().width as f64 / scale;
        let screen_h = monitor.size().height as f64 / scale;
        let bar_w = 260.0;
        let bar_h = 44.0;
        ((screen_w - bar_w) / 2.0, screen_h - bar_h - 60.0)
    } else {
        (500.0, 700.0)
    };

    let window = WebviewWindowBuilder::new(
        app,
        "scroll-capture-bar",
        WebviewUrl::App("scroll-capture-bar.html".into()),
    )
    .title("Scroll Capture")
    .inner_size(260.0, 44.0)
    .position(pos_x, pos_y)
    .transparent(true)
    .decorations(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .resizable(false)
    .build()
    .map_err(|e| format!("Failed to create scroll capture bar: {}", e))?;

    let _ = window.show();
    // Don't steal focus — user needs to interact with the target app
    // let _ = window.set_focus();

    Ok(())
}

// ════════════════════════════════════════════════════════════════════════
//  Video Editor commands
// ════════════════════════════════════════════════════════════════════════

/// Open the video editor window for a recorded video.
#[tauri::command]
pub async fn open_video_editor(app: AppHandle, path: String) -> Result<(), String> {
    do_open_video_editor(&app, &path)
}

/// Core logic to open the video editor window.
pub fn do_open_video_editor(app: &AppHandle, video_path: &str) -> Result<(), String> {
    // Close existing editor if open
    if let Some(win) = app.get_webview_window("video-editor") {
        let _ = win.destroy();
    }

    // Also close the thumbnail preview — editor replaces it
    if let Some(win) = app.get_webview_window("thumbnail") {
        let _ = win.destroy();
    }

    let url = format!(
        "video-editor.html?path={}",
        urlencoding::encode(video_path)
    );

    let window = WebviewWindowBuilder::new(
        app,
        "video-editor",
        WebviewUrl::App(url.into()),
    )
    .title("Zureshot Editor")
    .inner_size(960.0, 640.0)
    .min_inner_size(720.0, 480.0)
    .center()
    .transparent(false)
    .decorations(true)
    .resizable(true)
    .build()
    .map_err(|e| format!("Failed to create editor window: {}", e))?;

    let _ = window.show();
    let _ = window.set_focus();

    println!("[editor] Video editor opened for: {}", video_path);
    Ok(())
}

/// Get video metadata (duration, resolution, codec, etc.)
#[tauri::command]
pub async fn get_video_metadata(path: String) -> Result<serde_json::Value, String> {
    #[cfg(target_os = "macos")]
    {
        let meta = platform::macos::editor::get_video_metadata(&path)?;
        serde_json::to_value(meta).map_err(|e| e.to_string())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = path;
        Err("Video editor is not supported on this platform yet".into())
    }
}

/// Generate timeline thumbnail strip
#[tauri::command]
pub async fn generate_timeline_thumbnails(
    path: String,
    count: Option<usize>,
    thumb_height: Option<u32>,
) -> Result<Vec<serde_json::Value>, String> {
    #[cfg(target_os = "macos")]
    {
        let count = count.unwrap_or(20);
        let height = thumb_height.unwrap_or(60);
        tokio::task::spawn_blocking(move || {
            platform::macos::editor::generate_timeline_thumbnails(&path, count, height)
                .map(|v| v.into_iter().map(|t| serde_json::to_value(t).unwrap_or_default()).collect())
        })
        .await
        .map_err(|e| format!("Task join error: {}", e))?
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (path, count, thumb_height);
        Err("Video editor is not supported on this platform yet".into())
    }
}

/// Generate audio waveform data for timeline display
#[tauri::command]
pub async fn generate_waveform(
    path: String,
    num_samples: Option<usize>,
) -> Result<serde_json::Value, String> {
    #[cfg(target_os = "macos")]
    {
        let samples = num_samples.unwrap_or(200);
        tokio::task::spawn_blocking(move || {
            platform::macos::editor::generate_waveform(&path, samples)
                .and_then(|w| serde_json::to_value(w).map_err(|e| e.to_string()))
        })
        .await
        .map_err(|e| format!("Task join error: {}", e))?
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (path, num_samples);
        Err("Video editor is not supported on this platform yet".into())
    }
}

/// Trim video (stream copy — near-instant, no re-encode)
#[tauri::command]
pub async fn trim_video(
    input_path: String,
    start_secs: f64,
    end_secs: f64,
    output_path: Option<String>,
) -> Result<String, String> {
    #[cfg(target_os = "macos")]
    {
        let out = output_path.unwrap_or_else(|| {
            let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
            let base = dirs::download_dir()
                .or_else(dirs::home_dir)
                .unwrap_or_else(|| std::path::PathBuf::from("."));
            let zureshot_dir = base.join("Zureshot");
            let _ = std::fs::create_dir_all(&zureshot_dir);
            zureshot_dir
                .join(format!("zureshot_trimmed_{}.mp4", timestamp))
                .to_string_lossy()
                .to_string()
        });

        tokio::task::spawn_blocking(move || {
            platform::macos::editor::trim_video(&input_path, start_secs, end_secs, &out)
        })
        .await
        .map_err(|e| format!("Task join error: {}", e))?
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (input_path, start_secs, end_secs, output_path);
        Err("Video editor is not supported on this platform yet".into())
    }
}

/// Render a single preview frame with editor effects
#[tauri::command]
pub async fn render_preview_frame(
    path: String,
    time_secs: f64,
    width: Option<u32>,
    height: Option<u32>,
) -> Result<String, String> {
    #[cfg(target_os = "macos")]
    {
        let w = width.unwrap_or(640);
        let h = height.unwrap_or(360);
        let bg = platform::macos::editor::Background::Transparent;

        tokio::task::spawn_blocking(move || {
            platform::macos::editor::render_preview_frame(&path, time_secs, w, h, 0.0, 0.0, &bg)
        })
        .await
        .map_err(|e| format!("Task join error: {}", e))?
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (path, time_secs, width, height);
        Err("Video editor is not supported on this platform yet".into())
    }
}

/// Export video with editor effects (trim, background, zoom, etc.)
#[tauri::command]
pub async fn start_export(
    app: AppHandle,
    project: serde_json::Value,
) -> Result<String, String> {
    #[cfg(target_os = "macos")]
    {
        let project: platform::macos::editor::VideoEditProject =
            serde_json::from_value(project).map_err(|e| format!("Invalid project: {}", e))?;
        tokio::task::spawn_blocking(move || {
            platform::macos::editor::export_video(&project, &app)
        })
        .await
        .map_err(|e| format!("Task join error: {}", e))?
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, project);
        Err("Video editor is not supported on this platform yet".into())
    }
}

/// Get mouse track data for a video (for auto-zoom suggestions)
#[tauri::command]
pub async fn get_mouse_track(
    video_path: String,
) -> Result<serde_json::Value, String> {
    #[cfg(target_os = "macos")]
    {
        let track = platform::macos::mouse_tracker::load_mouse_track(&video_path)?;
        serde_json::to_value(track).map_err(|e| e.to_string())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = video_path;
        Err("Mouse tracking is not supported on this platform yet".into())
    }
}

/// Debug log from frontend (prints to terminal)
#[tauri::command]
pub fn log_debug(msg: String) {
    println!("{}", msg);
}

/// Get auto-zoom keyframe suggestions based on mouse tracking data
#[tauri::command]
pub async fn suggest_zoom_keyframes(
    video_path: String,
) -> Result<Vec<serde_json::Value>, String> {
    #[cfg(target_os = "macos")]
    {
        let track = platform::macos::mouse_tracker::load_mouse_track(&video_path)?;
        let kfs = platform::macos::mouse_tracker::suggest_zoom_keyframes(&track);
        Ok(kfs.into_iter().map(|k| serde_json::to_value(k).unwrap_or_default()).collect())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = video_path;
        Err("Zoom suggestions are not supported on this platform yet".into())
    }
}
