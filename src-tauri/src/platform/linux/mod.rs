//! Linux platform implementation — XDG Desktop Portal + GStreamer.
//!
//! Target: Ubuntu 24.04 LTS (Noble) x86_64, Wayland (GNOME) primary.

pub mod capture;
pub mod writer;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tauri::AppHandle;

use super::{CaptureRegion, RecordingQuality, StartRecordingConfig};

// ── RecordingHandle ──────────────────────────────────────────────────

/// Owns all Linux-specific recording state.
///
/// TODO (Phase 2): Will hold GStreamer pipeline, PipeWire connection, etc.
pub struct RecordingHandle {
    paused_flag: Arc<AtomicBool>,
}

impl RecordingHandle {
    /// Stop the recording pipeline.
    pub fn stop_capture(&self) {
        println!("[zureshot-linux] Stopping capture (TODO: GStreamer pipeline)");
        // TODO Phase 2: stop GStreamer pipeline
    }

    /// Finalize the output file.
    pub fn finalize(&self) {
        println!("[zureshot-linux] Finalizing recording (TODO: GStreamer EOS)");
        // TODO Phase 2: send EOS to GStreamer pipeline, wait for completion
    }

    /// Pause recording — drop incoming frames.
    pub fn pause(&self) {
        self.paused_flag.store(true, Ordering::Relaxed);
    }

    /// Resume recording.
    pub fn resume(&self) {
        self.paused_flag.store(false, Ordering::Relaxed);
    }

    /// Refresh window exclusion filter (no-op on Linux — Portal handles this).
    pub fn refresh_exclusion(&self, _app: &AppHandle) -> Result<(), String> {
        Ok(())
    }
}

// ── Recording lifecycle ──────────────────────────────────────────────

/// Set up the capture pipeline and begin recording.
///
/// TODO (Phase 2): XDG Desktop Portal ScreenCast → PipeWire → GStreamer.
pub fn start_recording(
    _app: &AppHandle,
    config: StartRecordingConfig,
) -> Result<RecordingHandle, String> {
    println!(
        "[zureshot-linux] start_recording: path={}, region={:?}, quality={:?}",
        config.output_path, config.region, config.quality
    );

    // TODO Phase 2: implement
    Err("Screen recording is not yet implemented on Linux. Coming in Phase 2.".into())
}

/// Take a screenshot of a specific screen region using XDG Portal.
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
