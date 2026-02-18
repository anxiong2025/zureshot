//! macOS platform implementation — ScreenCaptureKit + AVAssetWriter.

pub mod capture;
pub mod writer;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use objc2::rc::Retained;
use objc2_av_foundation::{AVAssetWriter, AVAssetWriterInput};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_screen_capture_kit::{SCStream, SCWindow};
use tauri::{AppHandle, Manager};

use super::StartRecordingConfig;

// ── RecordingHandle ──────────────────────────────────────────────────

/// Owns all macOS-specific recording state.
///
/// Created by `start_recording()`, consumed by `stop_capture()` + `finalize()`.
pub struct RecordingHandle {
    pub(crate) stream: Retained<SCStream>,
    pub(crate) writer: Retained<AVAssetWriter>,
    pub(crate) input: Retained<AVAssetWriterInput>,
    pub(crate) audio_input: Option<Retained<AVAssetWriterInput>>,
    pub(crate) mic_input: Option<Retained<AVAssetWriterInput>>,
    pub(crate) paused_flag: Arc<AtomicBool>,
}

// SAFETY: The ObjC objects inside are thread-safe. Access is serialized
// via the Mutex<RecordingState> in commands.rs.
unsafe impl Send for RecordingHandle {}
unsafe impl Sync for RecordingHandle {}

impl RecordingHandle {
    /// Stop the SCStream capture (blocks until confirmed).
    pub fn stop_capture(&self) {
        println!("[zureshot] Stopping capture stream...");
        capture::stop(&self.stream);
        println!("[zureshot] Capture stream stopped");
    }

    /// Finalize the MP4 file (writes moov atom).
    pub fn finalize(&self) {
        println!("[zureshot] Finalizing MP4...");
        writer::finalize(
            &self.writer,
            &self.input,
            self.audio_input.as_deref(),
            self.mic_input.as_deref(),
        );
    }

    /// Set the paused flag — capture delegate will drop frames.
    pub fn pause(&self) {
        self.paused_flag.store(true, Ordering::Relaxed);
    }

    /// Clear the paused flag — frames start being written again.
    pub fn resume(&self) {
        self.paused_flag.store(false, Ordering::Relaxed);
    }

    /// Update the SCStream content filter to exclude all windows belonging
    /// to our PID. Called after creating new Tauri windows (recording bar,
    /// dim overlay) so they don't appear in the captured video.
    pub fn refresh_exclusion(&self, app: &AppHandle) -> Result<(), String> {
        let (display, all_windows) = capture::get_display_and_windows()
            .map_err(|e| format!("Failed to get windows for exclusion refresh: {}", e))?;
        let exclude_windows = collect_app_windows_to_exclude(app, &all_windows);
        capture::update_stream_filter(&self.stream, &display, exclude_windows)
    }
}

// ── Recording lifecycle ──────────────────────────────────────────────

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

/// Set up the entire capture pipeline and begin recording.
///
/// This handles: display detection → dimension calculation → writer creation
/// → audio inputs → capture start → window exclusion.
pub fn start_recording(
    app: &AppHandle,
    config: StartRecordingConfig,
) -> Result<RecordingHandle, String> {
    let path = &config.output_path;

    // Remove existing file (AVAssetWriter won't overwrite)
    let _ = std::fs::remove_file(path);

    println!("[zureshot] Starting recording to: {}", path);

    // Get display and windows for potential exclusion
    let (display, all_windows) = capture::get_display_and_windows().map_err(|e| {
        eprintln!("[zureshot] {}", e);
        e
    })?;
    let (phys_width, phys_height, retina_scale) = capture::display_physical_size(&display);
    println!(
        "[zureshot] Display: {}x{} physical, scale={}",
        phys_width, phys_height, retina_scale
    );

    // Determine output dimensions and source rect
    let (width, height, source_rect) = if let Some(ref rgn) = config.region {
        let pixel_w = (rgn.width * retina_scale) as usize;
        let pixel_h = (rgn.height * retina_scale) as usize;
        // Ensure even dimensions for HEVC
        let pixel_w = if pixel_w % 2 != 0 { pixel_w + 1 } else { pixel_w };
        let pixel_h = if pixel_h % 2 != 0 { pixel_h + 1 } else { pixel_h };

        let rect = CGRect::new(
            CGPoint::new(rgn.x, rgn.y),
            CGSize::new(rgn.width, rgn.height),
        );
        println!(
            "[zureshot] Region: css({},{} {}x{}) → pixels({}x{}) scale={} quality={:?}",
            rgn.x, rgn.y, rgn.width, rgn.height, pixel_w, pixel_h, retina_scale, config.quality
        );
        (pixel_w, pixel_h, Some(rect))
    } else {
        println!(
            "[zureshot] Full screen: {}x{} (physical, {}x Retina) quality={:?}",
            phys_width, phys_height, retina_scale, config.quality
        );
        (phys_width, phys_height, None)
    };

    // Collect windows to exclude (our own app windows)
    let exclude_windows = collect_app_windows_to_exclude(app, &all_windows);

    // Create HEVC writer
    let (w, input) = writer::create_writer(path, width, height, config.quality).map_err(|e| {
        eprintln!("[zureshot] {}", e);
        e
    })?;

    // Create audio writer inputs if requested
    let audio_input = if config.capture_system_audio {
        let ai = writer::create_audio_input("system-audio").map_err(|e| {
            eprintln!("[zureshot] {}", e);
            e
        })?;
        let can_add: bool = unsafe { objc2::msg_send![&*w, canAddInput: &*ai] };
        if can_add {
            catch_objc_cmd("addInput(audio)", || unsafe { w.addInput(&ai) });
            println!("[zureshot] System audio track added to writer");
            Some(ai)
        } else {
            eprintln!("[zureshot] WARNING: Writer cannot add system audio input");
            None
        }
    } else {
        None
    };

    let mic_input = if config.capture_microphone {
        let mi = writer::create_audio_input("microphone").map_err(|e| {
            eprintln!("[zureshot] {}", e);
            e
        })?;
        let can_add: bool = unsafe { objc2::msg_send![&*w, canAddInput: &*mi] };
        if can_add {
            catch_objc_cmd("addInput(mic)", || unsafe { w.addInput(&mi) });
            println!("[zureshot] Microphone track added to writer");
            Some(mi)
        } else {
            eprintln!("[zureshot] WARNING: Writer cannot add microphone input");
            None
        }
    } else {
        None
    };

    // Start writing AFTER all inputs are added
    writer::start_writing(&w).map_err(|e| {
        eprintln!("[zureshot] {}", e);
        e
    })?;

    // Shared paused flag
    let paused_flag = Arc::new(AtomicBool::new(false));

    // Start capture
    let stream = capture::create_and_start(
        &display,
        width,
        height,
        w.clone(),
        input.clone(),
        audio_input.clone(),
        mic_input.clone(),
        source_rect,
        exclude_windows,
        config.quality,
        paused_flag.clone(),
        config.capture_system_audio,
        config.capture_microphone,
    )
    .map_err(|e| {
        eprintln!("[zureshot] {}", e);
        e
    })?;

    println!(
        "[zureshot] Recording started! systemAudio={}, mic={}, audioInput={}, micInput={}",
        config.capture_system_audio,
        config.capture_microphone,
        audio_input.is_some(),
        mic_input.is_some()
    );

    Ok(RecordingHandle {
        stream,
        writer: w,
        input,
        audio_input,
        mic_input,
        paused_flag,
    })
}

