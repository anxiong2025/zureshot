//! ScreenCaptureKit capture pipeline — zero-copy screen recording.
//!
//! Architecture:
//!   SCStream → CMSampleBuffer (IOSurface-backed, GPU memory)
//!            → AVAssetWriterInput → VideoToolbox H.264 encode → MP4
//!
//! No CPU-side pixel copying in the entire path.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;

use block2::RcBlock;
use dispatch2::DispatchQueue;
use objc2::rc::Retained;
use objc2::runtime::{NSObject, ProtocolObject};
use objc2::runtime::NSObjectProtocol;
use objc2::{define_class, msg_send, AllocAnyThread, DefinedClass};
use objc2_av_foundation::{AVAssetWriter, AVAssetWriterInput};
use objc2_core_media::CMSampleBuffer;
use objc2_foundation::{NSArray, NSError};
use objc2_screen_capture_kit::{
    SCContentFilter, SCDisplay, SCShareableContent, SCStream, SCStreamConfiguration,
    SCStreamOutput, SCStreamOutputType, SCWindow,
};
use objc2_core_media::CMTime;

// ────────────────────────────────────────────────────────────────
//  StreamOutput — SCStreamOutput delegate (receives raw frames)
// ────────────────────────────────────────────────────────────────

pub struct StreamOutputIvars {
    writer: Retained<AVAssetWriter>,
    input: Retained<AVAssetWriterInput>,
    session_started: AtomicBool,
    frame_count: AtomicU64,
    dropped_count: AtomicU64,
}

define_class!(
    // SAFETY: AVAssetWriter and AVAssetWriterInput are thread-safe ObjC objects.
    // The delegate is called on a serial dispatch queue, so no concurrent access.
    #[unsafe(super(NSObject))]
    #[thread_kind = AllocAnyThread]
    #[name = "ZSStreamOutput"]
    #[ivars = StreamOutputIvars]
    pub struct StreamOutput;

    // NSObjectProtocol is required by SCStreamOutput
    unsafe impl NSObjectProtocol for StreamOutput {}

    // SCStreamOutput protocol — receives captured frames
    unsafe impl SCStreamOutput for StreamOutput {
        #[unsafe(method(stream:didOutputSampleBuffer:ofType:))]
        fn stream_didOutputSampleBuffer_ofType(
            &self,
            _stream: &SCStream,
            sample_buffer: &CMSampleBuffer,
            _output_type: SCStreamOutputType,
        ) {
            let ivars = self.ivars();

            // Start the AVAssetWriter session on the very first frame.
            // The session start time must match the first sample's PTS.
            if !ivars.session_started.swap(true, Ordering::Relaxed) {
                let pts = unsafe { sample_buffer.presentation_time_stamp() };
                unsafe {
                    let _: () = msg_send![&*ivars.writer, startSessionAtSourceTime: pts];
                }
                eprintln!("[zureshot] First frame captured, encoding started");
            }

            // Zero-copy append: CMSampleBuffer (IOSurface) → VideoToolbox encoder
            unsafe {
                let ready: bool = msg_send![&*ivars.input, isReadyForMoreMediaData];
                if ready {
                    let _: bool = msg_send![&*ivars.input, appendSampleBuffer: sample_buffer];
                    ivars.frames_inc();
                } else {
                    ivars.dropped_inc();
                }
            }
        }
    }
);

impl StreamOutputIvars {
    fn frames_inc(&self) {
        let n = self.frame_count.fetch_add(1, Ordering::Relaxed);
        // Print progress every 60 frames (~1 second at 60fps)
        if (n + 1) % 60 == 0 {
            let dropped = self.dropped_count.load(Ordering::Relaxed);
            eprint!("\r[zureshot] Frames: {} | Dropped: {}    ", n + 1, dropped);
        }
    }

    fn dropped_inc(&self) {
        self.dropped_count.fetch_add(1, Ordering::Relaxed);
    }
}

impl StreamOutput {
    fn new_with(
        writer: Retained<AVAssetWriter>,
        input: Retained<AVAssetWriterInput>,
    ) -> Retained<Self> {
        let this = Self::alloc().set_ivars(StreamOutputIvars {
            writer,
            input,
            session_started: AtomicBool::new(false),
            frame_count: AtomicU64::new(0),
            dropped_count: AtomicU64::new(0),
        });
        unsafe { msg_send![super(this), init] }
    }

    pub fn frame_count(&self) -> u64 {
        self.ivars().frame_count.load(Ordering::Relaxed)
    }

    pub fn dropped_count(&self) -> u64 {
        self.ivars().dropped_count.load(Ordering::Relaxed)
    }
}

// ────────────────────────────────────────────────────────────────
//  Public API
// ────────────────────────────────────────────────────────────────

