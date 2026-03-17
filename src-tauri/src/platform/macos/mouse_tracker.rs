//! Mouse position tracker for recording sessions.
//!
//! Records cursor positions during screen recording at ~30Hz.
//! This data is used by the video editor's Auto Zoom feature to
//! intelligently generate zoom keyframes based on user activity.
//!
//! Uses CoreGraphics CGEvent API for low-overhead mouse tracking.

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::sync::Mutex;

/// A single mouse position sample.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MouseSample {
    /// Seconds since recording start
    pub time: f64,
    /// X position in screen coordinates (logical)
    pub x: f64,
    /// Y position in screen coordinates (logical)
    pub y: f64,
    /// Whether a click occurred at this sample
    pub clicked: bool,
}

/// Mouse track data for an entire recording session.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MouseTrack {
    pub samples: Vec<MouseSample>,
    pub duration_secs: f64,
    pub sample_rate_hz: f64,
}

/// State for the mouse tracker thread.
pub struct MouseTrackerState {
    pub running: Arc<AtomicBool>,
    pub samples: Arc<Mutex<Vec<MouseSample>>>,
    pub start_time: Arc<Mutex<Option<std::time::Instant>>>,
}

impl Default for MouseTrackerState {
    fn default() -> Self {
        Self {
            running: Arc::new(AtomicBool::new(false)),
            samples: Arc::new(Mutex::new(Vec::new())),
            start_time: Arc::new(Mutex::new(None)),
        }
    }
}

/// Start recording mouse positions in a background thread (~30 Hz).
/// Uses CGEventSource to get current mouse position — very low overhead.
pub fn start_mouse_tracking(state: &MouseTrackerState) {
    if state.running.load(Ordering::SeqCst) {
        return; // Already running
    }

    state.running.store(true, Ordering::SeqCst);

    // Clear previous data
    if let Ok(mut samples) = state.samples.lock() {
        samples.clear();
    }
    if let Ok(mut start) = state.start_time.lock() {
        *start = Some(std::time::Instant::now());
    }

    let running = state.running.clone();
    let samples = state.samples.clone();
    let start_time = state.start_time.clone();

    std::thread::spawn(move || {
        println!("[mouse-tracker] Started mouse position tracking at ~30 Hz");

        // Pre-allocate for ~10 minutes at 30Hz
        let interval = std::time::Duration::from_millis(33); // ~30 Hz

        while running.load(Ordering::SeqCst) {
            // Get current mouse position using CoreGraphics
            let (x, y, clicked) = get_mouse_position();

            let time = start_time
                .lock()
                .ok()
                .and_then(|s| s.map(|t| t.elapsed().as_secs_f64()))
                .unwrap_or(0.0);

            if let Ok(mut s) = samples.lock() {
                s.push(MouseSample {
                    time,
                    x,
                    y,
                    clicked,
                });
            }

            std::thread::sleep(interval);
        }

        let count = samples.lock().map(|s| s.len()).unwrap_or(0);
        println!("[mouse-tracker] Stopped. Collected {} samples", count);
    });
}

/// Stop mouse tracking and return the collected track data.
pub fn stop_mouse_tracking(state: &MouseTrackerState) -> MouseTrack {
    state.running.store(false, Ordering::SeqCst);

    // Brief pause to let thread wind down
    std::thread::sleep(std::time::Duration::from_millis(50));

    let samples = state
        .samples
        .lock()
        .map(|s| s.clone())
        .unwrap_or_default();

    let duration = state
        .start_time
        .lock()
        .ok()
        .and_then(|s| s.map(|t| t.elapsed().as_secs_f64()))
        .unwrap_or(0.0);

    MouseTrack {
        samples,
        duration_secs: duration,
        sample_rate_hz: 30.0,
    }
}