/// Take a screenshot of a specific screen region. Returns (width, height, file_size).
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

/// Reveal a file in Finder.
pub fn reveal_file(path: &str) -> Result<(), String> {
    std::process::Command::new("open")
        .args(["-R", path])
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Copy a PNG image to the clipboard using osascript.
pub fn copy_image_to_clipboard(path: &str) -> Result<(), String> {
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
    Ok(())
}

/// Show a native confirmation dialog. Returns `true` if user clicked `accept`.
pub fn show_confirm_dialog(title: &str, message: &str, accept: &str, cancel: &str) -> bool {
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

/// Show a native info dialog.
pub fn show_info_dialog(title: &str, message: &str) {
    let script = format!(
        "display dialog \"{}\" buttons {{\"OK\"}} default button \"OK\" with title \"{}\"",
        message, title
    );
    let _ = std::process::Command::new("osascript")
        .args(["-e", &script])
        .output();
}

/// Open a folder in Finder.
pub fn open_folder(path: &str) -> Result<(), String> {
    std::process::Command::new("open")
        .arg(path)
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}

// ── Autostart (Launch at Login) ──────────────────────────────────────

/// Check whether Zureshot is configured to launch at login.
///
/// Uses `osascript` to query System Events login items.
pub fn get_autostart_enabled() -> bool {
    let output = std::process::Command::new("osascript")
        .args([
            "-e",
            "tell application \"System Events\" to get the name of every login item",
        ])
        .output();

    match output {
        Ok(o) => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            stdout.contains("Zureshot")
        }
        Err(_) => false,
    }
}

/// Enable or disable launch at login.
///
/// Uses `osascript` to add/remove from System Events login items.
pub fn set_autostart_enabled(enabled: bool) {
    if enabled {
        // Find the .app bundle path — typically /Applications/Zureshot.app
        // or the development path
        let app_path = std::env::current_exe()
            .ok()
            .and_then(|p| {
                // Walk up to find the .app bundle
                let mut path = p.as_path();
                while let Some(parent) = path.parent() {
                    if path.extension().and_then(|e| e.to_str()) == Some("app") {
                        return Some(path.to_path_buf());
                    }
                    path = parent;
                }
                None
            })
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| "/Applications/Zureshot.app".to_string());

        let script = format!(
            "tell application \"System Events\" to make login item at end \
             with properties {{path:\"{}\", hidden:false}}",
            app_path
        );
        match std::process::Command::new("osascript")
            .args(["-e", &script])
            .output()
        {
            Ok(o) if o.status.success() => println!("[zureshot] Autostart enabled"),
            Ok(o) => eprintln!(
                "[zureshot] Failed to enable autostart: {}",
                String::from_utf8_lossy(&o.stderr)
            ),
            Err(e) => eprintln!("[zureshot] osascript error: {}", e),
        }
    } else {
        let script = "tell application \"System Events\" to delete login item \"Zureshot\"";
        match std::process::Command::new("osascript")
            .args(["-e", script])
            .output()
        {
            Ok(_) => println!("[zureshot] Autostart disabled"),
            Err(e) => eprintln!("[zureshot] osascript error: {}", e),
        }
    }
}

// ── First-run (no-op on macOS — handled by system permission dialog) ─

/// On macOS, the system handles the screen recording permission prompt
/// automatically, so no custom first-run guide is needed.
pub fn show_first_run_guide() {
    // no-op: macOS shows its own Screen Recording permission dialog
}

// ── Helpers ──────────────────────────────────────────────────────────

/// Collect SCWindow objects that belong to our app (for exclusion from capture).
fn collect_app_windows_to_exclude(
    app: &AppHandle,
    all_windows: &[Retained<SCWindow>],
) -> Vec<Retained<SCWindow>> {
    let our_pid = std::process::id() as i32;

    let our_labels: Vec<String> = app.webview_windows().keys().cloned().collect();
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