/// Get the main display via ScreenCaptureKit.
/// Blocks until the system returns available shareable content.
pub fn get_main_display() -> Retained<SCDisplay> {
    let (tx, rx) = mpsc::channel();

    let handler = RcBlock::new(
        move |content: *mut SCShareableContent, error: *mut NSError| {
            if !error.is_null() {
                let err_desc = unsafe { format!("{}", &*error) };
                let _ = tx.send(Err(err_desc));
            } else if !content.is_null() {
                let content = unsafe { Retained::retain(content) }.unwrap();
                let _ = tx.send(Ok(content));
            } else {
                let _ = tx.send(Err("SCShareableContent: null content and null error".into()));
            }
        },
    );

    unsafe {
        SCShareableContent::getShareableContentWithCompletionHandler(&handler);
    }

    let content = rx
        .recv()
        .expect("SCShareableContent channel closed")
        .unwrap_or_else(|e| {
            eprintln!("[zureshot] ERROR: {}", e);
            eprintln!("[zureshot] Screen Recording permission required.");
            eprintln!("[zureshot] → System Settings > Privacy & Security > Screen Recording");
            eprintln!("[zureshot] → Enable your terminal app, then restart it.");
            std::process::exit(1);
        });

    let displays = unsafe { content.displays() };
    assert!(!displays.is_empty(), "No displays found");

    // Return the first (main) display — objectAtIndex returns Retained
    displays.objectAtIndex(0)
}

/// Get the pixel dimensions of a display.
pub fn display_size(display: &SCDisplay) -> (usize, usize) {
    unsafe {
        let w = display.width() as usize;
        let h = display.height() as usize;
        (w, h)
    }
}

/// Create an SCStream, wire up the delegate, and start capturing.
///
/// The delegate receives CMSampleBuffers and directly appends them to the
/// AVAssetWriterInput — zero-copy, hardware-encoded H.264.
pub fn create_and_start(
    display: &SCDisplay,
    width: usize,
    height: usize,
    writer: Retained<AVAssetWriter>,
    input: Retained<AVAssetWriterInput>,
) -> Retained<SCStream> {
    // ── Stream configuration ──
    let config = unsafe {
        let c = SCStreamConfiguration::new();
        c.setWidth(width);
        c.setHeight(height);
        // 60 fps — minimumFrameInterval is the minimum time between frames
        c.setMinimumFrameInterval(CMTime::new(1, 60));
        c.setShowsCursor(true);
        // BGRA pixel format — universally supported, hardware encoder handles conversion
        c.setPixelFormat(u32::from_be_bytes(*b"BGRA"));
        // Queue depth: keep 5 frames max in the capture queue (backpressure)
        c.setQueueDepth(5);
        c
    };

    // ── Content filter: capture entire display ──
    let empty_windows: Retained<NSArray<SCWindow>> = NSArray::new();
    let filter = unsafe {
        SCContentFilter::initWithDisplay_excludingWindows(
            SCContentFilter::alloc(),
            display,
            &empty_windows,
        )
    };

    // ── Create delegate ──
    let delegate = StreamOutput::new_with(writer, input);

    // ── Create stream ──
    let stream = unsafe {
        SCStream::initWithFilter_configuration_delegate(
            SCStream::alloc(),
            &filter,
            &config,
            None, // No SCStreamDelegate (error handling), using SCStreamOutput only
        )
    };

    // ── Add output on a dedicated serial dispatch queue ──
    let queue = DispatchQueue::new("com.zureshot.capture", None);
    unsafe {
        stream
            .addStreamOutput_type_sampleHandlerQueue_error(
                ProtocolObject::from_ref(&*delegate),
                SCStreamOutputType(0), // Screen
                Some(&queue),
            )
            .expect("Failed to add stream output");
    }

    // ── Start capture (blocking wait) ──
    let (tx, rx) = mpsc::channel();
    let start_handler = RcBlock::new(move |error: *mut NSError| {
        if !error.is_null() {
            let err = unsafe { format!("{}", &*error) };
            let _ = tx.send(Err(err));
        } else {
            let _ = tx.send(Ok(()));
        }
    });
    unsafe {
        stream.startCaptureWithCompletionHandler(Some(&start_handler));
    }
    rx.recv()
        .unwrap()
        .expect("Failed to start capture");

    // The stream retains the delegate via addStreamOutput.
    // We must NOT drop the Rust Retained<StreamOutput> early though,
    // as that would decrement the refcount. Leak it — the stream owns it now.
    std::mem::forget(delegate);

    stream
}

/// Stop the capture stream (blocking wait).
pub fn stop(stream: &SCStream) {
    let (tx, rx) = mpsc::channel();
    let handler = RcBlock::new(move |error: *mut NSError| {
        if !error.is_null() {
            let err = unsafe { format!("{}", &*error) };
            let _ = tx.send(Err(err));
        } else {
            let _ = tx.send(Ok(()));
        }
    });
    unsafe {
        stream.stopCaptureWithCompletionHandler(Some(&handler));
    }
    // Allow timeout — if capture already stopped, don't hang
    match rx.recv_timeout(std::time::Duration::from_secs(5)) {
        Ok(Ok(())) => {}
        Ok(Err(e)) => eprintln!("[zureshot] Warning: stop capture error: {}", e),
        Err(_) => eprintln!("[zureshot] Warning: stop capture timed out"),
    }
}
