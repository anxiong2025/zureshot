#![allow(non_snake_case, dead_code)]
//! Zureshot — High-performance screen recorder for macOS.
//!
//! Zero-copy recording pipeline:
//!   ScreenCaptureKit → CMSampleBuffer (IOSurface, GPU) → VideoToolbox H.264 → MP4
//!
//! Usage:
//!   zureshot [output.mp4]
//!   Press Ctrl+C to stop recording.

mod capture;
mod writer;

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

static RUNNING: AtomicBool = AtomicBool::new(true);

extern "C" fn signal_handler(_sig: libc::c_int) {
    RUNNING.store(false, Ordering::SeqCst);
}

fn main() {
    let output_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "recording.mp4".into());

    // Remove existing file (AVAssetWriter won't overwrite)
    let _ = std::fs::remove_file(&output_path);

    eprintln!("[zureshot] Initializing screen capture...");

    // ── 1. Get main display ──
    let display = capture::get_main_display();
    let (width, height) = capture::display_size(&display);
    eprintln!("[zureshot] Display: {}x{}", width, height);

    // ── 2. Create H.264 writer ──
    eprintln!("[zureshot] Creating H.264 writer...");
    let (asset_writer, writer_input) = writer::create_writer(&output_path, width, height);
    eprintln!(
        "[zureshot] Encoder: H.264 High Profile | Bitrate: {} Mbps | Keyframe: 1s",
        if width * height >= 3840 * 2160 { 22 }
        else if width * height >= 2560 * 1440 { 14 }
        else { 8 }
    );

    // ── 3. Create capture stream + start ──
    let stream = capture::create_and_start(
        &display,
        width,
        height,
        asset_writer.clone(),
        writer_input.clone(),
    );

    eprintln!("[zureshot] Recording... Press Ctrl+C to stop");
    let start_time = Instant::now();

    // ── 4. Install signal handler ──
    unsafe {
        libc::signal(libc::SIGINT, signal_handler as libc::sighandler_t);
        libc::signal(libc::SIGTERM, signal_handler as libc::sighandler_t);
    }

    // ── 5. Wait for stop signal ──
    while RUNNING.load(Ordering::SeqCst) {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    let duration = start_time.elapsed();
    eprintln!("\n[zureshot] Stopping capture...");

    // ── 6. Stop capture ──
    capture::stop(&stream);

    // ── 7. Finalize MP4 ──
    eprintln!("[zureshot] Finalizing MP4...");
    writer::finalize(&asset_writer, &writer_input);

    // ── 8. Report ──
    let file_size = std::fs::metadata(&output_path)
        .map(|m| m.len())
        .unwrap_or(0);
    eprintln!(
        "[zureshot] Done! {} | {:.1}s | {:.1} MB",
        output_path,
        duration.as_secs_f64(),
        file_size as f64 / 1_048_576.0,
    );
}
