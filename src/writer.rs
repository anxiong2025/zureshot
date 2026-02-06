//! AVAssetWriter-based H.264 hardware encoding + MP4 muxing.
//!
//! Uses VideoToolbox hardware encoder internally (via AVAssetWriter).
//!
//! Encoding settings:
//!   - Codec: H.264 High Profile
//!   - Bitrate: VBR, adaptive to resolution
//!     - 4K:    22 Mbps average
//!     - 1440p: 14 Mbps average
//!     - 1080p: 8 Mbps average
//!   - Keyframe interval: 60 frames (1 second at 60fps)
//!   - Real-time encoding: enabled (low memory footprint)

use std::sync::mpsc;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{class, msg_send};
use objc2_av_foundation::{AVAssetWriter, AVAssetWriterInput};
use objc2_foundation::{NSError, NSString};

/// Catch ObjC exceptions and panic with a readable message.
fn catch_objc<R>(context: &str, f: impl FnOnce() -> R) -> R {
    use std::panic::AssertUnwindSafe;
    // SAFETY: We assert unwind safety here because ObjC exceptions are used
    // only for programming errors (invalid arguments), not for recoverable errors.
    objc2::exception::catch(AssertUnwindSafe(f)).unwrap_or_else(|e| {
        let desc = e
            .map(|ex| format!("{ex}"))
            .unwrap_or_else(|| "unknown ObjC exception".into());
        panic!("[zureshot] ObjC exception in {}: {}", context, desc);
    })
}

/// Create an AVAssetWriter + AVAssetWriterInput configured for H.264 recording.
///
/// The writer is started immediately (startWriting called).
/// Call `finalize()` when recording is complete.
pub fn create_writer(
    output_path: &str,
    width: usize,
    height: usize,
) -> (Retained<AVAssetWriter>, Retained<AVAssetWriterInput>) {
    // Resolve to absolute path (AVAssetWriter requires it)
    let abs_path = std::path::Path::new(output_path);
    let abs_path = if abs_path.is_absolute() {
        abs_path.to_path_buf()
    } else {
        std::env::current_dir().unwrap().join(abs_path)
    };
    let output_str = abs_path.to_str().unwrap();
    let path_str = NSString::from_str(output_str);

    // NSURL for the output file
    let url: Retained<AnyObject> =
        unsafe { msg_send![class!(NSURL), fileURLWithPath: &*path_str] };

    // AVFileType: "public.mpeg-4" (MP4 container)
    let file_type = NSString::from_str("public.mpeg-4");

    // Create AVAssetWriter
    let writer: Retained<AVAssetWriter> = catch_objc("AVAssetWriter creation", || {
        let mut error_ptr: *mut NSError = std::ptr::null_mut();
        let result: Option<Retained<AVAssetWriter>> = unsafe {
            msg_send![
                class!(AVAssetWriter),
                assetWriterWithURL: &*url,
                fileType: &*file_type,
                error: &mut error_ptr
            ]
        };
        match result {
            Some(w) => w,
            None => {
                let err = if !error_ptr.is_null() {
                    unsafe { format!("{}", &*error_ptr) }
                } else {
                    "unknown error".to_string()
                };
                panic!("Failed to create AVAssetWriter: {}", err);
            }
        }
    });

    // Video encoding settings (H.264 High Profile, VBR)
    let settings = create_video_settings(width, height);

    // AVMediaType: "vide" (video)
    let media_type = NSString::from_str("vide");

    // Create AVAssetWriterInput
    let input: Retained<AVAssetWriterInput> =
        catch_objc("AVAssetWriterInput creation", || unsafe {
            msg_send![
                class!(AVAssetWriterInput),
                assetWriterInputWithMediaType: &*media_type,
                outputSettings: &*settings
            ]
        });

    // Critical for screen recording: real-time mode keeps memory low
    // by not accumulating too many frames in the encoding pipeline
    unsafe {
        input.setExpectsMediaDataInRealTime(true);
    }

    // Wire input → writer and start
    catch_objc("addInput + startWriting", || unsafe {
        writer.addInput(&input);
        assert!(
            writer.startWriting(),
            "AVAssetWriter failed to start writing"
        );
    });

    eprintln!(
        "[zureshot] Writer ready: H.264 {}x{} → {}",
        width, height, output_str
    );
    (writer, input)
}

