//! Linux platform implementation — XDG Desktop Portal + GStreamer.
//!
//! Target: Ubuntu 24.04 LTS (Noble) x86_64, Wayland (GNOME) primary.
//!
//! Phase 2.5 architecture (pure Rust, in-process):
//!   ashpd (D-Bus) → XDG Portal ScreenCast → PipeWire fd + node_id
//!   gstreamer-rs  → in-process pipeline: pipewiresrc → encoder → mp4mux → filesink
//!   Pause/resume: GstPipeline PAUSED ↔ PLAYING (no segments, no ffmpeg)
//!   Encoding: auto-detect VA-API/NVENC hardware, fallback to x264

pub mod capture;
pub mod portal;
pub mod writer;

use std::os::fd::AsRawFd;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tauri::AppHandle;

use super::{RecordingQuality, StartRecordingConfig};

// ── RecordingHandle ──────────────────────────────────────────────────

/// Owns all Linux-specific recording state.
///
/// Phase 2.5: dramatically simplified vs Phase 2.
///   - No segment files, no segment counter, no ffmpeg concatenation
///   - Pause/resume via GStreamer native state changes
///   - Portal session kept alive via ashpd + tokio runtime
///
/// Created by `start_recording()`. Stored in `RecordingState` behind
/// an external `Mutex`, so methods take `&self` with interior mutability.
pub struct RecordingHandle {
    /// In-process GStreamer pipeline (None if stopped).
    pipeline: Mutex<Option<writer::GstPipeline>>,
    /// XDG Portal session (keeps PipeWire stream alive).
    session: Mutex<Option<portal::ScreencastSession>>,
    /// Shared paused flag.
    paused_flag: Arc<AtomicBool>,
    /// Final output file path.
    output_path: String,
}

// SAFETY: All interior state is behind Mutex or atomic types.
unsafe impl Send for RecordingHandle {}
unsafe impl Sync for RecordingHandle {}

impl RecordingHandle {
    /// Stop the recording pipeline (sends EOS → clean MP4).
    pub fn stop_capture(&self) {
        println!("[zureshot-linux] Stopping capture...");
        let mut pipeline_guard = self.pipeline.lock().unwrap();
        if let Some(ref pipeline) = *pipeline_guard {
            if let Err(e) = pipeline.stop() {
                println!("[zureshot-linux] Warning: stop error: {e}");
            }
        }
        *pipeline_guard = None;
        println!("[zureshot-linux] Capture stopped");
    }

    /// Finalize the output file: close portal session, release PipeWire.
    pub fn finalize(&self) {
        println!("[zureshot-linux] Finalizing recording: {}", self.output_path);

        // Close the portal session (releases PipeWire stream)
        let session = self.session.lock().unwrap().take();
        if let Some(session) = session {
            session.close();
        }

        println!("[zureshot-linux] Recording finalized: {}", self.output_path);
    }

    /// Pause recording: GStreamer pipeline PLAYING → PAUSED.
    ///
    /// Instant — no segment files, no subprocess teardown.
    pub fn pause(&self) {
        self.paused_flag.store(true, Ordering::Relaxed);
        println!("[zureshot-linux] Pausing recording...");

        let pipeline_guard = self.pipeline.lock().unwrap();
        if let Some(ref pipeline) = *pipeline_guard {
            if let Err(e) = pipeline.pause() {
                println!("[zureshot-linux] Warning: pause error: {e}");
            }
        }
        println!("[zureshot-linux] Recording paused");
    }

    /// Resume recording: GStreamer pipeline PAUSED → PLAYING.
    ///
    /// Instant — no new subprocess, no new segment file.
    pub fn resume(&self) {
        self.paused_flag.store(false, Ordering::Relaxed);
        println!("[zureshot-linux] Resuming recording...");

        let pipeline_guard = self.pipeline.lock().unwrap();
        if let Some(ref pipeline) = *pipeline_guard {
            if let Err(e) = pipeline.resume() {
                println!("[zureshot-linux] Warning: resume error: {e}");
            }
        }
        println!("[zureshot-linux] Recording resumed");
    }

    /// Refresh window exclusion filter (no-op on Linux — Portal handles this).
    pub fn refresh_exclusion(&self, _app: &AppHandle) -> Result<(), String> {
        Ok(())
    }
}

// ── Recording lifecycle ──────────────────────────────────────────────

