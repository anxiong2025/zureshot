//! Linux video writer — in-process GStreamer pipeline via `gstreamer-rs`.
//!
//! Phase 2.5 rewrite: replaces the gst-launch-1.0 subprocess approach with
//! a native Rust GStreamer pipeline, providing:
//!   - **Zero subprocess overhead**: pipeline runs in-process
//!   - **Native pause/resume**: GstPipeline PAUSED ↔ PLAYING (no segments)
//!   - **Hardware encoding**: VA-API (Intel/AMD) / NVENC (NVIDIA) auto-detection
//!   - **HEVC (H.265)**: when hardware encoder supports it (40% smaller files)
//!   - **EOS-based stop**: clean MP4 finalization via EOS event
//!
//! Pipeline topology:
//!   pipewiresrc → videoconvert → [videocrop] → videorate → capsfilter
//!     → encoder → parser → mp4mux → filesink
//!   [pulsesrc → audioconvert → audioresample → capsfilter
//!     → avenc_aac → aacparse → mp4mux]

use std::path::{Path, PathBuf};

use gstreamer as gst;
use gst::prelude::*;

use crate::platform::RecordingQuality;

/// Detected encoder information.
#[derive(Debug, Clone)]
pub struct EncoderInfo {
    /// GStreamer element factory name (e.g., "vaapih264enc", "x264enc").
    pub name: &'static str,
    /// Whether this is an HEVC (H.265) encoder.
    pub is_hevc: bool,
    /// Whether this is a hardware encoder.
    pub is_hardware: bool,
    /// Human-readable description.
    pub description: &'static str,
}

/// An in-process GStreamer recording pipeline.
///
/// Supports native pause/resume via GStreamer state changes.
/// No segment files, no subprocess, no ffmpeg concatenation.
pub struct GstPipeline {
    /// The GStreamer pipeline instance.
    pipeline: gst::Pipeline,
    /// Output file path.
    output_path: PathBuf,
    /// Encoder info (for logging).
    encoder_info: EncoderInfo,
}

impl GstPipeline {
    /// Get the output file path.
    pub fn output_path(&self) -> &Path {
        &self.output_path
    }

    /// Get encoder info.
    pub fn encoder_info(&self) -> &EncoderInfo {
        &self.encoder_info
    }

    /// Pause the pipeline (instant, no segment files).
    ///
    /// GStreamer handles buffering internally. When resumed, recording
    /// continues seamlessly from where it paused.
    pub fn pause(&self) -> Result<(), String> {
        println!("[zureshot-linux] Pausing GStreamer pipeline...");
        self.pipeline
            .set_state(gst::State::Paused)
            .map_err(|e| format!("Failed to pause pipeline: {e:?}"))?;
        println!("[zureshot-linux] Pipeline paused");
        Ok(())
    }

    /// Resume the pipeline (instant, no new segment).
    pub fn resume(&self) -> Result<(), String> {
        println!("[zureshot-linux] Resuming GStreamer pipeline...");
        self.pipeline
            .set_state(gst::State::Playing)
            .map_err(|e| format!("Failed to resume pipeline: {e:?}"))?;
        println!("[zureshot-linux] Pipeline resumed");
        Ok(())
    }

    /// Stop the pipeline gracefully via EOS.
    ///
    /// Sends an EOS event through the pipeline, which flushes the muxer
    /// and writes a valid MP4 file. Then transitions to Null state.
    pub fn stop(&self) -> Result<(), String> {
        println!("[zureshot-linux] Stopping GStreamer pipeline (sending EOS)...");

        // Send EOS event
        if !self.pipeline.send_event(gst::event::Eos::builder().build()) {
            println!("[zureshot-linux] Warning: failed to send EOS event");
        }

        // Wait for EOS message on the bus (or error/timeout)
        let bus = self.pipeline.bus().ok_or("Pipeline has no bus")?;
        let timeout = gst::ClockTime::from_seconds(15);
        let msg = bus.timed_pop_filtered(
            Some(timeout),
            &[gst::MessageType::Eos, gst::MessageType::Error],
        );

        match msg {
            Some(msg) => match msg.view() {
                gst::MessageView::Eos(..) => {
                    println!("[zureshot-linux] EOS received — MP4 finalized");
                }
                gst::MessageView::Error(err) => {
                    let debug = err.debug().unwrap_or_default();
                    println!(
                        "[zureshot-linux] GStreamer error during stop: {} ({debug})",
                        err.error()
                    );
                }
                _ => {}
            },
            None => {
                println!("[zureshot-linux] Warning: EOS timeout (15s), force stopping");
            }
        }

        // Transition to Null state
        self.pipeline
            .set_state(gst::State::Null)
            .map_err(|e| format!("Failed to set Null state: {e:?}"))?;

        println!(
            "[zureshot-linux] Pipeline stopped. Output: {}",
            self.output_path.display()
        );
        Ok(())
    }
}

