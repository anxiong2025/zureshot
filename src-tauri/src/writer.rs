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
use objc2_av_foundation::{
    AVAssetWriter, AVAssetWriterInput,
    AVVideoCodecKey, AVVideoWidthKey, AVVideoHeightKey,
    AVVideoCompressionPropertiesKey, AVVideoAverageBitRateKey,
    AVVideoMaxKeyFrameIntervalKey, AVVideoProfileLevelKey,
    AVVideoExpectedSourceFrameRateKey, AVVideoProfileLevelH264HighAutoLevel,
    AVVideoCodecTypeH264, AVVideoAllowFrameReorderingKey,
};
use objc2_foundation::{NSError, NSString, NSNumber};

/// Catch ObjC exceptions and return Result instead of panicking.
fn catch_objc<R>(context: &str, f: impl FnOnce() -> R) -> Result<R, String> {
    use std::panic::AssertUnwindSafe;
    objc2::exception::catch(AssertUnwindSafe(f)).map_err(|e| {
        let desc = e
            .map(|ex| format!("{ex}"))
            .unwrap_or_else(|| "unknown ObjC exception".into());
        format!("[zureshot] ObjC exception in {}: {}", context, desc)
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
) -> Result<(Retained<AVAssetWriter>, Retained<AVAssetWriterInput>), String> {
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
            Some(w) => Ok(w),
            None => {
                let err = if !error_ptr.is_null() {
                    unsafe { format!("{}", &*error_ptr) }
                } else {
                    "unknown error".to_string()
                };
                Err(format!("Failed to create AVAssetWriter: {}", err))
            }
        }
    })??;

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
        })?;

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
    })?;

    println!(
        "[zureshot] Writer ready: H.264 {}x{} → {}",
        width, height, output_str
    );
    Ok((writer, input))
}