/// Set up the capture pipeline and begin recording.
///
/// Flow: ashpd → XDG Portal → PipeWire fd + node_id → GStreamer pipeline
pub fn start_recording(
    _app: &AppHandle,
    config: StartRecordingConfig,
) -> Result<RecordingHandle, String> {
    println!(
        "[zureshot-linux] start_recording: path={}, region={:?}, quality={:?}, audio={}, mic={}",
        config.output_path, config.region, config.quality,
        config.capture_system_audio, config.capture_microphone
    );

    // ── Step 1: Request screen capture via XDG Portal (ashpd) ──
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

    // Region crop
    let region = config.region.as_ref().map(|r| {
        (r.x as i32, r.y as i32, r.width as i32, r.height as i32)
    });

    // Compute output dimensions and bitrate
    let (out_w, out_h) = if let Some((_, _, w, h)) = region {
        (w as u32, h as u32)
    } else {
        (src_width, src_height)
    };

    // Detect best encoder for adaptive bitrate
    gstreamer::init().map_err(|e| format!("GStreamer init: {e}"))?;
    let encoder_info = writer::detect_best_encoder();
    let bitrate_kbps = writer::compute_bitrate(out_w, out_h, &config.quality, &encoder_info);

    // ── Step 3: Start in-process GStreamer pipeline ──
    let pipeline_config = writer::PipelineConfig {
        node_id: session.node_id,
        fd: session.fd.as_raw_fd(),
        output_path: config.output_path.clone(),
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
        "[zureshot-linux] Recording started: {}x{} @ {}fps, {}kbps, encoder={} ({}), node={}",
        out_w, out_h, fps, bitrate_kbps,
        encoder_info.name, encoder_info.description,
        session.node_id
    );

    Ok(RecordingHandle {
        pipeline: Mutex::new(Some(pipeline)),
        session: Mutex::new(Some(session)),
        paused_flag: Arc::new(AtomicBool::new(false)),
        output_path: config.output_path,
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
    let parent = std::path::Path::new(path)
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| ".".to_string());

    std::process::Command::new("xdg-open")
        .arg(&parent)
        .spawn()
        .map_err(|e| format!("Failed to open file manager: {e}"))?;
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
                .map_err(|e| format!("Failed to read image file: {e}"))?;
            if let Some(ref mut stdin) = child.stdin {
                use std::io::Write;
                stdin
                    .write_all(&file_bytes)
                    .map_err(|e| format!("Failed to write to wl-copy: {e}"))?;
            }
            let status = child
                .wait()
                .map_err(|e| format!("wl-copy failed: {e}"))?;
            if status.success() {
                return Ok(());
            }
        }
        Err(_) => {}
    }

    // Fallback: xclip (X11)
    let output = std::process::Command::new("xclip")
        .args(["-selection", "clipboard", "-target", "image/png", "-i", path])
        .output()
        .map_err(|e| format!("Neither wl-copy nor xclip available: {e}"))?;

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
            "--title", title,
            "--text", message,
            "--ok-label", accept,
            "--cancel-label", cancel,
        ])
        .output();

    match result {
        Ok(output) => output.status.success(),
        Err(_) => {
            // zenity not available — try kdialog
            std::process::Command::new("kdialog")
                .args([
                    "--title", title,
                    "--yesno", message,
                    "--yes-label", accept,
                    "--no-label", cancel,
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
        .map_err(|e| format!("Failed to open folder: {e}"))?;
    Ok(())
}

// ── Autostart (Launch at Login) ──────────────────────────────────────

/// Path to the autostart .desktop file.
fn autostart_desktop_path() -> PathBuf {
    let config_dir = dirs::config_dir().unwrap_or_else(|| PathBuf::from("~/.config"));
    config_dir.join("autostart").join("zureshot.desktop")
}

/// Check whether Zureshot is configured to launch at login.
pub fn get_autostart_enabled() -> bool {
    autostart_desktop_path().exists()
}

/// Enable or disable launch at login by creating/removing the .desktop file.
pub fn set_autostart_enabled(enabled: bool) {
    let path = autostart_desktop_path();
    if enabled {
        let dir = path.parent().unwrap();
        let _ = std::fs::create_dir_all(dir);

        let exec = std::env::current_exe()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| "zureshot".to_string());

        let content = format!(
            "[Desktop Entry]\n\
             Type=Application\n\
             Name=Zureshot\n\
             Exec={exec}\n\
             Icon=zureshot\n\
             Comment=Screen capture and recording tool\n\
             Terminal=false\n\
             Categories=Utility;Graphics;\n\
             StartupNotify=false\n\
             X-GNOME-Autostart-enabled=true\n"
        );
        match std::fs::write(&path, content) {
            Ok(()) => println!("[zureshot-linux] Autostart enabled: {}", path.display()),
            Err(e) => eprintln!("[zureshot-linux] Failed to write autostart file: {e}"),
        }
    } else {
        match std::fs::remove_file(&path) {
            Ok(()) => println!("[zureshot-linux] Autostart disabled"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => eprintln!("[zureshot-linux] Failed to remove autostart file: {e}"),
        }
    }
}

// ── First-run permission guide ───────────────────────────────────────

/// Show a first-run dialog explaining Linux screen capture permissions.
pub fn show_first_run_guide() {
    show_info_dialog(
        "Welcome to Zureshot",
        "Zureshot uses XDG Desktop Portal for screen capture.\n\n\
         When you first record or take a screenshot, your desktop \
         environment will ask you to choose which screen or window \
         to share. This is a standard Linux security feature.\n\n\
         Tip: On GNOME, you can select \"Monitor\" to share your \
         entire screen, or pick a specific window.",
    );
}
