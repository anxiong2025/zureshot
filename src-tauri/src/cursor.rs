//! Cursor position and click tracking for zoom effect.
//!
//! Uses CGEventTap to passively observe mouse events during recording.
//! Events are timestamped relative to recording start and stored in memory.
//! After recording stops, events can be serialized to JSON for post-processing.
//!
//! Architecture:
//!   CGEventTap (passive listener) → callback → Vec<CursorEvent>
//!   Polling thread (10Hz) → periodic position snapshots
//!
//! The combination gives us:
//!   - Precise click timing (from CGEventTap)
//!   - Smooth position data (from polling)
//!   - Low overhead (~0.1% CPU)

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde::{Deserialize, Serialize};

/// A single cursor event (position snapshot or click).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CursorEvent {
    /// Seconds since recording started.
    pub time: f64,
    /// X position in logical (CSS) pixels, top-left origin.
    pub x: f64,
    /// Y position in logical (CSS) pixels, top-left origin.
    pub y: f64,
    /// Event type.
    pub kind: CursorEventKind,
}

/// What type of cursor event this is.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum CursorEventKind {
    /// Periodic position snapshot (from polling).
    Move,
    /// Left mouse button pressed.
    LeftDown,
    /// Left mouse button released.
    LeftUp,
    /// Right mouse button pressed.
    RightDown,
    /// Scroll wheel.
    Scroll,
}

/// Shared state for the cursor tracker.
struct TrackerState {
    events: Vec<CursorEvent>,
    start_time: Instant,
}

/// Handle to a running cursor tracker. Drop or call `stop()` to end tracking.
pub struct CursorTracker {
    running: Arc<AtomicBool>,
    state: Arc<Mutex<TrackerState>>,
    /// Join handle for the polling thread.
    poll_handle: Option<std::thread::JoinHandle<()>>,
    /// Join handle for the event tap thread.
    tap_handle: Option<std::thread::JoinHandle<()>>,
}

// CoreGraphics FFI for CGEventTap and mouse position
#[allow(non_camel_case_types, dead_code)]
pub mod cg {
    pub type CGEventTapLocation = u32;
    pub type CGEventTapPlacement = u32;
    pub type CGEventTapOptions = u32;
    pub type CGEventMask = u64;
    pub type CGEventType = u32;
    pub type CGEventRef = *mut std::ffi::c_void;
    pub type CFMachPortRef = *mut std::ffi::c_void;
    pub type CFRunLoopSourceRef = *mut std::ffi::c_void;
    pub type CFRunLoopRef = *mut std::ffi::c_void;

    // CGEventTap callback
    pub type CGEventTapCallBack = extern "C" fn(
        proxy: *mut std::ffi::c_void,
        event_type: CGEventType,
        event: CGEventRef,
        user_info: *mut std::ffi::c_void,
    ) -> CGEventRef;

    // Event tap locations
    pub const K_CG_HID_EVENT_TAP: CGEventTapLocation = 0;

    // Event tap options
    pub const K_CG_EVENT_TAP_OPTION_LISTEN_ONLY: CGEventTapOptions = 1;

    // Event types
    pub const K_CG_EVENT_LEFT_MOUSE_DOWN: CGEventType = 1;
    pub const K_CG_EVENT_LEFT_MOUSE_UP: CGEventType = 2;
    pub const K_CG_EVENT_RIGHT_MOUSE_DOWN: CGEventType = 3;
    pub const K_CG_EVENT_MOUSE_MOVED: CGEventType = 5;
    pub const K_CG_EVENT_LEFT_MOUSE_DRAGGED: CGEventType = 6;
    pub const K_CG_EVENT_SCROLL_WHEEL: CGEventType = 22;
    pub const K_CG_EVENT_TAP_DISABLED_BY_TIMEOUT: CGEventType = 0xFFFFFFFE;

    // CGPoint for mouse location
    #[repr(C)]
    #[derive(Clone, Copy, Debug)]
    pub struct CGPoint {
        pub x: f64,
        pub y: f64,
    }

    extern "C" {
        pub fn CGEventTapCreate(
            tap: CGEventTapLocation,
            place: CGEventTapPlacement,
            options: CGEventTapOptions,
            events_of_interest: CGEventMask,
            callback: CGEventTapCallBack,
            user_info: *mut std::ffi::c_void,
        ) -> CFMachPortRef;

        pub fn CFMachPortCreateRunLoopSource(
            allocator: *const std::ffi::c_void,
            port: CFMachPortRef,
            order: i64,
        ) -> CFRunLoopSourceRef;

        pub fn CFRunLoopGetCurrent() -> CFRunLoopRef;

        pub fn CFRunLoopAddSource(
            rl: CFRunLoopRef,
            source: CFRunLoopSourceRef,
            mode: *const std::ffi::c_void,
        );

        pub fn CFRunLoopRunInMode(
            mode: *const std::ffi::c_void,
            seconds: f64,
            return_after_source_handled: bool,
        ) -> i32;

        pub fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);