/// Finalize the recording: mark input as finished, complete the MP4 file.
///
/// This writes the moov atom and closes the file. The MP4 is not playable
/// until this completes successfully.
pub fn finalize(writer: &AVAssetWriter, input: &AVAssetWriterInput) {
    unsafe {
        input.markAsFinished();
    }

    let (tx, rx) = mpsc::channel();
    let handler = RcBlock::new(move || {
        let _ = tx.send(());
    });

    unsafe {
        writer.finishWritingWithCompletionHandler(&handler);
    }

    // Wait for finalization (usually < 1 second)
    match rx.recv_timeout(std::time::Duration::from_secs(30)) {
        Ok(()) => {
            let status = unsafe { writer.status() };
            if status.0 != 2 {
                // AVAssetWriterStatusCompleted = 2
                let err = unsafe { writer.error() };
                let err_str = err.map(|e| format!("{}", e)).unwrap_or_default();
                eprintln!(
                    "[zureshot] Warning: writer status {} — {}",
                    status.0, err_str
                );
            }
        }
        Err(_) => {
            eprintln!("[zureshot] Warning: finishWriting timed out (30s)");
        }
    }
}

// ────────────────────────────────────────────────────────────────
//  Video encoding settings
// ────────────────────────────────────────────────────────────────

/// Build the NSDictionary for AVAssetWriterInput video output settings.
///
/// Settings are optimized for:
/// - Maximum quality at the given resolution
/// - Low memory footprint (real-time VBR)
/// - 1-second keyframe interval for editing precision
fn create_video_settings(width: usize, height: usize) -> Retained<AnyObject> {
    unsafe {
        let dict: Retained<AnyObject> = msg_send![class!(NSMutableDictionary), new];

        // ── AVVideoCodecKey: H.264 ──
        dict_set(&dict, "AVVideoCodecKey", &*NSString::from_str("avc1"));

        // ── AVVideoWidthKey / AVVideoHeightKey ──
        // H.264 requires even dimensions — round up if needed
        let w = if width % 2 != 0 { width + 1 } else { width };
        let h = if height % 2 != 0 { height + 1 } else { height };
        let width_num: Retained<AnyObject> =
            msg_send![class!(NSNumber), numberWithInteger: w as isize];
        let height_num: Retained<AnyObject> =
            msg_send![class!(NSNumber), numberWithInteger: h as isize];
        dict_set(&dict, "AVVideoWidthKey", &*width_num);
        dict_set(&dict, "AVVideoHeightKey", &*height_num);

        // ── AVVideoCompressionPropertiesKey: encoding parameters ──
        let comp: Retained<AnyObject> = msg_send![class!(NSMutableDictionary), new];

        // Adaptive bitrate based on resolution
        let bitrate = compute_bitrate(width, height);
        let bitrate_num: Retained<AnyObject> =
            msg_send![class!(NSNumber), numberWithLongLong: bitrate];
        dict_set(&comp, "AVVideoAverageBitRateKey", &*bitrate_num);

        // Max keyframe interval: 60 frames = 1 second at 60fps
        // Enables precise editing (cut at any 1-second boundary)
        let keyframe_num: Retained<AnyObject> =
            msg_send![class!(NSNumber), numberWithInteger: 60_isize];
        dict_set(&comp, "AVVideoMaxKeyFrameIntervalKey", &*keyframe_num);

        // H.264 High Profile — best quality/compression ratio
        dict_set(
            &comp,
            "AVVideoProfileLevelKey",
            &*NSString::from_str("H264_High_AutoLevel"),
        );

        // Expected source frame rate — helps encoder allocate resources
        let fps_num: Retained<AnyObject> =
            msg_send![class!(NSNumber), numberWithInteger: 60_isize];
        dict_set(&comp, "AVVideoExpectedSourceFrameRateKey", &*fps_num);

        // Allow frame reordering: false — lower latency, slightly worse compression
        let no: Retained<AnyObject> = msg_send![class!(NSNumber), numberWithBool: false];
        dict_set(&comp, "AVVideoAllowFrameReorderingKey", &*no);

        dict_set(&dict, "AVVideoCompressionPropertiesKey", &*comp);

        dict
    }
}

/// Helper: set a key-value pair on an NSMutableDictionary via msg_send.
unsafe fn dict_set(dict: &AnyObject, key: &str, value: &AnyObject) {
    let key_str = NSString::from_str(key);
    let () = msg_send![dict, setObject: value, forKey: &*key_str];
}

/// Compute VBR target bitrate based on resolution.
///
/// These values are tuned for screen recording (sharp text, low motion):
///   - 4K (3840x2160):  22 Mbps — crisp text at native resolution
///   - 1440p:           14 Mbps — good balance
///   - 1080p:           8 Mbps  — sufficient for screen content
fn compute_bitrate(width: usize, height: usize) -> i64 {
    let pixels = width * height;
    if pixels >= 3840 * 2160 {
        22_000_000
    } else if pixels >= 2560 * 1440 {
        14_000_000
    } else {
        8_000_000
    }
}
