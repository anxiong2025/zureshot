//! AVAssetWriter-based HEVC (H.265) hardware encoding + MP4 muxing.
//!
//! **Apple Silicon (M-series) exclusive** — leverages the dedicated media engine:
//!   - HEVC Main profile with B-frames → hardware bidirectional prediction
//!   - Quality-over-speed mode → full rate-distortion optimization in silicon
//!   - 10-bit internal pipeline even for 8-bit source (M-series native)
//!   - Zero CPU involvement — entire encode path runs on media engine
//!
//! Encoding settings:
//!   - Codec: HEVC (H.265) Main Auto Level
//!   - Resolution: always native Retina (2x) for maximum sharpness
//!   - Standard: 30 fps, quality 0.72, bitrate ceiling ~3-8 Mbps
//!   - High: 60 fps, quality 0.85, bitrate ceiling ~5-12 Mbps
//!   - B-frames: enabled (~20-30% compression for free)
//!   - Keyframe interval: 4 seconds (optimal for screen content)
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
    AVVideoMaxKeyFrameIntervalDurationKey,
    AVVideoExpectedSourceFrameRateKey,
    AVVideoCodecTypeHEVC, AVVideoAllowFrameReorderingKey,
    AVVideoQualityKey, AVVideoColorPropertiesKey,
    AVVideoColorPrimariesKey, AVVideoColorPrimaries_ITU_R_709_2,
    AVVideoTransferFunctionKey, AVVideoTransferFunction_ITU_R_709_2,
    AVVideoYCbCrMatrixKey, AVVideoYCbCrMatrix_ITU_R_709_2,
    AVVideoProfileLevelKey,
};
use objc2_foundation::{NSError, NSString, NSNumber};

use crate::capture::RecordingQuality;

/// Audio encoding settings for AAC in MP4.
const AUDIO_SAMPLE_RATE: f64 = 48000.0;
const AUDIO_CHANNELS: i32 = 2;
const AUDIO_BITRATE: i32 = 128_000; // 128 kbps AAC

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

/// Create an AVAssetWriter + AVAssetWriterInput configured for HEVC recording.
///
/// The writer is NOT started — call `start_writing()` after adding all inputs.
/// Call `finalize()` when recording is complete.
pub fn create_writer(
    output_path: &str,
    width: usize,
    height: usize,
    quality: RecordingQuality,
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
    let settings = create_video_settings(width, height, quality);

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

    // Add video input (caller adds audio inputs, then calls start_writing)
    catch_objc("addInput(video)", || unsafe {
        writer.addInput(&input);
    })?;

    println!(
        "[zureshot] Writer ready: HEVC {}x{} BT.709 → {}",
        width, height, output_str
    );
    Ok((writer, input))
}