        pub fn CFRelease(cf: *const std::ffi::c_void);

        /// Get current mouse cursor position (screen coordinates).
        pub fn CGEventCreate(source: *const std::ffi::c_void) -> CGEventRef;
        pub fn CGEventGetLocation(event: CGEventRef) -> CGPoint;
    }

    // CFRunLoopMode
    extern "C" {
        pub static kCFRunLoopDefaultMode: *const std::ffi::c_void;
    }

    /// Build event mask from a list of event types.
    pub fn event_mask(types: &[CGEventType]) -> CGEventMask {
        let mut mask: CGEventMask = 0;
        for t in types {
            mask |= 1 << (*t as u64);
        }
        mask
    }

    /// Get current mouse position (logical screen coordinates, top-left origin).
    pub fn get_mouse_position() -> CGPoint {
        unsafe {
            let event = CGEventCreate(std::ptr::null());
            if event.is_null() {
                return CGPoint { x: 0.0, y: 0.0 };
            }
            let pos = CGEventGetLocation(event);
            CFRelease(event as *const _);
            pos
        }
    }
}

/// State passed to the CGEventTap callback via user_info pointer.
struct TapContext {
    state: Arc<Mutex<TrackerState>>,
    running: Arc<AtomicBool>,
}

/// CGEventTap callback — called for every mouse event.
extern "C" fn event_tap_callback(
    _proxy: *mut std::ffi::c_void,
    event_type: cg::CGEventType,
    event: cg::CGEventRef,
    user_info: *mut std::ffi::c_void,
) -> cg::CGEventRef {
    // Re-enable tap if macOS disabled it due to timeout
    if event_type == cg::K_CG_EVENT_TAP_DISABLED_BY_TIMEOUT {
        // We'd need the mach port to re-enable, but since we're listen-only
        // and this rarely happens, just log it.
        eprintln!("[zureshot-cursor] Event tap disabled by timeout");
        return event;
    }

    let ctx = unsafe { &*(user_info as *const TapContext) };
    if !ctx.running.load(Ordering::Relaxed) {
        return event;
    }

    let pos = unsafe { cg::CGEventGetLocation(event) };
    let kind = match event_type {
        cg::K_CG_EVENT_LEFT_MOUSE_DOWN => CursorEventKind::LeftDown,
        cg::K_CG_EVENT_LEFT_MOUSE_UP => CursorEventKind::LeftUp,
        cg::K_CG_EVENT_RIGHT_MOUSE_DOWN => CursorEventKind::RightDown,
        cg::K_CG_EVENT_SCROLL_WHEEL => CursorEventKind::Scroll,
        _ => return event, // Ignore move events from tap (we use polling instead)
    };

    let elapsed = {
        let state = ctx.state.lock().unwrap();
        state.start_time.elapsed().as_secs_f64()
    };

    let cursor_event = CursorEvent {
        time: elapsed,
        x: pos.x,
        y: pos.y,
        kind,
    };

    if let Ok(mut state) = ctx.state.lock() {
        state.events.push(cursor_event);
    }

    event // Pass-through (listen-only)
}