/// Finalize the recording: mark input as finished, complete the MP4 file.
///
/// This writes the moov atom and closes the file. The MP4 is not playable
/// until this completes successfully.
///
/// IMPORTANT: The caller MUST NOT hold any Mutex that Tauri sync commands
/// also acquire — GCD completion handlers may need the main thread, and
/// blocking it causes a deadlock where the moov atom is never written.
pub fn finalize(writer: &AVAssetWriter, input: &AVAssetWriterInput) {
    // Check writer status before finalizing
    let status_before = unsafe { writer.status() };
    println!(
        "[zureshot] Finalize: writer status = {} (0=Unknown, 1=Writing, 2=Completed, 3=Failed, 4=Cancelled)",
        status_before.0
    );

    if status_before.0 == 3 {
        let err = unsafe { writer.error() };
        let err_str = err.map(|e| format!("{}", e)).unwrap_or_default();
        println!("[zureshot] ERROR: Writer already in failed state: {}", err_str);
        return;
    }
    if status_before.0 != 1 {
        println!("[zureshot] ERROR: Writer not in Writing state, cannot finalize");
        return;
    }

    println!("[zureshot] Finalize: marking input as finished...");
    unsafe {
        input.markAsFinished();
    }

    println!("[zureshot] Finalize: calling finishWritingWithCompletionHandler...");
    let (tx, rx) = mpsc::channel();
    let handler = RcBlock::new(move || {
        println!("[zureshot] Finalize: completion handler fired!");
        let _ = tx.send(());
    });

    unsafe {
        writer.finishWritingWithCompletionHandler(&handler);
    }

    // Wait for completion, polling writer status as fallback.
    // The completion handler may not fire if GCD encounters scheduling issues,
    // but the writer status will still transition. Poll every 500ms to detect this.
    let start = std::time::Instant::now();
    let timeout = std::time::Duration::from_secs(15);

    loop {
        match rx.recv_timeout(std::time::Duration::from_millis(500)) {
            Ok(()) => break,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                println!("[zureshot] Finalize: channel disconnected unexpectedly");
                break;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // Poll writer status — it may have completed without the handler firing
                let status = unsafe { writer.status() };
                if status.0 != 1 {
                    println!(
                        "[zureshot] Finalize: writer status changed to {} (detected via poll)",
                        status.0
                    );
                    break;
                }
                if start.elapsed() > timeout {
                    println!(
                        "[zureshot] Finalize: TIMEOUT ({:.0}s) — writer still in status {}",
                        timeout.as_secs_f64(),
                        status.0
                    );
                    break;
                }
            }
        }
    }

    // Report final status
    let final_status = unsafe { writer.status() };
    if final_status.0 == 2 {
        println!("[zureshot] Finalize: SUCCESS — moov atom written");
    } else {
        let err = unsafe { writer.error() };
        let err_str = err.map(|e| format!("{}", e)).unwrap_or_default();
        println!(
            "[zureshot] Finalize: FAILED — status={}, error={}",
            final_status.0, err_str
        );
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
        let codec_key = AVVideoCodecKey.expect("AVVideoCodecKey not available");
        let codec_val = AVVideoCodecTypeH264.expect("AVVideoCodecTypeH264 not available");
        dict_set_nsstring(&dict, codec_key, codec_val);

        // ── AVVideoWidthKey / AVVideoHeightKey ──
        // H.264 requires even dimensions — round up if needed
        let w = if width % 2 != 0 { width + 1 } else { width };
        let h = if height % 2 != 0 { height + 1 } else { height };
        let width_key = AVVideoWidthKey.expect("AVVideoWidthKey not available");
        let height_key = AVVideoHeightKey.expect("AVVideoHeightKey not available");
        let width_num = NSNumber::new_isize(w as isize);
        let height_num = NSNumber::new_isize(h as isize);
        dict_set_nsstring(&dict, width_key, &width_num);
        dict_set_nsstring(&dict, height_key, &height_num);

        // ── AVVideoCompressionPropertiesKey: encoding parameters ──
        let comp: Retained<AnyObject> = msg_send![class!(NSMutableDictionary), new];

        // Adaptive bitrate based on resolution
        let bitrate = compute_bitrate(width, height);
        let bitrate_key = AVVideoAverageBitRateKey.expect("AVVideoAverageBitRateKey not available");
        let bitrate_num: Retained<AnyObject> =
            msg_send![class!(NSNumber), numberWithLongLong: bitrate];
        dict_set_nsstring(&comp, bitrate_key, &bitrate_num);

        // Max keyframe interval: 60 frames = 1 second at 60fps
        // Enables precise editing (cut at any 1-second boundary)
        let keyframe_key = AVVideoMaxKeyFrameIntervalKey.expect("AVVideoMaxKeyFrameIntervalKey not available");
        let keyframe_num = NSNumber::new_isize(60);
        dict_set_nsstring(&comp, keyframe_key, &keyframe_num);

        // H.264 High Profile — best quality/compression ratio
        let profile_key = AVVideoProfileLevelKey.expect("AVVideoProfileLevelKey not available");
        let profile_val = AVVideoProfileLevelH264HighAutoLevel.expect("AVVideoProfileLevelH264HighAutoLevel not available");
        dict_set_nsstring(&comp, profile_key, profile_val);

        // Expected source frame rate — helps encoder allocate resources
        let fps_key = AVVideoExpectedSourceFrameRateKey.expect("AVVideoExpectedSourceFrameRateKey not available");
        let fps_num = NSNumber::new_isize(60);
        dict_set_nsstring(&comp, fps_key, &fps_num);

        // Disable frame reordering for real-time screen recording (lower latency)
        let reorder_key = AVVideoAllowFrameReorderingKey.expect("AVVideoAllowFrameReorderingKey not available");
        let no = NSNumber::new_bool(false);
        dict_set_nsstring(&comp, reorder_key, &no);

        let comp_key = AVVideoCompressionPropertiesKey.expect("AVVideoCompressionPropertiesKey not available");
        dict_set_nsstring(&dict, comp_key, &comp);

        dict
    }
}

/// Helper: set a key-value pair on an NSMutableDictionary using an NSString key.
unsafe fn dict_set_nsstring(dict: &AnyObject, key: &NSString, value: &AnyObject) {
    let () = msg_send![dict, setObject: value, forKey: key];
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