/// Save mouse track data alongside the video file.
/// Creates a .mousetrack.json file next to the recording.
pub fn save_mouse_track(video_path: &str, track: &MouseTrack) -> Result<String, String> {
    // Save alongside video but hidden (dot-prefix) to reduce file clutter
    let video = std::path::Path::new(video_path);
    let dir = video.parent().unwrap_or(std::path::Path::new("."));
    let stem = video.file_stem().and_then(|s| s.to_str()).unwrap_or("recording");
    let track_path = dir.join(format!(".{}.mousetrack.json", stem)).to_string_lossy().to_string();

    let json = serde_json::to_string(track)
        .map_err(|e| format!("Failed to serialize mouse track: {}", e))?;

    std::fs::write(&track_path, json)
        .map_err(|e| format!("Failed to write mouse track: {}", e))?;

    println!("[mouse-tracker] Saved {} samples to {}", track.samples.len(), track_path);
    Ok(track_path)
}

/// Load mouse track data for a video file.
pub fn load_mouse_track(video_path: &str) -> Result<MouseTrack, String> {
    let video = std::path::Path::new(video_path);
    let dir = video.parent().unwrap_or(std::path::Path::new("."));
    let stem = video.file_stem().and_then(|s| s.to_str()).unwrap_or("recording");

    // Try hidden path first (new format), then fall back to old path
    let hidden_path = dir.join(format!(".{}.mousetrack.json", stem));
    let legacy_path = format!("{}.mousetrack.json", video_path.trim_end_matches(".mp4"));

    let track_path = if hidden_path.exists() {
        hidden_path.to_string_lossy().to_string()
    } else {
        legacy_path
    };

    let json = std::fs::read_to_string(&track_path)
        .map_err(|e| format!("Mouse track not found: {}", e))?;

    serde_json::from_str(&json)
        .map_err(|e| format!("Failed to parse mouse track: {}", e))
}

