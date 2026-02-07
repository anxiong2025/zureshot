//! Tauri commands for screen recording.
//!
//! These functions are exposed to the frontend via Tauri's IPC mechanism.

use crate::capture;
use crate::writer;
use objc2::rc::Retained;
use objc2_av_foundation::{AVAssetWriter, AVAssetWriterInput};
use objc2_screen_capture_kit::SCStream;
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager};

/// Recording state shared across commands
pub struct RecordingState {
    pub stream: Option<Retained<SCStream>>,
    pub writer: Option<Retained<AVAssetWriter>>,
    pub input: Option<Retained<AVAssetWriterInput>>,
    pub output_path: Option<String>,
    pub is_recording: bool,
    pub start_time: Option<std::time::Instant>,
}

impl Default for RecordingState {
    fn default() -> Self {
        Self {
            stream: None,
            writer: None,
            input: None,
            output_path: None,
            is_recording: false,
            start_time: None,
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
    pub duration_secs: f64,
    pub output_path: Option<String>,
}

/// Result of stopping a recording
#[derive(Clone, Serialize, Deserialize)]
pub struct RecordingResult {
    pub path: String,
    pub duration_secs: f64,
    pub file_size_bytes: u64,
}

/// Core logic to start recording (called from both tray and commands)
pub fn do_start_recording(app: &AppHandle, output_path: Option<String>) -> Result<String, String> {
    let state: tauri::State<'_, Mutex<RecordingState>> = app.state();
    let mut recording = state.lock().map_err(|e: std::sync::PoisonError<_>| e.to_string())?;

    if recording.is_recording {
        return Err("Recording already in progress".to_string());
    }

    // Generate output path if not provided
    let path = output_path.unwrap_or_else(|| {
        let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
        let downloads = dirs::download_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        downloads
            .join(format!("zureshot_{}.mp4", timestamp))
            .to_string_lossy()
            .to_string()
    });

    // Remove existing file (AVAssetWriter won't overwrite)
    let _ = std::fs::remove_file(&path);

    println!("[zureshot] Starting recording to: {}", path);

    // Get main display
    let display = capture::get_main_display().map_err(|e| {
        eprintln!("[zureshot] {}", e);
        e
    })?;
    let (width, height) = capture::display_size(&display);
    println!("[zureshot] Display: {}x{}", width, height);

    // Create H.264 writer
    let (writer, input) = writer::create_writer(&path, width, height).map_err(|e| {
        eprintln!("[zureshot] {}", e);
        e
    })?;

    // Start capture
    let stream = capture::create_and_start(&display, width, height, writer.clone(), input.clone())
        .map_err(|e| {
            eprintln!("[zureshot] {}", e);
            e
        })?;

    // Update state
    recording.stream = Some(stream);
    recording.writer = Some(writer);
    recording.input = Some(input);
    recording.output_path = Some(path.clone());
    recording.is_recording = true;
    recording.start_time = Some(std::time::Instant::now());

    println!("[zureshot] Recording started!");

    // Emit event to frontend
    let _ = app.emit("recording-started", &path);

    Ok(path)
}

/// Core logic to stop recording (called from both tray and commands)
pub fn do_stop_recording(app: &AppHandle) -> Result<RecordingResult, String> {
    // Extract all recording state while holding the mutex, then release it
    // BEFORE any blocking operations. Holding the mutex during GCD completion
    // handler waits can deadlock if the handler needs the main thread (which
    // Tauri sync commands also block on).
    let (stream, writer, input, output_path, duration) = {
        let state: tauri::State<'_, Mutex<RecordingState>> = app.state();
        let mut recording = state.lock().map_err(|e: std::sync::PoisonError<_>| e.to_string())?;

        if !recording.is_recording {
            return Err("No recording in progress".to_string());
        }

        let duration = recording
            .start_time
            .map(|t: std::time::Instant| t.elapsed().as_secs_f64())
            .unwrap_or(0.0);

        // Take values out — resets state and releases the ObjC references
        // from RecordingState (the locals now own them).
        let stream = recording.stream.take();
        let writer = recording.writer.take();
        let input = recording.input.take();
        let output_path = recording.output_path.take().unwrap_or_default();
        recording.is_recording = false;
        recording.start_time = None;

        (stream, writer, input, output_path, duration)
    }; // ← mutex released here

    println!("[zureshot] Stopping recording after {:.1}s", duration);

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
        writer::finalize(writer, input);
    }

    let file_size = std::fs::metadata(&output_path).map(|m| m.len()).unwrap_or(0);

    let result = RecordingResult {
        path: output_path.clone(),
        duration_secs: duration,
        file_size_bytes: file_size,
    };

    // Emit event to frontend with result
    let _ = app.emit("recording-stopped", &result);

    println!(
        "[zureshot] Recording complete: {} ({:.1}s, {:.1} MB)",
        output_path,
        duration,
        file_size as f64 / 1_048_576.0
    );

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
        do_start_recording(&app_clone, output_path).map(|_| ())
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
        duration_secs: recording
            .start_time
            .map(|t| t.elapsed().as_secs_f64())
            .unwrap_or(0.0),
        output_path: recording.output_path.clone(),
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
    dirs::download_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .to_string_lossy()
        .to_string()
}
