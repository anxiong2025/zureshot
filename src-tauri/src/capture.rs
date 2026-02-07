//! ScreenCaptureKit capture pipeline — zero-copy screen recording.
//!
//! Architecture:
//!   SCStream → CMSampleBuffer (IOSurface-backed, GPU memory)
//!            → AVAssetWriterInput → VideoToolbox H.264 encode → MP4
//!
//! No CPU-side pixel copying in the entire path.

use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::mpsc;

use block2::RcBlock;
use dispatch2::DispatchQueue;
use objc2::rc::Retained;
use objc2::runtime::{NSObject, ProtocolObject};
use objc2::runtime::NSObjectProtocol;
use objc2::{define_class, msg_send, AllocAnyThread, DefinedClass};
use objc2_av_foundation::{AVAssetWriter, AVAssetWriterInput};
use objc2_core_media::CMSampleBuffer;
use objc2_foundation::{NSArray, NSError, NSString};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_screen_capture_kit::{
    SCContentFilter, SCDisplay, SCShareableContent, SCStream, SCStreamConfiguration,
    SCStreamOutput, SCStreamOutputType, SCWindow,
};
use objc2_core_media::CMTime;
use serde::{Deserialize, Serialize};

/// Recording quality presets.
///
/// Both modes capture at native Retina resolution for maximum sharpness.
/// HEVC (H.265) hardware encoding keeps file sizes small.
///
/// Memory usage (2880×1800 Retina display, NV12 format):
///   Standard: native 2880×1800 @ 30fps → ~7.4 MB/frame × 3 queue = ~22 MB buffers
///   High:     native 2880×1800 @ 60fps → ~7.4 MB/frame × 3 queue = ~22 MB buffers
///
/// File size (HEVC, 60s recording at 2880×1800):
///   Standard (30fps, 10 Mbps): ~75 MB/min
///   High (60fps, 18 Mbps):     ~135 MB/min
///   CleanShot X (H.264):       ~120-180 MB/min
#[derive(Clone, Copy, Debug, Serialize, Deserialize, Default, PartialEq)]
pub enum RecordingQuality {
    /// Standard: native Retina resolution, 30 fps, HEVC.
    /// Sharp text, smooth enough for most content. Best file size.
    #[default]
    Standard,
    /// High: native Retina resolution, 60 fps, HEVC.
    /// Butter-smooth scrolling and animations. Premium quality.
    High,
}

// ────────────────────────────────────────────────────────────────
//  StreamOutput — SCStreamOutput delegate (receives raw frames)
// ────────────────────────────────────────────────────────────────