/// Analyze mouse track to suggest zoom keyframes (FocuSee-quality).
///
/// Two-pass algorithm:
/// 1. **Click detection**: Clicks are high-priority zoom targets (brief 2x pulse)
/// 2. **Dwell detection**: Mouse staying still = reading/focusing (sustained zoom)
///
/// Uses normalized coordinates (0-1) for resolution independence.
/// Each keyframe includes a `hold` duration so the frontend knows how long
/// to sustain the zoom before transitioning out.
pub fn suggest_zoom_keyframes(track: &MouseTrack) -> Vec<super::editor::ZoomKeyframe> {
    if track.samples.len() < 30 {
        return Vec::new();
    }

    // ── Normalize to 0-1 using full screen bounds ──
    // CGEvent::location() returns logical screen coordinates.
    // We need the main display's logical size to normalize properly.
    let (screen_w, screen_h) = get_main_display_logical_size();

    struct Sample { time: f64, nx: f64, ny: f64, clicked: bool }
    let norm: Vec<Sample> = track.samples.iter().map(|s| Sample {
        time: s.time,
        nx: if screen_w > 0.0 { (s.x / screen_w).clamp(0.0, 1.0) } else { 0.5 },
        ny: if screen_h > 0.0 { (s.y / screen_h).clamp(0.0, 1.0) } else { 0.5 },
        clicked: s.clicked,
    }).collect();

    // ── Pass 1: Click detection ──
    // Each click becomes a zoom keyframe with short hold (snappy zoom pulse).
    // Debounce: merge clicks within 0.4s window.
    struct ZoomCandidate { time: f64, cx: f64, cy: f64, zoom: f64, hold: f64, priority: f64 }
    let mut candidates: Vec<ZoomCandidate> = Vec::new();

    let mut last_click_time = -1.0_f64;
    for s in &norm {
        if s.clicked && (s.time - last_click_time) > 0.4 {
            candidates.push(ZoomCandidate {
                time: s.time,
                cx: s.nx,
                cy: s.ny,
                zoom: 2.0,      // clicks get a crisp 2x zoom
                hold: 0.35,     // brief hold — snap in and out
                priority: 10.0, // clicks are highest priority
            });
            last_click_time = s.time;
        }
    }

    // ── Pass 2: Dwell detection (velocity-based) ──
    // Compute per-sample velocity, then find runs of low velocity.
    let mut velocities: Vec<f64> = vec![0.0];
    for i in 1..norm.len() {
        let dt = (norm[i].time - norm[i-1].time).max(0.001);
        let dx = norm[i].nx - norm[i-1].nx;
        let dy = norm[i].ny - norm[i-1].ny;
        velocities.push((dx * dx + dy * dy).sqrt() / dt);
    }

    // Smooth velocity with 5-sample moving average
    let window = 5;
    let mut smooth_vel: Vec<f64> = vec![0.0; norm.len()];
    for i in 0..norm.len() {
        let lo = i.saturating_sub(window / 2);
        let hi = (i + window / 2 + 1).min(norm.len());
        let sum: f64 = velocities[lo..hi].iter().sum();
        smooth_vel[i] = sum / (hi - lo) as f64;
    }

    // Find low-velocity runs (dwell regions)
    let vel_threshold = 0.06;  // normalized units/second — below this = "still"
    let min_dwell_secs = 0.4;
    let max_dwell_secs = 4.0;

    let mut run_start: Option<usize> = None;
    for i in 0..norm.len() {
        if smooth_vel[i] < vel_threshold {
            if run_start.is_none() { run_start = Some(i); }
        } else {
            if let Some(start) = run_start {
                let dur = norm[i-1].time - norm[start].time;
                if dur >= min_dwell_secs && dur <= max_dwell_secs {
                    // Compute centroid of dwell region
                    let count = (i - start) as f64;
                    let avg_x: f64 = norm[start..i].iter().map(|s| s.nx).sum::<f64>() / count;
                    let avg_y: f64 = norm[start..i].iter().map(|s| s.ny).sum::<f64>() / count;

                    // Bounding box tightness → zoom level
                    let bb_w = norm[start..i].iter().map(|s| s.nx).fold(f64::MIN, f64::max)
                             - norm[start..i].iter().map(|s| s.nx).fold(f64::MAX, f64::min);
                    let bb_h = norm[start..i].iter().map(|s| s.ny).fold(f64::MIN, f64::max)
                             - norm[start..i].iter().map(|s| s.ny).fold(f64::MAX, f64::min);
                    let spread = (bb_w * bb_w + bb_h * bb_h).sqrt();
                    let tightness = 1.0 - (spread / 0.08).min(1.0);

                    let zoom = 1.5 + tightness * 1.0; // 1.5x-2.5x
                    let hold = (dur * 0.6).clamp(0.4, 2.0); // hold proportional to dwell

                    let center_time = (norm[start].time + norm[i-1].time) / 2.0;
                    candidates.push(ZoomCandidate {
                        time: center_time, cx: avg_x, cy: avg_y,
                        zoom, hold,
                        priority: dur, // longer dwells = higher priority
                    });
                }
                run_start = None;
            }
        }
    }
    // Final run check
    if let Some(start) = run_start {
        let end = norm.len() - 1;
        let dur = norm[end].time - norm[start].time;
        if dur >= min_dwell_secs && dur <= max_dwell_secs {
            let count = (end - start + 1) as f64;
            let avg_x: f64 = norm[start..=end].iter().map(|s| s.nx).sum::<f64>() / count;
            let avg_y: f64 = norm[start..=end].iter().map(|s| s.ny).sum::<f64>() / count;
            let center_time = (norm[start].time + norm[end].time) / 2.0;
            candidates.push(ZoomCandidate {
                time: center_time, cx: avg_x, cy: avg_y,
                zoom: 1.8, hold: (dur * 0.6).clamp(0.4, 2.0),
                priority: dur,
            });
        }
    }

    // ── Sort by time and merge overlapping candidates ──
    candidates.sort_by(|a, b| a.time.partial_cmp(&b.time).unwrap_or(std::cmp::Ordering::Equal));

    let mut merged: Vec<ZoomCandidate> = Vec::new();
    for c in candidates {
        if let Some(last) = merged.last_mut() {
            // Merge if within 1s — keep higher priority, blend position
            if (c.time - last.time).abs() < 1.0 {
                if c.priority > last.priority {
                    let blend = 0.6; // bias toward higher-priority
                    last.cx = last.cx * (1.0 - blend) + c.cx * blend;
                    last.cy = last.cy * (1.0 - blend) + c.cy * blend;
                    last.time = (last.time + c.time) / 2.0;
                    last.zoom = last.zoom.max(c.zoom);
                    last.hold = last.hold.max(c.hold);
                    last.priority = c.priority;
                }
                continue;
            }
        }
        merged.push(c);
    }

    // ── Convert to ZoomKeyframe (already normalized 0-1) ──
    // Output normalized coordinates directly — no screen coord conversion.
    // The internal `nx/ny` are already 0-1 relative to the mouse activity area,
    // which closely matches the video viewport.
    let mut keyframes: Vec<super::editor::ZoomKeyframe> = merged.iter().map(|c| {
        super::editor::ZoomKeyframe {
            time: c.time,
            zoom: c.zoom,
            center_x: c.cx.clamp(0.0, 1.0),
            center_y: c.cy.clamp(0.0, 1.0),
            easing: "spring".to_string(),
            hold: c.hold,
        }
    }).collect();

    // Cap at 8 keyframes — too many zooms feels chaotic, not polished.
    // Good products (Screen Studio, FocuSee) use ~3-6 zooms per minute.
    // Prioritize by zoom level (strongest zooms are most intentional).
    if keyframes.len() > 8 {
        keyframes.sort_by(|a, b| b.zoom.partial_cmp(&a.zoom).unwrap_or(std::cmp::Ordering::Equal));
        keyframes.truncate(8);
        keyframes.sort_by(|a, b| a.time.partial_cmp(&b.time).unwrap_or(std::cmp::Ordering::Equal));
    }

    // Ensure minimum 2s rest gap between keyframes so zoom "breathes".
    // Without rest, zoom groups merge into one endless zoom.
    let min_gap = 2.0; // seconds between zoom keyframes
    let mut filtered: Vec<super::editor::ZoomKeyframe> = Vec::new();
    for kf in &keyframes {
        if let Some(last) = filtered.last() {
            let last_end = last.time + last.hold + 0.6; // hold + exit transition
            if kf.time - last_end < min_gap {
                continue; // skip — too close, would merge into one long zoom
            }
        }
        filtered.push(kf.clone());
    }
    let keyframes = filtered;

    println!("[mouse-tracker] Suggested {} zoom keyframes ({} clicks, {} dwells)",
        keyframes.len(),
        keyframes.iter().filter(|k| k.hold < 0.4).count(),
        keyframes.iter().filter(|k| k.hold >= 0.4).count());
    keyframes
}

