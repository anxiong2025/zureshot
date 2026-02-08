//! Tauri commands for screen recording.
//!
//! These functions are exposed to the frontend via Tauri's IPC mechanism.

use crate::capture;
use crate::capture::RecordingQuality;
use crate::writer;
use objc2::rc::Retained;
use objc2_av_foundation::{AVAssetWriter, AVAssetWriterInput};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_screen_capture_kit::SCStream;
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, WebviewWindowBuilder, WebviewUrl};

/// Region definition for region-based recording (web coordinates: top-left origin, CSS pixels)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CaptureRegion {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Recording state shared across commands
pub struct RecordingState {
    pub stream: Option<Retained<SCStream>>,
    pub writer: Option<Retained<AVAssetWriter>>,
    pub input: Option<Retained<AVAssetWriterInput>>,
    /// System audio writer input (AAC track)
    pub audio_input: Option<Retained<AVAssetWriterInput>>,
    /// Microphone writer input (AAC track)
    pub mic_input: Option<Retained<AVAssetWriterInput>>,
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
    /// Shared paused flag read by the capture delegate (AtomicBool behind Arc)
    pub paused_flag: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
}

impl Default for RecordingState {
    fn default() -> Self {
        Self {
            stream: None,
            writer: None,
            input: None,
            audio_input: None,
            mic_input: None,
            output_path: None,
            is_recording: false,
            is_paused: false,
            start_time: None,
            pause_accumulated: std::time::Duration::ZERO,
            pause_start: None,
            region: None,
            quality: RecordingQuality::Standard,
            output_format: "video".to_string(),
            paused_flag: None,
        }
    }
}

// SAFETY: RecordingState contains Objective-C objects that are thread-safe.
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

