//! Real-time zoom effect — Screen Studio style auto-zoom.
//!
//! Uses ScreenCaptureKit's dynamic `updateConfiguration` to change the
//! capture `sourceRect` in real-time, following the mouse cursor with
//! smooth spring physics. This produces a zoomed output video directly
//! during recording — no post-processing needed.
//!
//! **Zoom behaviour (Screen Studio style)**:
//! - Mouse idle → zoom level = 1x (full region, no zoom)
//! - Mouse moves → smoothly zoom in to `max_zoom` and follow cursor
//! - Mouse stops → after `idle_delay`, smoothly zoom back out to 1x
//!
//! Architecture:
//!   Background thread (30 Hz) → read cursor position → detect movement
//!     → spring physics for zoom level + pan → compute sourceRect
//!     → SCStream.updateConfiguration(sourceRect)

use objc2::rc::Retained;
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_screen_capture_kit::{SCStream, SCStreamConfiguration};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::cursor::cg;

/// Wrapper to send Retained<SCStream> across threads.
/// SAFETY: SCStream is an ObjC object that is internally thread-safe.
/// Apple's ScreenCaptureKit documentation confirms that updateConfiguration
/// can be called from any thread.
struct SendableStream(Retained<SCStream>);
unsafe impl Send for SendableStream {}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Configuration
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Zoom effect configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ZoomConfig {
    /// Maximum zoom level (e.g., 2.0 = 200%).
    pub max_zoom: f64,

    // ── Spring parameters for PAN animation ──
    /// Natural frequency for pan spring. Higher = tighter cursor following.
    /// Lower values (3-4) give a silky delayed follow; higher (6-10) = snappy.
    pub pan_omega: f64,
    /// Damping ratio for pan spring.
    ///   ζ < 1.0 → slight overshoot (bouncy pan)
    ///   ζ = 1.0 → critically damped (smooth, no overshoot)
    ///   ζ > 1.0 → over-damped (sluggish)
    pub pan_damping: f64,

    // ── Spring parameters for ZOOM animation ──
    /// Natural frequency for zoom spring. Controls how fast zoom in/out feels.
    pub zoom_omega: f64,
    /// Damping ratio for zoom spring.
    pub zoom_damping: f64,

    // ── Idle detection ──
    /// How long (seconds) mouse must be still before zooming back to 1x.
    pub idle_delay: f64,
    /// How long (seconds) mouse must keep moving before zoom-in starts.
    /// Prevents accidental zoom from small/brief mouse movements.
    pub move_delay: f64,
    /// Mouse movement threshold (logical pixels/tick) to consider "moving".
    pub move_threshold: f64,

    /// How often to update the viewport (Hz). 60 = butter-smooth on M-series.
    pub update_rate: u32,
}