impl CursorTracker {
    /// Start tracking cursor position and clicks.
    /// Returns a handle that can be used to stop tracking and retrieve events.
    pub fn start() -> Self {
        let running = Arc::new(AtomicBool::new(true));
        let state = Arc::new(Mutex::new(TrackerState {
            events: Vec::with_capacity(1024),
            start_time: Instant::now(),
        }));

        // ── Polling thread: capture position at ~20Hz ──
        let poll_running = running.clone();
        let poll_state = state.clone();
        let poll_handle = std::thread::Builder::new()
            .name("zureshot-cursor-poll".into())
            .spawn(move || {
                println!("[zureshot-cursor] Polling thread started (20Hz)");
                while poll_running.load(Ordering::Relaxed) {
                    let pos = cg::get_mouse_position();
                    let elapsed = {
                        let s = poll_state.lock().unwrap();
                        s.start_time.elapsed().as_secs_f64()
                    };

                    if let Ok(mut s) = poll_state.lock() {
                        s.events.push(CursorEvent {
                            time: elapsed,
                            x: pos.x,
                            y: pos.y,
                            kind: CursorEventKind::Move,
                        });
                    }

                    std::thread::sleep(std::time::Duration::from_millis(50)); // 20Hz
                }
                println!("[zureshot-cursor] Polling thread stopped");
            })
            .expect("Failed to spawn cursor polling thread");

        // ── Event tap thread: capture clicks ──
        let tap_running = running.clone();
        let tap_state = state.clone();
        let tap_handle = std::thread::Builder::new()
            .name("zureshot-cursor-tap".into())
            .spawn(move || {
                println!("[zureshot-cursor] Event tap thread started");

                let ctx = Box::new(TapContext {
                    state: tap_state,
                    running: tap_running.clone(),
                });
                let ctx_ptr = Box::into_raw(ctx);

                let mask = cg::event_mask(&[
                    cg::K_CG_EVENT_LEFT_MOUSE_DOWN,
                    cg::K_CG_EVENT_LEFT_MOUSE_UP,
                    cg::K_CG_EVENT_RIGHT_MOUSE_DOWN,
                    cg::K_CG_EVENT_SCROLL_WHEEL,
                ]);

                let tap = unsafe {
                    cg::CGEventTapCreate(
                        cg::K_CG_HID_EVENT_TAP,
                        0,  // kCGHeadInsertEventTap
                        cg::K_CG_EVENT_TAP_OPTION_LISTEN_ONLY,
                        mask,
                        event_tap_callback,
                        ctx_ptr as *mut std::ffi::c_void,
                    )
                };

                if tap.is_null() {
                    eprintln!("[zureshot-cursor] Failed to create event tap (need Accessibility permission)");
                    // Clean up context
                    unsafe { drop(Box::from_raw(ctx_ptr)); }
                    return;
                }

                let source = unsafe {
                    cg::CFMachPortCreateRunLoopSource(std::ptr::null(), tap, 0)
                };
                if source.is_null() {
                    eprintln!("[zureshot-cursor] Failed to create run loop source");
                    unsafe {
                        cg::CFRelease(tap as *const _);
                        drop(Box::from_raw(ctx_ptr));
                    }
                    return;
                }

                unsafe {
                    let rl = cg::CFRunLoopGetCurrent();
                    cg::CFRunLoopAddSource(rl, source, cg::kCFRunLoopDefaultMode);
                    cg::CGEventTapEnable(tap, true);
                }

                // Run the event loop until stopped
                while tap_running.load(Ordering::Relaxed) {
                    unsafe {
                        // Run for 0.2s, then check if we should stop
                        cg::CFRunLoopRunInMode(cg::kCFRunLoopDefaultMode, 0.2, false);
                    }
                }

                // Cleanup
                unsafe {
                    cg::CGEventTapEnable(tap, false);
                    cg::CFRelease(source as *const _);
                    cg::CFRelease(tap as *const _);
                    drop(Box::from_raw(ctx_ptr));
                }

                println!("[zureshot-cursor] Event tap thread stopped");
            })
            .expect("Failed to spawn event tap thread");

        println!("[zureshot-cursor] Cursor tracker started");

        CursorTracker {
            running,
            state,
            poll_handle: Some(poll_handle),
            tap_handle: Some(tap_handle),
        }
    }

    /// Stop tracking and return all collected events, sorted by time.
    pub fn stop(mut self) -> Vec<CursorEvent> {
        self.running.store(false, Ordering::SeqCst);

        if let Some(h) = self.poll_handle.take() {
            let _ = h.join();
        }
        if let Some(h) = self.tap_handle.take() {
            let _ = h.join();
        }

        let mut events = {
            let mut state = self.state.lock().unwrap();
            std::mem::take(&mut state.events)
        };

        // Sort by time (polling and tap events may interleave)
        events.sort_by(|a, b| a.time.partial_cmp(&b.time).unwrap_or(std::cmp::Ordering::Equal));

        let clicks = events.iter().filter(|e| e.kind == CursorEventKind::LeftDown).count();
        let moves = events.iter().filter(|e| e.kind == CursorEventKind::Move).count();
        println!(
            "[zureshot-cursor] Tracker stopped: {} events ({} moves, {} clicks)",
            events.len(), moves, clicks
        );

        events
    }
}

impl Drop for CursorTracker {
    fn drop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        // Don't join here — we may be dropped on the main thread and the tap thread
        // needs the run loop to exit. The threads will exit on their own.
    }
}

/// Save cursor events to a JSON file alongside the recording.
pub fn save_cursor_events(events: &[CursorEvent], mp4_path: &str) -> Result<String, String> {
    let json_path = mp4_path.replace(".mp4", ".cursor.json");
    let json = serde_json::to_string_pretty(events)
        .map_err(|e| format!("Failed to serialize cursor events: {}", e))?;
    std::fs::write(&json_path, json)
        .map_err(|e| format!("Failed to write cursor events: {}", e))?;
    println!("[zureshot-cursor] Saved {} events to {}", events.len(), json_path);
    Ok(json_path)
}

/// Load cursor events from a JSON sidecar file.
pub fn load_cursor_events(json_path: &str) -> Result<Vec<CursorEvent>, String> {
    let json = std::fs::read_to_string(json_path)
        .map_err(|e| format!("Failed to read cursor events: {}", e))?;
    let events: Vec<CursorEvent> = serde_json::from_str(&json)
        .map_err(|e| format!("Failed to parse cursor events: {}", e))?;
    Ok(events)
}