/// Catch ObjC exceptions for a simple void call.
fn catch_objc_cmd(context: &str, f: impl FnOnce()) {
    use std::panic::AssertUnwindSafe;
    if let Err(e) = objc2::exception::catch(AssertUnwindSafe(f)) {
        let desc = e
            .map(|ex| format!("{ex}"))
            .unwrap_or_else(|| "unknown ObjC exception".into());
        eprintln!("[zureshot] ObjC exception in {}: {}", context, desc);
    }
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

    // Remove existing file (AVAssetWriter won't overwrite)
    let _ = std::fs::remove_file(&path);

    println!("[zureshot] Starting recording to: {}", path);

    // Get display and windows for potential exclusion
    let (display, all_windows) = capture::get_display_and_windows().map_err(|e| {
        eprintln!("[zureshot] {}", e);
        e
    })?;
    let (phys_width, phys_height, retina_scale) = capture::display_physical_size(&display);
    println!("[zureshot] Display: {}x{} physical, scale={}", phys_width, phys_height, retina_scale);

    // Determine output dimensions and source rect
    let (width, height, source_rect) = if let Some(ref rgn) = region {
        // Use Retina scale to convert CSS/logical pixels → physical pixels.
        // Region coordinates from the web UI are in logical (CSS) points.
        // Output dimensions must be in physical pixels for pixel-perfect sharpness.
        let pixel_w = (rgn.width * retina_scale) as usize;
        let pixel_h = (rgn.height * retina_scale) as usize;
        // Ensure even dimensions for HEVC
        let pixel_w = if pixel_w % 2 != 0 { pixel_w + 1 } else { pixel_w };
        let pixel_h = if pixel_h % 2 != 0 { pixel_h + 1 } else { pixel_h };

        // ScreenCaptureKit sourceRect uses logical points (top-left origin),
        // same as CSS coordinates. No coordinate conversion needed.
        let rect = CGRect::new(
            CGPoint::new(rgn.x, rgn.y),
            CGSize::new(rgn.width, rgn.height),
        );
        println!(
            "[zureshot] Region: css({},{} {}x{}) → pixels({}x{}) scale={} quality={:?}",
            rgn.x, rgn.y, rgn.width, rgn.height,
            pixel_w, pixel_h, retina_scale, quality
        );
        (pixel_w, pixel_h, Some(rect))
    } else {
        // Full screen: native Retina physical pixels for both Standard and High.
        // Standard vs High only differs in frame rate (30 vs 60 fps).
        println!("[zureshot] Full screen: {}x{} (physical, {}x Retina) quality={:?}", phys_width, phys_height, retina_scale, quality);
        (phys_width, phys_height, None)
    };

    // Collect windows to exclude (our own app windows)
    let exclude_windows = collect_app_windows_to_exclude(app, &all_windows);

    // Create H.264 writer
    let (writer, input) = writer::create_writer(&path, width, height, quality).map_err(|e| {
        eprintln!("[zureshot] {}", e);
        e
    })?;

    // Create audio writer inputs if audio capture is requested
    let audio_input = if capture_system_audio {
        let ai = writer::create_audio_input("system-audio").map_err(|e| {
            eprintln!("[zureshot] {}", e);
            e
        })?;
        // Verify writer can accept this input before adding
        let can_add: bool = unsafe { objc2::msg_send![&*writer, canAddInput: &*ai] };
        if can_add {
            catch_objc_cmd("addInput(audio)", || unsafe {
                writer.addInput(&ai);
            });
            println!("[zureshot] System audio track added to writer");
            Some(ai)
        } else {
            eprintln!("[zureshot] WARNING: Writer cannot add system audio input — audio will not be recorded");
            None
        }
    } else {
        None
    };

    let mic_input = if capture_microphone {
        let mi = writer::create_audio_input("microphone").map_err(|e| {
            eprintln!("[zureshot] {}", e);
            e
        })?;
        let can_add: bool = unsafe { objc2::msg_send![&*writer, canAddInput: &*mi] };
        if can_add {
            catch_objc_cmd("addInput(mic)", || unsafe {
                writer.addInput(&mi);
            });
            println!("[zureshot] Microphone track added to writer");
            Some(mi)
        } else {
            eprintln!("[zureshot] WARNING: Writer cannot add microphone input — mic will not be recorded");
            None
        }
    } else {
        None
    };

    // Start writing AFTER all inputs are added.
    // AVAssetWriter does not allow adding inputs after startWriting().
    writer::start_writing(&writer).map_err(|e| {
        eprintln!("[zureshot] {}", e);
        e
    })?;

    // Shared paused flag — the capture delegate checks this on every frame
    let paused_flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));

    // Start capture
    let stream = capture::create_and_start(
        &display,
        width,
        height,
        writer.clone(),
        input.clone(),
        audio_input.clone(),
        mic_input.clone(),
        source_rect,
        exclude_windows,
        quality,
        paused_flag.clone(),
        capture_system_audio,
        capture_microphone,
    )
    .map_err(|e| {
        eprintln!("[zureshot] {}", e);
        e
    })?;

    // Update state
    recording.stream = Some(stream);
    recording.writer = Some(writer);
    recording.input = Some(input);
    recording.audio_input = audio_input;
    recording.mic_input = mic_input;
    recording.output_path = Some(path.clone());
    recording.is_recording = true;
    recording.is_paused = false;
    recording.start_time = Some(std::time::Instant::now());
    recording.pause_accumulated = std::time::Duration::ZERO;
    recording.pause_start = None;
    recording.region = region.clone();
    recording.quality = quality;
    recording.output_format = output_format.unwrap_or_else(|| "video".to_string());
    recording.paused_flag = Some(paused_flag);

    println!(
        "[zureshot] Recording started! systemAudio={}, mic={}, audioInput={}, micInput={}",
        capture_system_audio,
        capture_microphone,
        recording.audio_input.is_some(),
        recording.mic_input.is_some()
    );

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

/// Collect SCWindow objects that belong to our app (for exclusion from capture).
/// Matches by the app's process ID.
fn collect_app_windows_to_exclude(
    app: &AppHandle,
    all_windows: &[Retained<objc2_screen_capture_kit::SCWindow>],
) -> Vec<Retained<objc2_screen_capture_kit::SCWindow>> {
    let our_pid = std::process::id() as i32;

    // Collect window labels we own in Tauri
    let our_labels: Vec<String> = app
        .webview_windows()
        .keys()
        .cloned()
        .collect();
    println!(
        "[zureshot] Excluding windows for PID {} (Tauri windows: {:?})",
        our_pid, our_labels
    );

    let mut excluded = Vec::new();
    for w in all_windows {
        let pid = unsafe { w.owningApplication() }
            .map(|app_ref| unsafe { app_ref.processID() })
            .unwrap_or(-1);
        if pid == our_pid {
            let title = unsafe { w.title() }
                .map(|t| t.to_string())
                .unwrap_or_default();
            println!("[zureshot] Excluding window: PID={} title={:?}", pid, title);
            excluded.push(w.clone());
        }
    }
    excluded
}