impl Default for ZoomConfig {
    fn default() -> Self {
        Self {
            max_zoom: 2.0,
            // Pan spring: slightly under-damped for silky feel with subtle bounce
            pan_omega: 4.0,
            pan_damping: 0.85,
            // Zoom spring: slow and smooth for a gradual zoom feel
            zoom_omega: 2.5,
            zoom_damping: 1.0,  // critically damped — no bounce on zoom
            // Idle detection
            idle_delay: 1.0,    // 1s idle before zoom out
            move_delay: 0.4,    // mouse must move for 0.4s before zoom starts
            move_threshold: 3.0, // 3 logical pixels movement = "moving"
            // 60 Hz — butter-smooth zoom on Apple Silicon.
            // M-series media engine handles updateConfiguration at 60 Hz trivially.
            // At High quality (60fps recording), this gives 1:1 zoom/frame sync.
            update_rate: 60,
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Damped Spring Physics
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// A 1D damped harmonic oscillator (spring).
///
/// Models the equation:  x'' + 2ζω₀ x' + ω₀² x = ω₀² target
///
/// When ζ ≈ 1 (critically damped), the spring settles to the target as fast
/// as possible without overshooting — perfect for smooth camera panning.
/// When ζ < 1, there's a subtle bounce that gives a more organic "Screen
/// Studio" feel.
#[derive(Clone, Debug)]
struct Spring {
    /// Current value.
    pos: f64,
    /// Current velocity.
    vel: f64,
    /// Natural frequency (rad/s).
    omega: f64,
    /// Damping ratio (0-2 typical).
    damping: f64,
}

impl Spring {
    fn new(initial: f64, omega: f64, damping: f64) -> Self {
        Self {
            pos: initial,
            vel: 0.0,
            omega,
            damping,
        }
    }

    /// Advance the spring towards `target` by `dt` seconds.
    /// Uses semi-implicit Euler integration (stable for stiff springs).
    fn step(&mut self, target: f64, dt: f64) {
        // Force = ω₀²(target - pos) - 2ζω₀ vel
        let accel = self.omega * self.omega * (target - self.pos)
            - 2.0 * self.damping * self.omega * self.vel;

        // Semi-implicit Euler: update velocity first, then position
        self.vel += accel * dt;
        self.pos += self.vel * dt;
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ZoomController — real-time zoom via SCStream updateConfiguration
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Handle to a running real-time zoom controller.
/// The controller runs a background thread that updates the SCStream's
/// sourceRect to follow the cursor with spring-animated panning.
///
/// Drop or call `stop()` to end the zoom effect and restore full capture.
pub struct ZoomController {
    running: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl ZoomController {
    /// Start the real-time zoom controller.
    ///
    /// - `stream`: the active SCStream to update
    /// - `region_origin`: top-left of the recording region in logical screen coords (0,0 for fullscreen)
    /// - `region_size`: size of the recording region in logical screen coords
    /// - `output_width`/`output_height`: video output dimensions in physical pixels
    /// - `config`: zoom configuration
    pub fn start(
        stream: Retained<SCStream>,
        region_origin: (f64, f64),
        region_size: (f64, f64),
        output_width: usize,
        output_height: usize,
        config: ZoomConfig,
        capture_system_audio: bool,
        capture_microphone: bool,
    ) -> Self {
        let running = Arc::new(AtomicBool::new(true));
        let running_clone = running.clone();
        let sendable = SendableStream(stream);

        let handle = std::thread::Builder::new()
            .name("zoom-controller".into())
            .spawn(move || {
                zoom_thread(
                    sendable,
                    region_origin,
                    region_size,
                    output_width,
                    output_height,
                    config,
                    running_clone,
                    capture_system_audio,
                    capture_microphone,
                );
            })
            .expect("Failed to spawn zoom controller thread");

        println!("[zureshot-zoom] Real-time zoom controller started");

        Self {
            running,
            handle: Some(handle),
        }
    }

    /// Stop the zoom controller and wait for the thread to finish.
    pub fn stop(mut self) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
        println!("[zureshot-zoom] Zoom controller stopped");
    }
}

impl Drop for ZoomController {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        // Don't join in Drop — the thread will notice running=false and exit
    }
}

/// Background thread: reads cursor position, detects movement/idle,
/// animates zoom level + pan position, updates SCStream configuration.
fn zoom_thread(
    stream: SendableStream,
    region_origin: (f64, f64),
    region_size: (f64, f64),
    output_width: usize,
    output_height: usize,
    config: ZoomConfig,
    running: Arc<AtomicBool>,
    capture_system_audio: bool,
    capture_microphone: bool,
) {
    let stream = &stream.0;

    let dt = 1.0 / config.update_rate as f64;
    let sleep_dur = std::time::Duration::from_secs_f64(dt);

    // Initialize springs at current mouse position (or center of region)
    let initial_mouse = cg::get_mouse_position();
    let init_x = if initial_mouse.x >= region_origin.0
        && initial_mouse.x <= region_origin.0 + region_size.0
    {
        initial_mouse.x
    } else {
        region_origin.0 + region_size.0 / 2.0
    };
    let init_y = if initial_mouse.y >= region_origin.1
        && initial_mouse.y <= region_origin.1 + region_size.1
    {
        initial_mouse.y
    } else {
        region_origin.1 + region_size.1 / 2.0
    };

    let mut pan_x = Spring::new(init_x, config.pan_omega, config.pan_damping);
    let mut pan_y = Spring::new(init_y, config.pan_omega, config.pan_damping);

    // Zoom spring: starts at 1.0 (no zoom), targets max_zoom when moving
    let mut zoom_spring = Spring::new(1.0, config.zoom_omega, config.zoom_damping);

    // Idle detection state
    let mut prev_mouse_x = init_x;
    let mut prev_mouse_y = init_y;
    let mut idle_time: f64 = 999.0; // start idle (no zoom)
    let mut move_time: f64 = 0.0;   // how long mouse has been continuously moving

    println!(
        "[zureshot-zoom] Region: ({:.0},{:.0}) {:.0}x{:.0}, output: {}x{}, max_zoom: {:.1}x",
        region_origin.0, region_origin.1, region_size.0, region_size.1,
        output_width, output_height, config.max_zoom
    );

    let mut update_count: u64 = 0;

    while running.load(Ordering::Relaxed) {
        // 1. Read current cursor position (global screen logical coords)
        let mouse = cg::get_mouse_position();

        // 2. Detect mouse movement
        let dx = mouse.x - prev_mouse_x;
        let dy = mouse.y - prev_mouse_y;
        let moved_dist = (dx * dx + dy * dy).sqrt();
        prev_mouse_x = mouse.x;
        prev_mouse_y = mouse.y;

        // Two thresholds:
        //   - move_threshold (3px): large movement needed to TRIGGER zoom-in
        //   - keep_threshold (0.5px): any tiny movement keeps zoom ALIVE
        // Once zoomed in, even micro-movements (hovering, small adjustments)
        // prevent zoom-out. Only true stillness triggers zoom-out.
        let already_zoomed = zoom_spring.pos > 1.05;
        let active_threshold = if already_zoomed { 0.5 } else { config.move_threshold };

        if moved_dist > active_threshold {
            // Mouse is active — accumulate move time, reset idle
            if moved_dist > config.move_threshold {
                move_time += dt;  // only big moves count toward triggering zoom
            }
            idle_time = 0.0;
        } else {
            idle_time += dt;
            // Reset move_time when mouse truly stops
            if idle_time > 0.15 {
                move_time = 0.0;
            }
        }

        // 3. Determine target zoom level
        //    - Need sustained movement (move_time > move_delay) to start zooming in
        //    - After mouse stops for idle_delay, zoom back out
        let target_zoom = if move_time >= config.move_delay && idle_time < config.idle_delay {
            config.max_zoom  // sustained movement → zoom in
        } else {
            1.0  // brief movement or idle → stay/return to 1x
        };

        // 4. Step spring physics
        let sub_dt = dt / 4.0;
        for _ in 0..4 {
            zoom_spring.step(target_zoom, sub_dt);
        }

        // Current zoom level (clamped between 1.0 and max_zoom)
        let current_zoom = zoom_spring.pos.clamp(1.0, config.max_zoom);

        // 5. Compute viewport based on current zoom
        // When zoom ~ 1.0, viewport = full region (no pan needed)
        // When zoom > 1.0, viewport is smaller and follows cursor
        let viewport_w = region_size.0 / current_zoom;
        let viewport_h = region_size.1 / current_zoom;

        // Only pan-follow cursor when zoomed in
        // When zooming IN: use dead zone so small movements don't cause jitter.
        // When zooming OUT: blend pan back to region center.
        let zooming_in = target_zoom > 1.0;
        let effective_zoom_ratio = if zooming_in {
            1.0
        } else {
            ((current_zoom - 1.0) / (config.max_zoom - 1.0)).clamp(0.0, 1.0)
        };

        // Clamp mouse target to within the recording region (for pan)
        let target_x = mouse.x.clamp(
            region_origin.0 + viewport_w / 2.0,
            region_origin.0 + region_size.0 - viewport_w / 2.0,
        );
        let target_y = mouse.y.clamp(
            region_origin.1 + viewport_h / 2.0,
            region_origin.1 + region_size.1 - viewport_h / 2.0,
        );

        // ── Gradual pan with cursor-always-visible guarantee ──
        // Panning strength is a smooth curve based on cursor distance from
        // viewport center, BUT the cursor must ALWAYS remain visible with
        // a 10% margin from the viewport edge.
        //
        // Two-pass approach:
        //   Pass 1: cubic ease-in for smooth follow (no jitter for small moves)
        //   Pass 2: hard clamp so cursor is never outside the viewport
        let pan_target_x;
        let pan_target_y;

        if current_zoom > 1.01 && effective_zoom_ratio > 0.5 {
            let half_vw = viewport_w / 2.0;
            let half_vh = viewport_h / 2.0;

            // Current viewport center (from spring position)
            let cur_cx = pan_x.pos;
            let cur_cy = pan_y.pos;

            // Cursor offset from viewport center, normalized to 0..1
            let dx = target_x - cur_cx;
            let dy = target_y - cur_cy;
            let nx = if half_vw > 0.1 { (dx.abs() / half_vw).clamp(0.0, 1.0) } else { 0.0 };
            let ny = if half_vh > 0.1 { (dy.abs() / half_vh).clamp(0.0, 1.0) } else { 0.0 };

            // Pass 1: cubic ease-in for smooth follow
            let strength_x = nx * nx * nx;
            let strength_y = ny * ny * ny;
            let soft_x = cur_cx + dx * strength_x;
            let soft_y = cur_cy + dy * strength_y;

            // Pass 2: hard guarantee — cursor must be within 90% of viewport
            // (10% margin from each edge, so cursor never hugs the very edge)
            let margin = 0.10;
            let safe_half_w = half_vw * (1.0 - margin);
            let safe_half_h = half_vh * (1.0 - margin);

            // If cursor would be outside the safe area, force pan to keep it inside
            pan_target_x = soft_x.clamp(target_x - safe_half_w, target_x + safe_half_w);
            pan_target_y = soft_y.clamp(target_y - safe_half_h, target_y + safe_half_h);
        } else {
            // Not zoomed in or zooming out → blend to center
            let center_x = region_origin.0 + region_size.0 / 2.0;
            let center_y = region_origin.1 + region_size.1 / 2.0;
            pan_target_x = center_x + (target_x - center_x) * effective_zoom_ratio;
            pan_target_y = center_y + (target_y - center_y) * effective_zoom_ratio;
        };

        for _ in 0..4 {
            pan_x.step(pan_target_x, sub_dt);
            pan_y.step(pan_target_y, sub_dt);
        }

        // 6. Compute sourceRect — clamp so viewport stays within the recording region
        let half_w = viewport_w / 2.0;
        let half_h = viewport_h / 2.0;

        let min_x = region_origin.0;
        let min_y = region_origin.1;
        let max_x = region_origin.0 + region_size.0;
        let max_y = region_origin.1 + region_size.1;

        let cx = pan_x.pos.clamp(min_x + half_w, max_x - half_w);
        let cy = pan_y.pos.clamp(min_y + half_h, max_y - half_h);

        let source_x = cx - half_w;
        let source_y = cy - half_h;

        // 7. Skip update if nothing materially changed (saves ObjC allocations)
        //    When zoom is ~1.0 and settled, no need to keep pushing configs.
        let zoom_near_one = (current_zoom - 1.0).abs() < 0.005;
        let spring_settled = zoom_spring.vel.abs() < 0.01
            && pan_x.vel.abs() < 0.1
            && pan_y.vel.abs() < 0.1;
        if zoom_near_one && spring_settled && update_count > 1 {
            // No visible change — skip this update
            update_count += 1;
            std::thread::sleep(sleep_dur);
            continue;
        }

        // 8. Update SCStream configuration with new sourceRect
        //    IMPORTANT: SCStreamConfiguration::new() defaults capturesAudio=false.
        //    updateConfiguration applies ALL properties — must re-specify audio.
        let new_config = unsafe {
            let c = SCStreamConfiguration::new();
            c.setWidth(output_width);
            c.setHeight(output_height);
            c.setSourceRect(CGRect::new(
                CGPoint::new(source_x, source_y),
                CGSize::new(viewport_w, viewport_h),
            ));
            c.setDestinationRect(CGRect::new(
                CGPoint::new(0.0, 0.0),
                CGSize::new(output_width as f64, output_height as f64),
            ));
            c.setScalesToFit(true);
            // Re-specify audio settings (defaults are all false/0)
            if capture_system_audio || capture_microphone {
                c.setSampleRate(48000);
                c.setChannelCount(2);
            }
            if capture_system_audio {
                c.setCapturesAudio(true);
                c.setExcludesCurrentProcessAudio(true);
            }
            if capture_microphone {
                c.setCaptureMicrophone(true);
            }
            c
        };

        unsafe {
            stream.updateConfiguration_completionHandler(&new_config, None);
        }

        // Log periodically
        update_count += 1;
        if update_count == 1 || update_count % 300 == 0 {
            println!(
                "[zureshot-zoom] #{}: zoom={:.2}x (target={:.1}), viewport={:.0}x{:.0}, idle={:.1}s",
                update_count, current_zoom, target_zoom, viewport_w, viewport_h, idle_time
            );
        }

        std::thread::sleep(sleep_dur);
    }

    // Restore full region capture before stopping
    println!("[zureshot-zoom] Restoring full capture region...");
    let restore_config = unsafe {
        let c = SCStreamConfiguration::new();
        c.setWidth(output_width);
        c.setHeight(output_height);
        c.setSourceRect(CGRect::new(
            CGPoint::new(region_origin.0, region_origin.1),
            CGSize::new(region_size.0, region_size.1),
        ));
        c.setDestinationRect(CGRect::new(
            CGPoint::new(0.0, 0.0),
            CGSize::new(output_width as f64, output_height as f64),
        ));
        c.setScalesToFit(true);
        // Re-specify audio settings
        if capture_system_audio || capture_microphone {
            c.setSampleRate(48000);
            c.setChannelCount(2);
        }
        if capture_system_audio {
            c.setCapturesAudio(true);
            c.setExcludesCurrentProcessAudio(true);
        }
        if capture_microphone {
            c.setCaptureMicrophone(true);
        }
        c
    };
    unsafe {
        stream.updateConfiguration_completionHandler(&restore_config, None);
    }
    // Give SCK a moment to process the restore
    std::thread::sleep(std::time::Duration::from_millis(100));

    println!("[zureshot-zoom] Thread exiting (after {} updates)", update_count);
}