impl Drop for GstPipeline {
    fn drop(&mut self) {
        // Ensure pipeline is cleaned up
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}

/// Configuration for building a GStreamer pipeline.
pub struct PipelineConfig {
    /// PipeWire node ID (from portal).
    pub node_id: u32,
    /// PipeWire remote fd (from portal).
    pub fd: i32,
    /// Output file path.
    pub output_path: String,
    /// Frames per second.
    pub fps: i32,
    /// Target bitrate in kbps.
    pub bitrate_kbps: i32,
    /// Source stream dimensions (from portal).
    pub source_width: Option<u32>,
    pub source_height: Option<u32>,
    /// Region crop in pixels: (x, y, width, height).
    pub region: Option<(i32, i32, i32, i32)>,
    /// Capture system audio via PulseAudio monitor source.
    pub capture_system_audio: bool,
    /// Capture microphone input.
    pub capture_mic: bool,
}

/// Detect the best available video encoder.
///
/// Priority order:
///   1. VA-API H.265 (Intel/AMD hardware, best compression)
///   2. NVENC H.265 (NVIDIA hardware, best compression)
///   3. VA-API H.264 (Intel/AMD hardware)
///   4. NVENC H.264 (NVIDIA hardware)
///   5. x264enc (software fallback, always available)
///
/// Returns info about the selected encoder.
pub fn detect_best_encoder() -> EncoderInfo {
    let candidates: &[EncoderInfo] = &[
        EncoderInfo {
            name: "vaapih265enc",
            is_hevc: true,
            is_hardware: true,
            description: "VA-API HEVC (Intel/AMD GPU)",
        },
        EncoderInfo {
            name: "nvh265enc",
            is_hevc: true,
            is_hardware: true,
            description: "NVENC HEVC (NVIDIA GPU)",
        },
        EncoderInfo {
            name: "vaapih264enc",
            is_hevc: false,
            is_hardware: true,
            description: "VA-API H.264 (Intel/AMD GPU)",
        },
        EncoderInfo {
            name: "nvh264enc",
            is_hevc: false,
            is_hardware: true,
            description: "NVENC H.264 (NVIDIA GPU)",
        },
        EncoderInfo {
            name: "x264enc",
            is_hevc: false,
            is_hardware: false,
            description: "x264 H.264 (CPU software)",
        },
    ];

    for info in candidates {
        if gst::ElementFactory::find(info.name).is_some() {
            println!(
                "[zureshot-linux] Encoder selected: {} ({})",
                info.name, info.description
            );
            return info.clone();
        }
    }

    // Ultimate fallback (x264enc should always be available)
    println!("[zureshot-linux] Warning: no encoder found, defaulting to x264enc");
    candidates.last().unwrap().clone()
}

/// Build and start an in-process GStreamer recording pipeline.
///
/// Returns a `GstPipeline` handle for pause/resume/stop control.
pub fn start_pipeline(config: &PipelineConfig) -> Result<GstPipeline, String> {
    // Initialize GStreamer (safe to call multiple times)
    gst::init().map_err(|e| format!("GStreamer init failed: {e}"))?;

    let pipeline = gst::Pipeline::default();

    // ── Video source: PipeWire ──
    let src = gst::ElementFactory::make("pipewiresrc")
        .property("fd", config.fd)
        .property("path", config.node_id.to_string())
        .property("do-timestamp", true)
        .property("keepalive-time", 1000i32)
        .build()
        .map_err(|e| format!("pipewiresrc: {e}. Install: gstreamer1.0-pipewire"))?;

    // ── Color space conversion ──
    let convert = gst::ElementFactory::make("videoconvert")
        .build()
        .map_err(|e| format!("videoconvert: {e}"))?;

    // ── Region crop (optional) ──
    let crop = if let Some((x, y, w, h)) = config.region {
        if let (Some(sw), Some(sh)) = (config.source_width, config.source_height) {
            let top = y;
            let left = x;
            let right = sw as i32 - x - w;
            let bottom = sh as i32 - y - h;
            if top >= 0 && left >= 0 && right >= 0 && bottom >= 0 {
                println!(
                    "[zureshot-linux] Crop: top={top} left={left} right={right} bottom={bottom} \
                     (src {}x{}, region {w}x{h}+{x}+{y})",
                    sw, sh
                );
                let elem = gst::ElementFactory::make("videocrop")
                    .property("top", top)
                    .property("left", left)
                    .property("right", right)
                    .property("bottom", bottom)
                    .build()
                    .map_err(|e| format!("videocrop: {e}"))?;
                Some(elem)
            } else {
                println!("[zureshot-linux] Warning: invalid crop margins, skipping crop");
                None
            }
        } else {
            None
        }
    } else {
        None
    };

    // ── Frame rate control ──
    let rate = gst::ElementFactory::make("videorate")
        .build()
        .map_err(|e| format!("videorate: {e}"))?;

    let caps_filter = gst::ElementFactory::make("capsfilter")
        .property(
            "caps",
            gst::Caps::builder("video/x-raw")
                .field("framerate", gst::Fraction::new(config.fps, 1))
                .build(),
        )
        .build()
        .map_err(|e| format!("capsfilter: {e}"))?;

    // ── Video encoder (auto-detect best available) ──
    let encoder_info = detect_best_encoder();
    let encoder = build_encoder(&encoder_info, config)?;

    // ── Parser (H.264 or H.265) ──
    let parser_name = if encoder_info.is_hevc {
        "h265parse"
    } else {
        "h264parse"
    };
    let parser = gst::ElementFactory::make(parser_name)
        .build()
        .map_err(|e| format!("{parser_name}: {e}"))?;

    // ── MP4 Muxer ──
    let mux = gst::ElementFactory::make("mp4mux")
        .name("mux")
        .property("fragment-duration", 1000u32)
        .build()
        .map_err(|e| format!("mp4mux: {e}"))?;

    // ── File sink ──
    let sink = gst::ElementFactory::make("filesink")
        .property("location", &config.output_path)
        .build()
        .map_err(|e| format!("filesink: {e}"))?;

    // ── Assemble video branch ──
    // Collect all video elements in order
    let mut video_elems: Vec<&gst::Element> = vec![&src, &convert];
    if let Some(ref c) = crop {
        video_elems.push(c);
    }
    video_elems.extend_from_slice(&[&rate, &caps_filter, &encoder, &parser]);

    // Add all video elements to pipeline
    for elem in &video_elems {
        pipeline
            .add(*elem)
            .map_err(|e| format!("Failed to add video element: {e}"))?;
    }
    pipeline
        .add_many([&mux, &sink])
        .map_err(|e| format!("Failed to add mux/sink: {e}"))?;

    // Link video chain
    gst::Element::link_many(video_elems.as_slice())
        .map_err(|e| format!("Failed to link video chain: {e}"))?;

    // Link parser → mux (video pad)
    let has_audio = config.capture_system_audio || config.capture_mic;
    if has_audio {
        parser
            .link_pads(Some("src"), &mux, Some("video_%u"))
            .map_err(|e| format!("Failed to link parser→mux: {e}"))?;
    } else {
        parser
            .link(&mux)
            .map_err(|e| format!("Failed to link parser→mux: {e}"))?;
    }

    // Link mux → filesink
    mux.link(&sink)
        .map_err(|e| format!("Failed to link mux→sink: {e}"))?;

    // ── Audio branches (optional) ──
    if config.capture_system_audio {
        add_audio_branch(&pipeline, &mux, true, "audio_0")?;
    }
    if config.capture_mic {
        let pad_name = if config.capture_system_audio {
            "audio_1"
        } else {
            "audio_0"
        };
        add_audio_branch(&pipeline, &mux, false, pad_name)?;
    }

    // ── Start playing ──
    pipeline
        .set_state(gst::State::Playing)
        .map_err(|e| format!("Failed to start pipeline: {e:?}"))?;

    println!(
        "[zureshot-linux] Pipeline started: {} @ {}fps {}kbps, encoder={}",
        config.output_path, config.fps, config.bitrate_kbps, encoder_info.name
    );

    Ok(GstPipeline {
        pipeline,
        output_path: PathBuf::from(&config.output_path),
        encoder_info,
    })
}

/// Build the video encoder element with appropriate properties.
fn build_encoder(info: &EncoderInfo, config: &PipelineConfig) -> Result<gst::Element, String> {
    let mut builder = gst::ElementFactory::make(info.name);

    match info.name {
        "x264enc" => {
            builder = builder
                .property_from_str("speed-preset", "ultrafast")
                .property_from_str("tune", "zerolatency")
                .property("bitrate", config.bitrate_kbps as u32)
                .property("key-int-max", (config.fps * 2) as u32);
        }
        "vaapih264enc" | "vaapih265enc" => {
            builder = builder.property("bitrate", config.bitrate_kbps as u32);
        }
        "nvh264enc" | "nvh265enc" => {
            builder = builder.property("bitrate", config.bitrate_kbps as u32);
        }
        "x265enc" => {
            builder = builder
                .property_from_str("speed-preset", "ultrafast")
                .property_from_str("tune", "zerolatency")
                .property("bitrate", config.bitrate_kbps as u32);
        }
        _ => {}
    }

    builder
        .build()
        .map_err(|e| format!("Failed to create encoder '{}': {e}", info.name))
}

/// Add an audio branch to the pipeline and link it to the muxer.
///
/// For system audio: uses PulseAudio monitor source (captures desktop audio).
/// For microphone: uses default PulseAudio input device.
fn add_audio_branch(
    pipeline: &gst::Pipeline,
    mux: &gst::Element,
    is_system_audio: bool,
    mux_pad_name: &str,
) -> Result<(), String> {
    let label = if is_system_audio {
        "system audio"
    } else {
        "microphone"
    };
    println!("[zureshot-linux] Adding {label} branch → mux.{mux_pad_name}");

    // Audio source
    let mut src_builder = gst::ElementFactory::make("pulsesrc");
    let monitor_source = if is_system_audio {
        get_default_monitor_source()
    } else {
        None
    };
    if let Some(ref monitor) = monitor_source {
        src_builder = src_builder.property("device", monitor.as_str());
    }
    let audio_src = src_builder
        .build()
        .map_err(|e| format!("pulsesrc ({label}): {e}"))?;

    // Audio processing
    let audio_convert = gst::ElementFactory::make("audioconvert")
        .build()
        .map_err(|e| format!("audioconvert: {e}"))?;
    let audio_resample = gst::ElementFactory::make("audioresample")
        .build()
        .map_err(|e| format!("audioresample: {e}"))?;
    let audio_caps = gst::ElementFactory::make("capsfilter")
        .property(
            "caps",
            gst::Caps::builder("audio/x-raw")
                .field("rate", 48000i32)
                .field("channels", 2i32)
                .build(),
        )
        .build()
        .map_err(|e| format!("audio capsfilter: {e}"))?;

    // AAC encoder
    let aac_enc = gst::ElementFactory::make("avenc_aac")
        .property("bitrate", 128000i32)
        .build()
        .map_err(|e| format!("avenc_aac: {e}. Install: gstreamer1.0-libav"))?;

    let aac_parse = gst::ElementFactory::make("aacparse")
        .build()
        .map_err(|e| format!("aacparse: {e}"))?;

    // Add all to pipeline
    pipeline
        .add_many([
            &audio_src,
            &audio_convert,
            &audio_resample,
            &audio_caps,
            &aac_enc,
            &aac_parse,
        ])
        .map_err(|e| format!("Failed to add {label} elements: {e}"))?;

    // Link audio chain
    gst::Element::link_many([
        &audio_src,
        &audio_convert,
        &audio_resample,
        &audio_caps,
        &aac_enc,
        &aac_parse,
    ])
    .map_err(|e| format!("Failed to link {label} chain: {e}"))?;

    // Link to muxer audio pad
    aac_parse
        .link_pads(Some("src"), mux, Some(mux_pad_name))
        .map_err(|e| format!("Failed to link {label}→mux: {e}"))?;

    Ok(())
}

/// Get the PulseAudio monitor source for the default audio sink.
///
/// On PipeWire (Ubuntu 24.04), `pactl` queries through the PulseAudio
/// compatibility layer. The monitor source captures system audio output.
fn get_default_monitor_source() -> Option<String> {
    let output = std::process::Command::new("pactl")
        .args(["get-default-sink"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let sink_name = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if sink_name.is_empty() {
        return None;
    }

    let monitor = format!("{sink_name}.monitor");
    println!("[zureshot-linux] Default audio monitor: {monitor}");
    Some(monitor)
}

/// Compute recording bitrate (kbps) based on resolution, quality, and encoder.
///
/// Hardware/HEVC encoders are more efficient, so we can use lower bitrates
/// for the same visual quality compared to x264.
pub fn compute_bitrate(
    width: u32,
    height: u32,
    quality: &RecordingQuality,
    encoder: &EncoderInfo,
) -> i32 {
    let pixels = (width as u64) * (height as u64);

    // Base bitrates for software H.264
    let base = match quality {
        RecordingQuality::Standard => {
            if pixels >= 3840 * 2160 {
                16_000
            } else if pixels >= 2560 * 1440 {
                10_000
            } else if pixels >= 1920 * 1080 {
                8_000
            } else {
                5_000
            }
        }
        RecordingQuality::High => {
            if pixels >= 3840 * 2160 {
                24_000
            } else if pixels >= 2560 * 1440 {
                16_000
            } else if pixels >= 1920 * 1080 {
                12_000
            } else {
                8_000
            }
        }
    };

    // Adjust for encoder efficiency
    if encoder.is_hevc {
        // HEVC is ~40% more efficient than H.264
        (base as f64 * 0.65) as i32
    } else if encoder.is_hardware {
        // Hardware H.264 is slightly less efficient than x264 ultrafast
        // but much faster, so use similar bitrate
        base
    } else {
        base
    }
}
