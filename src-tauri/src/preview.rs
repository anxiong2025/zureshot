//! Native preview window — shows real-time zoomed capture preview.
//!
//! Architecture:
//!   SCStream → CMSampleBuffer → IOSurface → CALayer.contents
//!
//! The preview window is a borderless, floating NSWindow with a CALayer
//! that directly displays the IOSurface from each captured frame.
//! Zero-copy: IOSurface is GPU memory shared between SCK and Core Animation.
//!
//! The window covers the full recording region so the user sees exactly\n//! what the zoomed camera captures during recording.
//!
//! **Thread safety**: All NSWindow/CALayer operations are dispatched to the
//! main thread via `dispatch_async(dispatch_get_main_queue(), ...)`.

use objc2::msg_send;
use objc2::runtime::NSObject;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
use std::sync::Arc;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// CoreFoundation FFI
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

extern "C" {
    fn CFRetain(cf: *const c_void) -> *const c_void;
    fn CFRelease(cf: *const c_void);
    fn CGMainDisplayID() -> u32;
    fn CGDisplayHideCursor(display: u32) -> i32;
    fn CGDisplayShowCursor(display: u32) -> i32;
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// PreviewWindow
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// A native borderless preview window that shows the current capture frame.
///
/// The preview uses IOSurface → CALayer for zero-copy GPU rendering.
/// All AppKit operations are dispatched to the main thread.
pub struct PreviewWindow {
    /// Shared latest IOSurface pointer (set by capture callback, read by refresh loop)
    surface: Arc<AtomicPtr<c_void>>,
    /// The NSWindow object — stored as raw pointer for thread-safe access.
    /// All operations on it are dispatched to main thread.
    window_ptr: *mut c_void,
    /// The CALayer object — raw pointer, same thread-safety model.
    layer_ptr: *mut c_void,
    /// Previously displayed IOSurface (retained, released on next frame or close)
    prev_surface: *mut c_void,
}

// SAFETY: The raw pointers (window_ptr, layer_ptr) are only accessed via
// dispatch_async to the main thread, or through AtomicPtr. The struct itself
// can be moved between threads safely.
unsafe impl Send for PreviewWindow {}

impl PreviewWindow {
    /// Create and show a preview window that covers the full recording region.
    ///
    /// - `region_x/y`: recording region origin in screen logical coords
    /// - `region_w/h`: recording region size in logical coords
    ///
    /// This can be called from any thread — window creation is dispatched
    /// to the main thread via dispatch_sync.
    pub fn new(
        region_x: f64,
        region_y: f64,
        region_w: f64,
        region_h: f64,
    ) -> Self {
        // Create window on main thread using dispatch_sync
        let (window_ptr, layer_ptr) =
            Self::create_window_on_main_thread(region_x, region_y, region_w, region_h);

        println!(
            "[zureshot-preview] Preview window created: {:.0}x{:.0} at ({:.0},{:.0})",
            region_w, region_h, region_x, region_y,
        );

        Self {
            surface: Arc::new(AtomicPtr::new(std::ptr::null_mut())),
            window_ptr,
            layer_ptr,
            prev_surface: std::ptr::null_mut(),
        }
    }

    /// Create NSWindow + CALayer on the main thread (blocking).
    /// Returns raw pointers to window and layer (both retained).
    fn create_window_on_main_thread(
        x: f64,
        top_y: f64,
        w: f64,
        h: f64,
    ) -> (*mut c_void, *mut c_void) {
        use std::sync::mpsc;
        // Use usize channel since *mut c_void isn't Send
        let (tx, rx) = mpsc::channel::<(usize, usize)>();

        // Dispatch to main thread — blocks until window is created.
        let queue = dispatch2::DispatchQueue::main();
        queue.exec_sync(move || {
            let (window_ptr, layer_ptr) = unsafe { Self::create_window_inner(x, top_y, w, h) };
            let _ = tx.send((window_ptr as usize, layer_ptr as usize));
        });

        let (w_addr, l_addr) = rx.recv().expect("Failed to receive window pointers from main thread");
        (w_addr as *mut c_void, l_addr as *mut c_void)
    }

    /// Actually create the NSWindow and CALayer. MUST run on main thread.
    unsafe fn create_window_inner(
        x: f64,
        top_y: f64,
        w: f64,
        h: f64,
    ) -> (*mut c_void, *mut c_void) {
        // Convert from top-left to bottom-left coordinates (macOS screen coords)
        let screen: *mut NSObject = msg_send![objc2::class!(NSScreen), mainScreen];
        let screen_frame: objc2_core_foundation::CGRect = msg_send![screen, frame];
        let screen_height = screen_frame.size.height;
        let y = screen_height - top_y - h;

        // Content rect
        let frame = objc2_core_foundation::CGRect::new(
            objc2_core_foundation::CGPoint::new(x, y),
            objc2_core_foundation::CGSize::new(w, h),
        );

        // NSWindowStyleMaskBorderless = 0
        let style_mask: usize = 0;

        // Create NSWindow
        let alloc: *mut NSObject = msg_send![objc2::class!(NSWindow), alloc];
        let window: *mut NSObject = msg_send![
            alloc,
            initWithContentRect: frame,
            styleMask: style_mask,
            backing: 2usize,
            defer: objc2::runtime::Bool::NO
        ];

        if window.is_null() {
            panic!("[zureshot-preview] Failed to create NSWindow");
        }

        // Retain the window so it doesn't get deallocated
        CFRetain(window as *const c_void);

        // Configure window
        let _: () = msg_send![window, setLevel: 25isize]; // NSScreenSaverWindowLevel
        let _: () = msg_send![window, setOpaque: objc2::runtime::Bool::NO];
        let _: () = msg_send![window, setHasShadow: objc2::runtime::Bool::YES];
        let _: () = msg_send![window, setIgnoresMouseEvents: objc2::runtime::Bool::YES];
        // NSWindowCollectionBehaviorCanJoinAllSpaces | Stationary
        let behavior: usize = (1 << 0) | (1 << 4);
        let _: () = msg_send![window, setCollectionBehavior: behavior];

        // Transparent background
        let clear_color: *mut NSObject = msg_send![objc2::class!(NSColor), clearColor];
        let _: () = msg_send![window, setBackgroundColor: clear_color];

        // Get content view and make it layer-backed
        let content_view: *mut NSObject = msg_send![window, contentView];
        let _: () = msg_send![content_view, setWantsLayer: objc2::runtime::Bool::YES];

        // Get the backing CALayer
        let layer: *mut NSObject = msg_send![content_view, layer];
        if layer.is_null() {
            panic!("[zureshot-preview] Failed to get CALayer from content view");
        }

        // Retain the layer
        CFRetain(layer as *const c_void);

        // Configure layer for video display — fill the entire window
        let gravity: &objc2_foundation::NSString =
            objc2_foundation::ns_string!("resize");
        let _: () = msg_send![layer, setContentsGravity: gravity];

        // Show the window
        let _: () = msg_send![window, orderFrontRegardless];

        // Hide the system cursor at the display level — the captured frame
        // includes the cursor, so only the capture-rendered cursor is visible.
        // CGDisplayHideCursor works at the Quartz/display level, unlike
        // [NSCursor hide] which is app-level and doesn't work when the
        // preview window has ignoresMouseEvents:YES.
        CGDisplayHideCursor(CGMainDisplayID());

        (window as *mut c_void, layer as *mut c_void)
    }

    /// Get a clone of the shared surface Arc for use in the capture callback.
    pub fn surface_slot(&self) -> Arc<AtomicPtr<c_void>> {
        self.surface.clone()
    }

    /// Push an IOSurface from the capture callback (lock-free, any thread).
    /// Retains the new surface and releases the one it replaces in the slot.
    /// This ensures at most 1 extra IOSurface is retained (the latest one).
    pub fn push_surface(slot: &AtomicPtr<c_void>, new_surface: *mut c_void) {
        if new_surface.is_null() {
            return;
        }
        // Retain the new surface before storing
        unsafe { CFRetain(new_surface as *const c_void); }
        // Atomically swap — get the previously stored surface
        let old = slot.swap(new_surface, Ordering::AcqRel);
        // Release the one we just replaced (if any)
        if !old.is_null() {
            unsafe { CFRelease(old as *const c_void); }
        }
    }

    /// Display the latest frame on the CALayer.
    /// Dispatches setContents to main thread for thread safety.
    pub fn display_latest_frame(&mut self) {
        let surface = self.surface.swap(std::ptr::null_mut(), Ordering::AcqRel);
        if surface.is_null() {
            return;
        }

        // Release the previously displayed surface
        if !self.prev_surface.is_null() {
            unsafe { CFRelease(self.prev_surface as *const c_void); }
        }
        // Hold onto this surface until the next frame (CALayer also retains internally)
        self.prev_surface = surface;

        // Set as CALayer contents — dispatch to main thread
        let layer_addr = self.layer_ptr as usize;
        let surface_addr = surface as usize;
        let queue = dispatch2::DispatchQueue::main();
        queue.exec_async(move || {
            unsafe {
                let layer_ptr = layer_addr as *mut NSObject;
                let surface_ptr = surface_addr as *mut NSObject;
                let _: () = msg_send![layer_ptr, setContents: surface_ptr];
            }
        });
    }

    /// Close and destroy the preview window. Dispatches to main thread.
    pub fn close(mut self) {
        // Release any pending surface in the slot
        let pending = self.surface.swap(std::ptr::null_mut(), Ordering::AcqRel);
        if !pending.is_null() {
            unsafe { CFRelease(pending as *const c_void); }
        }
        // Release the previously displayed surface
        if !self.prev_surface.is_null() {
            unsafe { CFRelease(self.prev_surface as *const c_void); }
            self.prev_surface = std::ptr::null_mut();
        }

        // Close window and release on main thread
        let window_addr = self.window_ptr as usize;
        let layer_addr = self.layer_ptr as usize;
        let queue = dispatch2::DispatchQueue::main();
        queue.exec_sync(move || {
            unsafe {
                // Unhide the system cursor (display level)
                CGDisplayShowCursor(CGMainDisplayID());

                let window_ptr = window_addr as *mut NSObject;
                let _: () = msg_send![window_ptr, close];
                CFRelease(window_addr as *const c_void);
                CFRelease(layer_addr as *const c_void);
            }
        });

        println!("[zureshot-preview] Preview window closed");
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Preview refresh — timer-based display update
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Handle for the preview display refresh loop.
/// Runs a background thread that dispatches CALayer updates to main thread.
pub struct PreviewRefresh {
    running: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl PreviewRefresh {
    /// Start the refresh loop.
    /// Takes ownership of the PreviewWindow and runs display updates at ~30fps.
    pub fn start(mut preview: PreviewWindow) -> Self {
        let running = Arc::new(AtomicBool::new(true));
        let running_clone = running.clone();

        let handle = std::thread::Builder::new()
            .name("preview-refresh".into())
            .spawn(move || {
                let interval = std::time::Duration::from_millis(33); // ~30fps
                while running_clone.load(Ordering::Relaxed) {
                    preview.display_latest_frame();
                    std::thread::sleep(interval);
                }
                // Clean up on main thread
                preview.close();
            })
            .expect("Failed to spawn preview refresh thread");

        Self {
            running,
            handle: Some(handle),
        }
    }

    /// Stop the refresh loop and close the preview window.
    pub fn stop(mut self) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
        println!("[zureshot-preview] Preview refresh stopped");
    }
}

impl Drop for PreviewRefresh {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
    }
}