// ── Platform-specific mouse position retrieval ──────────────────────

/// Get the current mouse position and click state via CoreGraphics.
/// Get the main display's logical size (points, not pixels).
/// Used to normalize mouse coordinates (CGEvent returns logical coords).
fn get_main_display_logical_size() -> (f64, f64) {
    use objc2_core_graphics::CGMainDisplayID;
    let display_id = CGMainDisplayID();
    let w = objc2_core_graphics::CGDisplayPixelsWide(display_id) as f64;
    let h = objc2_core_graphics::CGDisplayPixelsHigh(display_id) as f64;
    if w > 0.0 && h > 0.0 {
        (w, h)
    } else {
        (1920.0, 1080.0) // fallback
    }
}

fn get_mouse_position() -> (f64, f64, bool) {
    use objc2_core_graphics::{CGEvent, CGEventSource, CGEventSourceStateID, CGMouseButton};

    // CGEvent::new with None source gives us the current mouse location
    let event = CGEvent::new(None);
    if let Some(ref event) = event {
        let point = CGEvent::location(Some(event));

        // Check for left mouse button state
        let button_state = CGEventSource::button_state(
            CGEventSourceStateID::CombinedSessionState,
            CGMouseButton::Left,
        );

        (point.x, point.y, button_state)
    } else {
        (0.0, 0.0, false)
    }
}
