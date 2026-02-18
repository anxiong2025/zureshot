//! Linux platform implementation — XDG Desktop Portal + GStreamer.
//!
//! Target: Ubuntu 24.04 LTS (Noble) x86_64, Wayland (GNOME) primary.
//!
//! Recording architecture:
//!   XDG Portal ScreenCast → PipeWire node_id
//!   gst-launch-1.0: pipewiresrc → x264enc → mp4mux → filesink
//!   Pause/resume: segment-based recording + ffmpeg concatenation

pub mod capture;
pub mod portal;
pub mod writer;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use tauri::AppHandle;

use super::{CaptureRegion, RecordingQuality, StartRecordingConfig};

// ── RecordingHandle ──────────────────────────────────────────────────

/// Owns all Linux-specific recording state.
///
/// Created by `start_recording()`. The handle is stored in `RecordingState`
/// behind an external `Mutex`, so all methods take `&self` and use interior
/// mutability where needed.
pub struct RecordingHandle {
    /// Active GStreamer pipeline (None if paused or stopped).
    pipeline: Mutex<Option<writer::GstPipeline>>,
    /// XDG Portal session info.
    session: portal::ScreencastSession,
    /// Completed segment files (for pause/resume concatenation).
    segments: Mutex<Vec<PathBuf>>,
    /// Counter for generating unique segment filenames.
    segment_counter: AtomicU32,
    /// Shared paused flag.
    paused_flag: Arc<AtomicBool>,
    /// Final output file path.
    output_path: String,
    /// Recording parameters (needed to restart pipeline on resume).
    fps: i32,
    bitrate_kbps: i32,
    region: Option<(i32, i32, i32, i32)>,
    capture_system_audio: bool,
    capture_microphone: bool,
}

// SAFETY: All interior state is behind Mutex or atomic types.
unsafe impl Send for RecordingHandle {}
unsafe impl Sync for RecordingHandle {}

impl RecordingHandle {
    /// Stop the recording pipeline (sends SIGINT → EOS → clean MP4).
    pub fn stop_capture(&self) {
        println!("[zureshot-linux] Stopping capture...");
        let mut pipeline_guard = self.pipeline.lock().unwrap();
        if let Some(ref mut pipeline) = *pipeline_guard {
            if let Err(e) = pipeline.stop() {
                println!("[zureshot-linux] Warning: stop error: {}", e);
            }
            // Save this segment
            let seg_path = pipeline.output_path().to_path_buf();
            if seg_path.exists() {
                self.segments.lock().unwrap().push(seg_path);
            }
        }
        *pipeline_guard = None;
        println!("[zureshot-linux] Capture stopped");
    }

    /// Finalize the output file: concatenate segments, close portal session.
    pub fn finalize(&self) {
        println!("[zureshot-linux] Finalizing recording...");

        // Concatenate segments into the final output
        let segments = self.segments.lock().unwrap();
        if !segments.is_empty() {
            match writer::concatenate_segments(&segments, &self.output_path) {
                Ok(()) => println!("[zureshot-linux] Finalized: {}", self.output_path),
                Err(e) => println!("[zureshot-linux] Finalize error: {}", e),
            }
        }

        // Close the portal session (releases PipeWire stream)
        portal::close_session(&self.session.session_handle);
    }

    /// Pause recording: stop current GStreamer pipeline (saves segment).
    ///
    /// The portal session stays open so the PipeWire node remains valid.
    /// On resume, a new gst-launch process connects to the same node.
    pub fn pause(&self) {
        self.paused_flag.store(true, Ordering::Relaxed);
        println!("[zureshot-linux] Pausing recording...");

        let mut pipeline_guard = self.pipeline.lock().unwrap();
        if let Some(ref mut pipeline) = *pipeline_guard {
            if let Err(e) = pipeline.stop() {
                println!("[zureshot-linux] Warning: pause stop error: {}", e);
            }
            let seg_path = pipeline.output_path().to_path_buf();
            if seg_path.exists() {
                self.segments.lock().unwrap().push(seg_path);
            }
        }
        *pipeline_guard = None;
        println!("[zureshot-linux] Recording paused (segment saved)");
    }