/// Create an AVAssetWriterInput for AAC audio encoding.
///
/// Used for both system audio and microphone tracks.
pub fn create_audio_input(label: &str) -> Result<Retained<AVAssetWriterInput>, String> {
    // Audio settings dictionary:
    //   AVFormatIDKey: kAudioFormatMPEG4AAC (1633772320)
    //   AVSampleRateKey: 48000
    //   AVNumberOfChannelsKey: 2
    //   AVEncoderBitRateKey: 128000
    let settings: Retained<AnyObject> = unsafe {
        let dict: Retained<AnyObject> = msg_send![class!(NSMutableDictionary), new];

        // AVFormatIDKey = "AVFormatIDKey"
        let format_key = NSString::from_str("AVFormatIDKey");
        // kAudioFormatMPEG4AAC = 'aac ' = 1633772320
        let format_val = NSNumber::new_i32(1633772320);
        let () = msg_send![&*dict, setObject: &*format_val, forKey: &*format_key];

        // AVSampleRateKey = "AVSampleRateKey"
        let rate_key = NSString::from_str("AVSampleRateKey");
        let rate_val = NSNumber::new_f64(AUDIO_SAMPLE_RATE);
        let () = msg_send![&*dict, setObject: &*rate_val, forKey: &*rate_key];

        // AVNumberOfChannelsKey = "AVNumberOfChannelsKey"
        let ch_key = NSString::from_str("AVNumberOfChannelsKey");
        let ch_val = NSNumber::new_i32(AUDIO_CHANNELS);
        let () = msg_send![&*dict, setObject: &*ch_val, forKey: &*ch_key];

        // AVEncoderBitRateKey = "AVEncoderBitRateKey"
        let br_key = NSString::from_str("AVEncoderBitRateKey");
        let br_val = NSNumber::new_i32(AUDIO_BITRATE);
        let () = msg_send![&*dict, setObject: &*br_val, forKey: &*br_key];

        dict
    };

    // AVMediaType: "soun" (audio)
    let media_type = NSString::from_str("soun");

    let input: Retained<AVAssetWriterInput> =
        catch_objc(&format!("AVAssetWriterInput creation ({})", label), || unsafe {
            msg_send![
                class!(AVAssetWriterInput),
                assetWriterInputWithMediaType: &*media_type,
                outputSettings: &*settings
            ]
        })?;

    unsafe {
        input.setExpectsMediaDataInRealTime(true);
    }

    println!("[zureshot] Audio input ready ({}): AAC {}Hz {}ch {}kbps",
        label, AUDIO_SAMPLE_RATE as i32, AUDIO_CHANNELS, AUDIO_BITRATE / 1000);
    Ok(input)
}

/// Start the AVAssetWriter after all inputs have been added.
///
/// MUST be called after `create_writer()` and all `addInput()` calls.
/// AVAssetWriter does not allow adding inputs after writing has started.
pub fn start_writing(writer: &AVAssetWriter) -> Result<(), String> {
    catch_objc("startWriting", || unsafe {
        assert!(
            writer.startWriting(),
            "AVAssetWriter failed to start writing"
        );
    })
}