/// Core logic to stop recording (called from both tray and commands)
pub fn do_stop_recording(app: &AppHandle) -> Result<RecordingResult, String> {
    // Extract all recording state while holding the mutex, then release it
    // BEFORE any blocking operations. Holding the mutex during GCD completion
    // handler waits can deadlock if the handler needs the main thread (which
    // Tauri sync commands also block on).
    let (stream, writer, input, audio_input, mic_input, output_path, duration, output_format) = {
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

        // Take values out — resets state and releases the ObjC references
        // from RecordingState (the locals now own them).
        let stream = recording.stream.take();
        let writer = recording.writer.take();
        let input = recording.input.take();
        let audio_input = recording.audio_input.take();
        let mic_input = recording.mic_input.take();
        let output_path = recording.output_path.take().unwrap_or_default();
        let output_format = std::mem::replace(&mut recording.output_format, "video".to_string());
        recording.is_recording = false;
        recording.is_paused = false;
        recording.start_time = None;
        recording.pause_accumulated = std::time::Duration::ZERO;
        recording.pause_start = None;
        recording.region = None;
        recording.quality = RecordingQuality::Standard;
        recording.paused_flag = None;

        (stream, writer, input, audio_input, mic_input, output_path, duration, output_format)
    }; // ← mutex released here

    println!("[zureshot] Stopping recording after {:.1}s", duration);

    // Close the recording bar and dim overlay windows
    if let Some(win) = app.get_webview_window("recording-bar") {
        let _ = win.destroy();
    }
    if let Some(win) = app.get_webview_window("recording-overlay") {
        let _ = win.destroy();
    }

    // Stop capture — blocks until SCStream confirms stop (no mutex held)
    if let Some(ref stream) = stream {
        println!("[zureshot] Stopping capture stream...");
        capture::stop(stream);
        println!("[zureshot] Capture stream stopped");
    }

    // Brief pause to let the capture dispatch queue fully drain
    std::thread::sleep(std::time::Duration::from_millis(200));

    // Finalize MP4 — writes the moov atom (no mutex held)
    if let (Some(ref writer), Some(ref input)) = (&writer, &input) {
        println!("[zureshot] Finalizing MP4...");
        writer::finalize(
            writer,
            input,
            audio_input.as_deref(),
            mic_input.as_deref(),
        );
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

/// Open the recorded file in Finder
#[tauri::command]
pub async fn reveal_in_finder(path: String) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .args(["-R", &path])
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    Ok(())
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
    if let Some(ref flag) = recording.paused_flag {
        flag.store(true, std::sync::atomic::Ordering::Relaxed);
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
    if let Some(ref flag) = recording.paused_flag {
        flag.store(false, std::sync::atomic::Ordering::Relaxed);
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
    let bar_width = 200.0;
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

/// Refresh the SCStream content filter to exclude all windows belonging to our PID.
/// Called after creating new windows (recording bar, dim overlay) so they don't
/// appear in the captured video.
pub fn refresh_stream_exclusion(app: &AppHandle) -> Result<(), String> {
    // Clone the stream handle while holding the lock briefly
    let stream = {
        let state: tauri::State<'_, Mutex<RecordingState>> = app.state();
        let recording = state.lock().map_err(|e| e.to_string())?;
        recording
            .stream
            .as_ref()
            .cloned()
            .ok_or("No active stream to update")?
    };

    // Get fresh window list (no mutex held — avoids deadlock)
    let (display, all_windows) = capture::get_display_and_windows()
        .map_err(|e| format!("Failed to get windows for exclusion refresh: {}", e))?;

    let exclude_windows = collect_app_windows_to_exclude(app, &all_windows);

    capture::update_stream_filter(&stream, &display, exclude_windows)
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

    // Pass mode=screenshot via URL query parameter
    let window = WebviewWindowBuilder::new(
        app,
        "region-selector",
        WebviewUrl::App("region-selector.html?mode=screenshot".into()),
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

    println!("[zureshot] Screenshot region selector opened");
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
    let (img_w, img_h, file_size) = capture::take_screenshot_region(x, y, width, height, &temp_path)?;

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

    // Use NSPasteboard to copy the image to clipboard (macOS)
    #[cfg(target_os = "macos")]
    {
        // Use osascript to set clipboard to the image file — simple & reliable
        let script = format!(
            "set the clipboard to (read (POSIX file \"{}\") as «class PNGf»)",
            path
        );
        let output = std::process::Command::new("osascript")
            .args(["-e", &script])
            .output()
            .map_err(|e| format!("Failed to run osascript: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("Failed to copy to clipboard: {}", stderr));
        }
    }

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
    .inner_size(260.0, 170.0)
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