    /// Resume recording: start a new GStreamer pipeline (new segment).
    pub fn resume(&self) {
        self.paused_flag.store(false, Ordering::Relaxed);
        println!("[zureshot-linux] Resuming recording...");

        let seg_idx = self.segment_counter.fetch_add(1, Ordering::Relaxed);
        let seg_output = writer::segment_path(&self.output_path, seg_idx);

        let config = writer::PipelineConfig {
            node_id: self.session.node_id,
            output_path: seg_output.to_string_lossy().to_string(),
            fps: self.fps,
            bitrate_kbps: self.bitrate_kbps,
            source_width: self.session.width,
            source_height: self.session.height,
            region: self.region,
            capture_system_audio: self.capture_system_audio,
            capture_mic: self.capture_microphone,
        };

        match writer::start_pipeline(&config) {
            Ok(pipeline) => {
                *self.pipeline.lock().unwrap() = Some(pipeline);
                println!("[zureshot-linux] Recording resumed (new segment)");
            }
            Err(e) => {
                println!("[zureshot-linux] Failed to resume: {}", e);
            }
        }
    }

    /// Refresh window exclusion filter (no-op on Linux — Portal handles this).
    pub fn refresh_exclusion(&self, _app: &AppHandle) -> Result<(), String> {
        Ok(())
    }
}

// ── Recording lifecycle ──────────────────────────────────────────────

/// Set up the capture pipeline and begin recording.
///
/// Flow: XDG Portal → PipeWire node_id → gst-launch-1.0 pipeline
pub fn start_recording(
    _app: &AppHandle,
    config: StartRecordingConfig,
) -> Result<RecordingHandle, String> {
    println!(
        "[zureshot-linux] start_recording: path={}, region={:?}, quality={:?}, audio={}, mic={}",
        config.output_path, config.region, config.quality,
        config.capture_system_audio, config.capture_microphone
    );

    // ── Step 1: Request screen capture via XDG Portal ──
    // TODO: store and reuse restore_token across sessions
    let session = portal::request_screencast(None)?;

    // ── Step 2: Determine recording parameters ──
    let fps = match config.quality {
        RecordingQuality::Standard => 30,
        RecordingQuality::High => 60,
    };

    // Use portal-reported dimensions or defaults
    let src_width = session.width.unwrap_or(1920);
    let src_height = session.height.unwrap_or(1080);

    // Region crop (convert CSS pixels to capture pixels)
    // For MVP, assume 1:1 scaling. HiDPI adjustment can be added in Phase 3.
    let region = config.region.as_ref().map(|r| {
        (r.x as i32, r.y as i32, r.width as i32, r.height as i32)
    });

    // Compute bitrate based on output dimensions
    let (out_w, out_h) = if let Some((_, _, w, h)) = region {
        (w as u32, h as u32)
    } else {
        (src_width, src_height)
    };
    let bitrate_kbps = writer::compute_bitrate(out_w, out_h, &config.quality);

    // ── Step 3: Start GStreamer pipeline ──
    let seg_idx = 0u32;
    let seg_output = writer::segment_path(&config.output_path, seg_idx);

    let pipeline_config = writer::PipelineConfig {
        node_id: session.node_id,
        output_path: seg_output.to_string_lossy().to_string(),
        fps,
        bitrate_kbps,
        source_width: Some(src_width),
        source_height: Some(src_height),
        region,
        capture_system_audio: config.capture_system_audio,
        capture_mic: config.capture_microphone,
    };

    let pipeline = writer::start_pipeline(&pipeline_config)?;

    println!(
        "[zureshot-linux] Recording started: {}x{} @ {}fps, {}kbps, node={}",
        out_w, out_h, fps, bitrate_kbps, session.node_id
    );

    Ok(RecordingHandle {
        pipeline: Mutex::new(Some(pipeline)),
        session,
        segments: Mutex::new(Vec::new()),
        segment_counter: AtomicU32::new(seg_idx + 1),
        paused_flag: Arc::new(AtomicBool::new(false)),
        output_path: config.output_path,
        fps,
        bitrate_kbps,
        region,
        capture_system_audio: config.capture_system_audio,
        capture_microphone: config.capture_microphone,
    })
}