/// Finalize the recording: mark input as finished, complete the MP4 file.
///
/// This writes the moov atom and closes the file. The MP4 is not playable
/// until this completes successfully.
///
/// IMPORTANT: The caller MUST NOT hold any Mutex that Tauri sync commands
/// also acquire — GCD completion handlers may need the main thread, and
/// blocking it causes a deadlock where the moov atom is never written.
pub fn finalize(
    writer: &AVAssetWriter,
    input: &AVAssetWriterInput,
    audio_input: Option<&AVAssetWriterInput>,
    mic_input: Option<&AVAssetWriterInput>,
) {
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

    println!("[zureshot] Finalize: marking inputs as finished...");
    let mark_result = catch_objc("markAsFinished", || unsafe {
        input.markAsFinished();
        if let Some(ai) = audio_input {
            ai.markAsFinished();
            println!("[zureshot] Finalize: audio input marked finished");
        }
        if let Some(mi) = mic_input {
            mi.markAsFinished();
            println!("[zureshot] Finalize: mic input marked finished");
        }
    });
    if let Err(e) = mark_result {
        println!("[zureshot] Warning: {}", e);
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
/// Uses HEVC (H.265) for best quality-to-size ratio:
/// - Hardware-accelerated on all Apple Silicon
/// - ~40-50% smaller files than H.264 at equal quality
/// - Combined bitrate + quality targeting for optimal output
fn create_video_settings(width: usize, height: usize, quality: RecordingQuality) -> Retained<AnyObject> {
    let fps: isize = match quality {
        RecordingQuality::Standard => 30,
        RecordingQuality::High => 60,
    };

    unsafe {
        let dict: Retained<AnyObject> = msg_send![class!(NSMutableDictionary), new];

        // ── AVVideoCodecKey: HEVC (H.265) ──
        let codec_key = AVVideoCodecKey.expect("AVVideoCodecKey not available");
        let codec_val = AVVideoCodecTypeHEVC.expect("AVVideoCodecTypeHEVC not available");
        dict_set_nsstring(&dict, codec_key, codec_val);

        // ── AVVideoWidthKey / AVVideoHeightKey ──
        // HEVC also requires even dimensions
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

        // Adaptive bitrate for HEVC (lower than H.264 at same visual quality)
        let bitrate = compute_bitrate(width, height, quality);
        let bitrate_key = AVVideoAverageBitRateKey.expect("AVVideoAverageBitRateKey not available");
        let bitrate_num: Retained<AnyObject> =
            msg_send![class!(NSNumber), numberWithLongLong: bitrate];
        dict_set_nsstring(&comp, bitrate_key, &bitrate_num);

        // AVVideoQualityKey: 0.0–1.0, hint to encoder for quality-targeted VBR.
        // Combined with bitrate, the encoder uses bitrate as ceiling and quality
        // as the target — sharp screen text with minimal file size bloat.
        // Quality key: HEVC is much more efficient than H.264 at same quality
        // value. 0.72 produces razor-sharp screen text. With B-frames enabled,
        // static screen content compresses extremely well at this level.
        let quality_val: f64 = match quality {
            RecordingQuality::Standard => 0.72,
            RecordingQuality::High => 0.85,
        };
        let quality_key = AVVideoQualityKey.expect("AVVideoQualityKey not available");
        let quality_num = NSNumber::new_f64(quality_val);
        dict_set_nsstring(&comp, quality_key, &quality_num);

        // Max keyframe interval: 4 seconds (duration-based).
        // Screen content is mostly static — longer GOP = much better compression.
        // B-frames + 4s GOP = optimal compression for HEVC screen recording.
        // Still fine for seeking in video editors (most seek to nearest keyframe).
        let keyframe_key = AVVideoMaxKeyFrameIntervalDurationKey
            .expect("AVVideoMaxKeyFrameIntervalDurationKey not available");
        let keyframe_num = NSNumber::new_f64(4.0);
        dict_set_nsstring(&comp, keyframe_key, &keyframe_num);

        // Expected source frame rate — helps encoder allocate resources
        let fps_key = AVVideoExpectedSourceFrameRateKey.expect("AVVideoExpectedSourceFrameRateKey not available");
        let fps_num = NSNumber::new_isize(fps);
        dict_set_nsstring(&comp, fps_key, &fps_num);

        // ── B-frames: ENABLE frame reordering ──
        // B-frames reference both past and future frames → dramatically better
        // compression for screen content where most of the frame is static.
        // ~20-30% smaller files. No downside for recorded files (latency is
        // irrelevant — we're not streaming). Apple Silicon HEVC encoder
        // handles B-frames entirely in hardware.
        let reorder_key = AVVideoAllowFrameReorderingKey.expect("AVVideoAllowFrameReorderingKey not available");
        let yes = NSNumber::new_bool(true);
        dict_set_nsstring(&comp, reorder_key, &yes);

        // ── Encoder priority: quality over speed ──
        // "Prioritize encoding quality" tells VideoToolbox to spend more cycles
        // per frame for tighter compression. On Apple Silicon hardware encoder,
        // this costs virtually no extra CPU — the media engine has dedicated
        // rate-distortion optimization circuits.
        let priority_key = NSString::from_str("PrioritizeEncodingSpeedOverQuality");
        let priority_val = NSNumber::new_bool(false);
        dict_set_nsstring(&comp, &priority_key, &priority_val);

        // ── M-series: max performance, don't throttle for power ──
        // On Apple Silicon, the media engine is power-efficient by design.
        // Setting this to false tells VT to use full encoding throughput
        // rather than throttling for battery life — important for 60fps
        // high-quality recording on MacBooks.
        let power_key = NSString::from_str("MaximizePowerEfficiency");
        let power_val = NSNumber::new_bool(false);
        dict_set_nsstring(&comp, &power_key, &power_val);

        // ── HEVC Profile: Main Auto Level ──
        // Explicitly request Main profile to ensure the hardware encoder uses
        // the optimal encoding tools for screen content on Apple Silicon.
        // "HEVC_Main_AutoLevel" is the VideoToolbox profile string for HEVC Main.
        let profile_key = AVVideoProfileLevelKey.expect("AVVideoProfileLevelKey not available");
        let profile_val = NSString::from_str("HEVC_Main_AutoLevel");
        dict_set_nsstring(&comp, profile_key, &profile_val);

        let comp_key = AVVideoCompressionPropertiesKey.expect("AVVideoCompressionPropertiesKey not available");
        dict_set_nsstring(&dict, comp_key, &comp);

        // ── BT.709 color properties ──
        // Explicitly tag the video stream with BT.709 color space metadata.
        // This prevents implicit color space conversions between capture (sRGB)
        // and encoding that can cause softening of text edges.
        // sRGB ≈ BT.709 transfer + BT.709 primaries — a lossless metadata match.
        let color_props: Retained<AnyObject> = msg_send![class!(NSMutableDictionary), new];

        let primaries_key = AVVideoColorPrimariesKey.expect("AVVideoColorPrimariesKey not available");
        let primaries_val = AVVideoColorPrimaries_ITU_R_709_2.expect("AVVideoColorPrimaries_ITU_R_709_2 not available");
        dict_set_nsstring(&color_props, primaries_key, primaries_val);

        let transfer_key = AVVideoTransferFunctionKey.expect("AVVideoTransferFunctionKey not available");
        let transfer_val = AVVideoTransferFunction_ITU_R_709_2.expect("AVVideoTransferFunction_ITU_R_709_2 not available");
        dict_set_nsstring(&color_props, transfer_key, transfer_val);

        let matrix_key = AVVideoYCbCrMatrixKey.expect("AVVideoYCbCrMatrixKey not available");
        let matrix_val = AVVideoYCbCrMatrix_ITU_R_709_2.expect("AVVideoYCbCrMatrix_ITU_R_709_2 not available");
        dict_set_nsstring(&color_props, matrix_key, matrix_val);

        let color_props_key = AVVideoColorPropertiesKey.expect("AVVideoColorPropertiesKey not available");
        dict_set_nsstring(&dict, color_props_key, &color_props);

        dict
    }
}

/// Helper: set a key-value pair on an NSMutableDictionary using an NSString key.
unsafe fn dict_set_nsstring(dict: &AnyObject, key: &NSString, value: &AnyObject) {
    let () = msg_send![dict, setObject: value, forKey: key];
}

/// Compute VBR target bitrate based on resolution and quality.
///
/// HEVC achieves equivalent visual quality at ~60% of H.264 bitrate.
/// With B-frames enabled, actual bitrate for screen content is typically
/// 40-60% of the ceiling. High ceilings ensure fast-scrolling / video
/// playback scenes never hit the limit and degrade quality.
fn compute_bitrate(width: usize, height: usize, quality: RecordingQuality) -> i64 {
    let pixels = width * height;
    // HEVC + B-frames + quality-targeted VBR → actual bitrate for screen
    // content is typically 30-50% of the ceiling. These ceilings only kick
    // in during high-motion scenes (fast scrolling, video playback, zoom).
    match quality {
        RecordingQuality::Standard => {
            if pixels >= 3840 * 2160 {
                8_000_000   // 4K+: 8 Mbps ceiling
            } else if pixels >= 2560 * 1440 {
                6_000_000   // 1440p+: 6 Mbps ceiling
            } else if pixels >= 1920 * 1080 {
                4_000_000   // 1080p+: 4 Mbps ceiling
            } else {
                3_000_000   // Small region: 3 Mbps ceiling
            }
        }
        RecordingQuality::High => {
            if pixels >= 3840 * 2160 {
                12_000_000  // 4K+: 12 Mbps ceiling
            } else if pixels >= 2560 * 1440 {
                10_000_000  // 1440p+: 10 Mbps ceiling
            } else if pixels >= 1920 * 1080 {
                7_000_000   // 1080p+: 7 Mbps ceiling
            } else {
                5_000_000   // Small region: 5 Mbps ceiling
            }
        }
    }
}