pub struct StreamOutputIvars {
    writer: Retained<AVAssetWriter>,
    input: Retained<AVAssetWriterInput>,
    /// Optional system audio writer input
    audio_input: Option<Retained<AVAssetWriterInput>>,
    /// Optional microphone writer input
    mic_input: Option<Retained<AVAssetWriterInput>>,
    session_started: AtomicBool,
    error_logged: AtomicBool,
    frame_count: AtomicU64,
    dropped_count: AtomicU64,
    /// Last appended PTS value (numerator) — for monotonicity enforcement.
    /// Stored as i64; -1 means "no frame yet".
    last_pts_value: AtomicI64,
    /// Last appended PTS timescale — for monotonicity enforcement.
    last_pts_timescale: AtomicI64,
    /// Count of frames skipped due to non-monotonic PTS.
    pts_skip_count: AtomicU64,
    /// Shared paused flag — when true, frames are dropped (not written to file).
    paused: std::sync::Arc<AtomicBool>,
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
            output_type: SCStreamOutputType,
        ) {
            let ivars = self.ivars();

            // ── 0. Paused — drop frames silently ──
            if ivars.paused.load(Ordering::Relaxed) {
                return;
            }

            // ── Route by output type ──
            // SCStreamOutputType(0) = Screen, (1) = Audio, (2) = Microphone
            if output_type.0 == 1 {
                // System audio
                if let Some(ref audio_input) = ivars.audio_input {
                    self.append_audio(sample_buffer, audio_input, ivars);
                }
                return;
            } else if output_type.0 == 2 {
                // Microphone
                if let Some(ref mic_input) = ivars.mic_input {
                    self.append_audio(sample_buffer, mic_input, ivars);
                }
                return;
            }

            // ── Screen frames (type 0) ──

            // ── 1. Validate CMSampleBuffer ──
            let is_valid: bool = unsafe { sample_buffer.is_valid() };
            if !is_valid {
                ivars.dropped_inc();
                return;
            }
            let data_ready: bool = unsafe {
                sample_buffer.data_is_ready()
            };
            if !data_ready {
                ivars.dropped_inc();
                return;
            }

            // ── 1b. Skip non-video frames ──
            // ScreenCaptureKit sends status/info frames (Started, Idle, Blank, etc.)
            // that pass is_valid/data_is_ready but contain no pixel data.
            // Appending these to AVAssetWriter puts it in a permanent failed state.
            let has_image = unsafe { sample_buffer.image_buffer() };
            if has_image.is_none() {
                return;
            }

            // ── 2. Get PTS and enforce monotonicity ──
            let pts = unsafe { sample_buffer.presentation_time_stamp() };
            // Copy packed struct fields to local variables (CMTime is repr(packed))
            let pts_value = pts.value;
            let pts_timescale = pts.timescale;
            // Skip frames with invalid timestamps
            if pts_value <= 0 || pts_timescale <= 0 {
                ivars.dropped_inc();
                return;
            }
            // Check strictly increasing PTS (compare as rational numbers)
            let prev_val = ivars.last_pts_value.load(Ordering::Relaxed);
            let prev_ts = ivars.last_pts_timescale.load(Ordering::Relaxed);
            if prev_val >= 0 && prev_ts > 0 {
                // Compare: pts_value/pts_timescale > prev_val/prev_ts
                // Cross-multiply to avoid floating point:
                let lhs = (pts_value as i128) * (prev_ts as i128);
                let rhs = (prev_val as i128) * (pts_timescale as i128);
                if lhs <= rhs {
                    // Non-monotonic PTS — skip this frame
                    let skip_n = ivars.pts_skip_count.fetch_add(1, Ordering::Relaxed);
                    if skip_n < 5 || skip_n % 100 == 0 {
                        println!(
                            "[zureshot] Skipping non-monotonic PTS: {}/{} <= {}/{} (skip #{})",
                            pts_value, pts_timescale, prev_val, prev_ts, skip_n + 1
                        );
                    }
                    ivars.dropped_inc();
                    return;
                }
            }

            // ── 3. Start session on first valid frame ──
            if !ivars.session_started.swap(true, Ordering::Relaxed) {
                unsafe {
                    let _: () = msg_send![&*ivars.writer, startSessionAtSourceTime: pts];
                }
                println!(
                    "[zureshot] First frame captured, PTS={}/{}, encoding started",
                    pts_value, pts_timescale
                );
            }

            // ── 4. Append frame to writer (zero-copy) ──
            unsafe {
                let ready: bool = msg_send![&*ivars.input, isReadyForMoreMediaData];
                if ready {
                    let ok: bool = msg_send![&*ivars.input, appendSampleBuffer: sample_buffer];
                    if ok {
                        // Update last PTS
                        ivars.last_pts_value.store(pts_value, Ordering::Relaxed);
                        ivars.last_pts_timescale.store(pts_timescale as i64, Ordering::Relaxed);
                        ivars.frames_inc();
                    } else {
                        // Writer entered failed state — log full error ONCE
                        if !ivars.error_logged.swap(true, Ordering::Relaxed) {
                            let status: i64 = msg_send![&*ivars.writer, status];
                            let error: Option<Retained<NSError>> = msg_send![&*ivars.writer, error];
                            let err_desc = error.as_ref()
                                .map(|e| format!("{}", e))
                                .unwrap_or_else(|| "unknown".into());
                            let err_domain: Option<Retained<NSString>> = error.as_ref()
                                .map(|e| e.domain());
                            let err_code: i64 = error.as_ref()
                                .map(|e| e.code() as i64)
                                .unwrap_or(-1);
                            let err_info_str = error.as_ref()
                                .map(|e| format!("{:?}", e.userInfo()))
                                .unwrap_or_else(|| "none".into());
                            println!("[zureshot] !! Writer FAILED at frame {} !!",
                                ivars.frame_count.load(Ordering::Relaxed));
                            println!("[zureshot]    status={}", status);
                            println!("[zureshot]    error={}", err_desc);
                            println!("[zureshot]    domain={:?} code={}",
                                err_domain.as_ref().map(|d| d.to_string()), err_code);
                            println!("[zureshot]    userInfo={}", err_info_str);
                            println!("[zureshot]    last PTS={}/{} current PTS={}/{}",
                                prev_val, prev_ts, pts_value, pts_timescale);
                        }
                        ivars.dropped_inc();
                    }
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
            println!("[zureshot] Frames: {} | Dropped: {}", n + 1, dropped);
        }
    }

    fn dropped_inc(&self) {
        self.dropped_count.fetch_add(1, Ordering::Relaxed);
    }
}