/// Take a screenshot of a specific screen region.
pub fn take_screenshot_region(
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    output_path: &str,
) -> Result<(usize, usize, u64), String> {
    capture::take_screenshot_region(x, y, width, height, output_path)
}

// ── System integration ───────────────────────────────────────────────

/// Reveal a file in the default file manager.
pub fn reveal_file(path: &str) -> Result<(), String> {
    // xdg-open opens the parent directory; there's no standard "select file"
    // equivalent across all Linux file managers.
    let parent = std::path::Path::new(path)
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| ".".to_string());

    std::process::Command::new("xdg-open")
        .arg(&parent)
        .spawn()
        .map_err(|e| format!("Failed to open file manager: {}", e))?;
    Ok(())
}

/// Copy a PNG image to the clipboard.
pub fn copy_image_to_clipboard(path: &str) -> Result<(), String> {
    // Try wl-copy first (Wayland), fall back to xclip (X11)
    let wl_result = std::process::Command::new("wl-copy")
        .args(["--type", "image/png"])
        .stdin(std::process::Stdio::piped())
        .spawn();

    match wl_result {
        Ok(mut child) => {
            let file_bytes = std::fs::read(path)
                .map_err(|e| format!("Failed to read image file: {}", e))?;
            if let Some(ref mut stdin) = child.stdin {
                use std::io::Write;
                stdin
                    .write_all(&file_bytes)
                    .map_err(|e| format!("Failed to write to wl-copy: {}", e))?;
            }
            let status = child
                .wait()
                .map_err(|e| format!("wl-copy failed: {}", e))?;
            if status.success() {
                return Ok(());
            }
        }
        Err(_) => {
            // wl-copy not found, try xclip
        }
    }

    // Fallback: xclip (X11)
    let output = std::process::Command::new("xclip")
        .args(["-selection", "clipboard", "-target", "image/png", "-i", path])
        .output()
        .map_err(|e| format!("Neither wl-copy nor xclip available: {}", e))?;

    if !output.status.success() {
        return Err(format!(
            "xclip failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(())
}

/// Show a confirmation dialog using zenity. Returns `true` if user clicked OK.
pub fn show_confirm_dialog(title: &str, message: &str, accept: &str, cancel: &str) -> bool {
    let result = std::process::Command::new("zenity")
        .args([
            "--question",
            "--title",
            title,
            "--text",
            message,
            "--ok-label",
            accept,
            "--cancel-label",
            cancel,
        ])
        .output();

    match result {
        Ok(output) => output.status.success(),
        Err(_) => {
            // zenity not available — try kdialog
            std::process::Command::new("kdialog")
                .args([
                    "--title",
                    title,
                    "--yesno",
                    message,
                    "--yes-label",
                    accept,
                    "--no-label",
                    cancel,
                ])
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
        }
    }
}

/// Show an info dialog using zenity.
pub fn show_info_dialog(title: &str, message: &str) {
    let result = std::process::Command::new("zenity")
        .args(["--info", "--title", title, "--text", message])
        .output();

    if result.is_err() {
        // Fallback to kdialog
        let _ = std::process::Command::new("kdialog")
            .args(["--title", title, "--msgbox", message])
            .output();
    }
}

/// Open a folder in the default file manager.
pub fn open_folder(path: &str) -> Result<(), String> {
    std::process::Command::new("xdg-open")
        .arg(path)
        .spawn()
        .map_err(|e| format!("Failed to open folder: {}", e))?;
    Ok(())
}