impl StreamOutput {
    /// Append an audio CMSampleBuffer to the given AVAssetWriterInput.
    fn append_audio(
        &self,
        sample_buffer: &CMSampleBuffer,
        audio_input: &AVAssetWriterInput,
        ivars: &StreamOutputIvars,
    ) {
        let is_valid: bool = unsafe { sample_buffer.is_valid() };
        if !is_valid { return; }
        let data_ready: bool = unsafe { sample_buffer.data_is_ready() };
        if !data_ready { return; }

        // Start session on first valid frame (video or audio, whichever comes first)
        if !ivars.session_started.swap(true, Ordering::Relaxed) {
            let pts = unsafe { sample_buffer.presentation_time_stamp() };
            unsafe {
                let _: () = msg_send![&*ivars.writer, startSessionAtSourceTime: pts];
            }
        }

        unsafe {
            let ready: bool = msg_send![audio_input, isReadyForMoreMediaData];
            if ready {
                let _ok: bool = msg_send![audio_input, appendSampleBuffer: sample_buffer];
            }
        }
    }

    fn new_with(
        writer: Retained<AVAssetWriter>,
        input: Retained<AVAssetWriterInput>,
        audio_input: Option<Retained<AVAssetWriterInput>>,
        mic_input: Option<Retained<AVAssetWriterInput>>,
        paused: std::sync::Arc<AtomicBool>,
    ) -> Retained<Self> {
        let this = Self::alloc().set_ivars(StreamOutputIvars {
            writer,
            input,
            audio_input,
            mic_input,
            session_started: AtomicBool::new(false),
            error_logged: AtomicBool::new(false),
            frame_count: AtomicU64::new(0),
            dropped_count: AtomicU64::new(0),
            last_pts_value: AtomicI64::new(-1),
            last_pts_timescale: AtomicI64::new(0),
            pts_skip_count: AtomicU64::new(0),
            paused,
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

/// Fetch SCShareableContent (blocks until the system returns it).
fn get_shareable_content() -> Result<Retained<SCShareableContent>, String> {
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

    rx.recv()
        .map_err(|_| "SCShareableContent channel closed".to_string())?
        .map_err(|e| {
            format!(
                "Screen Recording permission denied. \
                 → System Settings > Privacy & Security > Screen Recording \
                 → Enable Zureshot, then restart the app. ({})",
                e
            )
        })
}

/// Get the main display via ScreenCaptureKit.
/// Blocks until the system returns available shareable content.
pub fn get_main_display() -> Result<Retained<SCDisplay>, String> {
    let content = get_shareable_content()?;
    let displays = unsafe { content.displays() };
    if displays.is_empty() {
        return Err("No displays found".to_string());
    }
    Ok(displays.objectAtIndex(0))
}

/// Get the main display and all windows (for exclusion filtering).
pub fn get_display_and_windows() -> Result<(Retained<SCDisplay>, Vec<Retained<SCWindow>>), String> {
    let content = get_shareable_content()?;
    let displays = unsafe { content.displays() };
    if displays.is_empty() {
        return Err("No displays found".to_string());
    }
    let display = displays.objectAtIndex(0);
    let sc_windows = unsafe { content.windows() };
    let mut windows = Vec::new();
    for i in 0..sc_windows.len() {
        windows.push(sc_windows.objectAtIndex(i));
    }
    Ok((display, windows))
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
///
/// If `source_rect` is provided, only that region of the display is captured
/// (coordinates in logical points, macOS bottom-left origin).
/// `exclude_windows` are filtered out of the capture (e.g. overlay/indicator).
pub fn create_and_start(
    display: &SCDisplay,
    width: usize,
    height: usize,
    writer: Retained<AVAssetWriter>,
    input: Retained<AVAssetWriterInput>,
    audio_input: Option<Retained<AVAssetWriterInput>>,
    mic_input: Option<Retained<AVAssetWriterInput>>,
    source_rect: Option<CGRect>,
    exclude_windows: Vec<Retained<SCWindow>>,
    quality: RecordingQuality,
    paused_flag: std::sync::Arc<AtomicBool>,
    capture_system_audio: bool,
    capture_microphone: bool,
) -> Result<Retained<SCStream>, String> {
    // ── Stream configuration ──
    // H.264 requires even dimensions — round up if needed (must match writer settings)
    let width = if width % 2 != 0 { width + 1 } else { width };
    let height = if height % 2 != 0 { height + 1 } else { height };

    let fps: i32 = match quality {
        RecordingQuality::Standard => 30,
        RecordingQuality::High => 60,
    };

    let config = unsafe {
        let c = SCStreamConfiguration::new();
        c.setWidth(width);
        c.setHeight(height);
        // Frame interval based on quality
        c.setMinimumFrameInterval(CMTime::new(1, fps));
        c.setShowsCursor(true);
        // NV12 (420v) pixel format — native format for HEVC/H.264 encoding
        // BGRA requires GPU color space conversion which can fail after a few seconds.
        // 420v is what the VideoToolbox HEVC encoder natively consumes → zero-copy.
        c.setPixelFormat(u32::from_be_bytes(*b"420v"));
        // Queue depth: 3 frames (reduced from 5 for lower memory).
        // With zero-copy pipeline, frames move through quickly.
        // Lower queue = less IOSurface memory held = smaller RSS.
        c.setQueueDepth(3);

        // ── Region crop (if requested) ──
        if let Some(rect) = source_rect {
            c.setSourceRect(rect);
            c.setDestinationRect(CGRect::new(
                CGPoint::new(0.0, 0.0),
                CGSize::new(width as f64, height as f64),
            ));
            println!(
                "[zureshot] Region capture: sourceRect=({},{} {}x{}), output={}x{}",
                rect.origin.x, rect.origin.y,
                rect.size.width, rect.size.height,
                width, height
            );
        }

        // ── Audio capture ──
        if capture_system_audio {
            c.setCapturesAudio(true);
            c.setExcludesCurrentProcessAudio(true);
            c.setSampleRate(48000);
            c.setChannelCount(2);
            println!("[zureshot] System audio capture enabled (48kHz stereo)");
        }
        if capture_microphone {
            c.setCaptureMicrophone(true);
            println!("[zureshot] Microphone capture enabled");
        }

        c
    };

    println!(
        "[zureshot] Capture config: {}x{} @ {}fps, quality={:?}, queueDepth=3",
        width, height, fps, quality
    );

    // ── Content filter: capture display, excluding specified windows ──
    let exclude_array: Retained<NSArray<SCWindow>> = if exclude_windows.is_empty() {
        NSArray::new()
    } else {
        let refs: Vec<&SCWindow> = exclude_windows.iter().map(|w| &**w).collect();
        NSArray::from_slice(&refs)
    };
    let filter = unsafe {
        SCContentFilter::initWithDisplay_excludingWindows(
            SCContentFilter::alloc(),
            display,
            &exclude_array,
        )
    };

    // ── Create delegate ──
    let delegate = StreamOutput::new_with(writer, input, audio_input, mic_input, paused_flag);

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

        // Add audio output if system audio capture is enabled
        if capture_system_audio {
            stream
                .addStreamOutput_type_sampleHandlerQueue_error(
                    ProtocolObject::from_ref(&*delegate),
                    SCStreamOutputType(1), // Audio
                    Some(&queue),
                )
                .expect("Failed to add audio stream output");
            println!("[zureshot] Audio stream output added");
        }

        // Add microphone output if mic capture is enabled
        if capture_microphone {
            stream
                .addStreamOutput_type_sampleHandlerQueue_error(
                    ProtocolObject::from_ref(&*delegate),
                    SCStreamOutputType(2), // Microphone
                    Some(&queue),
                )
                .expect("Failed to add microphone stream output");
            println!("[zureshot] Microphone stream output added");
        }
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
        .map_err(|_| "Capture start channel closed".to_string())?
        .map_err(|e| format!("Failed to start capture: {}", e))?;

    // The stream retains the delegate via addStreamOutput.
    // We must NOT drop the Rust Retained<StreamOutput> early though,
    // as that would decrement the refcount. Leak it — the stream owns it now.
    std::mem::forget(delegate);

    Ok(stream)
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
        Ok(Err(e)) => println!("[zureshot] Warning: stop capture error: {}", e),
        Err(_) => println!("[zureshot] Warning: stop capture timed out"),
    }
}

/// Update the stream's content filter to exclude the given windows.
/// Called after creating new windows (recording bar, overlay) so they
/// are dynamically excluded from the capture.
pub fn update_stream_filter(
    stream: &SCStream,
    display: &SCDisplay,
    exclude_windows: Vec<Retained<SCWindow>>,
) -> Result<(), String> {
    let exclude_array: Retained<NSArray<SCWindow>> = if exclude_windows.is_empty() {
        NSArray::new()
    } else {
        let refs: Vec<&SCWindow> = exclude_windows.iter().map(|w| &**w).collect();
        NSArray::from_slice(&refs)
    };

    let filter = unsafe {
        SCContentFilter::initWithDisplay_excludingWindows(
            SCContentFilter::alloc(),
            display,
            &exclude_array,
        )
    };

    let (tx, rx) = mpsc::channel();
    let handler = RcBlock::new(move |error: *mut NSError| {
        if !error.is_null() {
            let err = unsafe { format!("{}", &*error) };
            let _ = tx.send(Err(err));
        } else {
            let _ = tx.send(Ok(()));
        }
    });

    // Call SCStream.updateContentFilter:completionHandler: via msg_send
    unsafe {
        let _: () = msg_send![stream, updateContentFilter: &*filter, completionHandler: &*handler];
    }

    match rx.recv_timeout(std::time::Duration::from_secs(5)) {
        Ok(Ok(())) => {
            println!(
                "[zureshot] Stream filter updated ({} windows excluded)",
                exclude_windows.len()
            );
            Ok(())
        }
        Ok(Err(e)) => Err(format!("updateContentFilter error: {}", e)),
        Err(_) => Err("updateContentFilter timed out".to_string()),
    }
}
