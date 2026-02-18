//! Linux video writer — GStreamer pipeline management via `gst-launch-1.0`.
//!
//! Architecture:
//!   XDG Portal → PipeWire node_id
//!   gst-launch-1.0: pipewiresrc → videoconvert → x264enc → mp4mux → filesink
//!   (optional)      pulsesrc → audioconvert → avenc_aac → mp4mux
//!
//! Pause/resume uses segment-based recording:
//!   - Pause: send SIGINT to gst-launch (triggers EOS, clean shutdown)
//!   - Resume: start new gst-launch process (new segment file)
//!   - Stop + Finalize: concatenate segments with ffmpeg

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use crate::platform::RecordingQuality;

/// A running GStreamer recording pipeline (gst-launch-1.0 process).
pub struct GstPipeline {
    /// The gst-launch-1.0 child process.
    process: Child,
    /// Output file path for this segment.
    output_path: PathBuf,
}

impl GstPipeline {
    /// Stop the pipeline gracefully by sending SIGINT (triggers EOS).
    ///
    /// GStreamer's `-e` flag ensures that SIGINT causes an EOS event,
    /// which flushes the muxer and writes a valid MP4 file.
    pub fn stop(&mut self) -> Result<(), String> {
        let pid = self.process.id();
        println!("[zureshot-linux] Sending SIGINT to gst-launch (PID: {})", pid);

        // Send SIGINT via kill command (avoids libc dependency)
        let _ = Command::new("kill")
            .args(["-s", "INT", &pid.to_string()])
            .output();

        // Wait for process to finish (with timeout)
        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_secs(10);

        loop {
            match self.process.try_wait() {
                Ok(Some(status)) => {
                    println!("[zureshot-linux] gst-launch exited: {}", status);
                    return Ok(());
                }
                Ok(None) => {
                    if start.elapsed() > timeout {
                        println!("[zureshot-linux] gst-launch timeout, force killing");
                        let _ = self.process.kill();
                        let _ = self.process.wait();
                        return Ok(());
                    }
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
                Err(e) => {
                    return Err(format!("Failed to wait for gst-launch: {}", e));
                }
            }
        }
    }

    /// Get the output file path of this segment.
    pub fn output_path(&self) -> &Path {
        &self.output_path
    }
}

/// Configuration for starting a GStreamer pipeline.
pub struct PipelineConfig {
    pub node_id: u32,
    pub output_path: String,
    pub fps: i32,
    pub bitrate_kbps: i32,
    /// Source stream dimensions (from portal, for crop calculation).
    pub source_width: Option<u32>,
    pub source_height: Option<u32>,
    /// Region crop in pixels: (x, y, width, height).
    pub region: Option<(i32, i32, i32, i32)>,
    pub capture_system_audio: bool,
    pub capture_mic: bool,
}

/// Build and start a GStreamer recording pipeline.
///
/// Spawns `gst-launch-1.0 -e <pipeline>` as a child process.
/// The `-e` flag ensures SIGINT triggers EOS for clean MP4 finalization.
pub fn start_pipeline(config: &PipelineConfig) -> Result<GstPipeline, String> {
    let pipeline_str = build_pipeline_string(config)?;
    println!("[zureshot-linux] GStreamer pipeline:\n  {}", pipeline_str);

    // gst-launch-1.0 takes the pipeline description as space-separated args
    let args: Vec<&str> = pipeline_str.split_whitespace().collect();

    let child = Command::new("gst-launch-1.0")
        .arg("-e") // Send EOS on SIGINT
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            format!(
                "Failed to start gst-launch-1.0: {}. \
                 Install: sudo apt install gstreamer1.0-tools gstreamer1.0-plugins-good \
                 gstreamer1.0-plugins-ugly gstreamer1.0-pipewire",
                e
            )
        })?;

    println!(
        "[zureshot-linux] gst-launch started (PID: {}), recording to: {}",
        child.id(),
        config.output_path
    );

    Ok(GstPipeline {
        process: child,
        output_path: PathBuf::from(&config.output_path),
    })
}

/// Build the GStreamer pipeline description string.
fn build_pipeline_string(config: &PipelineConfig) -> Result<String, String> {
    let has_audio = config.capture_system_audio || config.capture_mic;

    // ── Video branch ──
    let mut video_parts: Vec<String> = Vec::new();

    // PipeWire source
    video_parts.push(format!(
        "pipewiresrc path={} do-timestamp=true keepalive-time=1000",
        config.node_id
    ));

    // Color space conversion
    video_parts.push("videoconvert".to_string());

    // Region crop (if specified)
    if let Some((x, y, w, h)) = config.region {
        if let (Some(src_w), Some(src_h)) = (config.source_width, config.source_height) {
            let top = y;
            let left = x;
            let right = (src_w as i32) - x - w;
            let bottom = (src_h as i32) - y - h;

            // Only apply crop if all margins are non-negative
            if top >= 0 && left >= 0 && right >= 0 && bottom >= 0 {
                video_parts.push(format!(
                    "videocrop top={} left={} right={} bottom={}",
                    top, left, right, bottom
                ));
                println!(
                    "[zureshot-linux] Region crop: top={} left={} right={} bottom={} \
                     (source {}x{}, region {}x{}+{}+{})",
                    top, left, right, bottom, src_w, src_h, w, h, x, y
                );
            }
        }
    }

    // Frame rate control
    video_parts.push("videorate".to_string());
    video_parts.push(format!("video/x-raw,framerate={}/1", config.fps));

    // H.264 encoding (software, fast preset for real-time)
    video_parts.push(format!(
        "x264enc speed-preset=ultrafast tune=zerolatency bitrate={} key-int-max={}",
        config.bitrate_kbps,
        config.fps * 2 // Keyframe every 2 seconds
    ));
    video_parts.push("video/x-h264,profile=main".to_string());
    video_parts.push("h264parse".to_string());

    if has_audio {
        // Connect video to named muxer
        video_parts.push("mux.video_0".to_string());
    }

    // ── Audio branches (if requested) ──
    let mut audio_parts: Vec<String> = Vec::new();

    if config.capture_system_audio {
        let monitor_device = get_default_monitor_source();
        let device_arg = match &monitor_device {
            Some(dev) => format!("pulsesrc device={}", dev),
            None => "pulsesrc".to_string(),
        };
        audio_parts.push(format!(
            "{} ! audioconvert ! audioresample ! audio/x-raw,rate=48000,channels=2 \
             ! avenc_aac bitrate=128000 ! aacparse ! mux.audio_0",
            device_arg
        ));
    }

    if config.capture_mic {
        let audio_idx = if config.capture_system_audio { 1 } else { 0 };
        audio_parts.push(format!(
            "pulsesrc ! audioconvert ! audioresample ! audio/x-raw,rate=48000,channels=2 \
             ! avenc_aac bitrate=128000 ! aacparse ! mux.audio_{}",
            audio_idx
        ));
    }

    // ── Assemble full pipeline ──
    let mut full_pipeline: Vec<String> = Vec::new();

    // Video branch
    full_pipeline.push(video_parts.join(" ! "));

    if has_audio {
        for branch in &audio_parts {
            full_pipeline.push(branch.clone());
        }
        // Muxer → file (defined last, GStreamer parser resolves references)
        full_pipeline.push(format!(
            "mp4mux name=mux fragment-duration=1000 ! filesink location={}",
            config.output_path
        ));
    } else {
        // No audio: direct mux → file
        full_pipeline.push(format!(
            "! mp4mux ! filesink location={}",
            config.output_path
        ));
    }

    Ok(full_pipeline.join(" "))
}

/// Get the PulseAudio monitor source for the default audio sink.
///
/// On PipeWire (Ubuntu 24.04), `pactl` queries through the PulseAudio
/// compatibility layer. The monitor source captures system audio output.
fn get_default_monitor_source() -> Option<String> {
    let output = Command::new("pactl")
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

    let monitor = format!("{}.monitor", sink_name);
    println!("[zureshot-linux] Default audio monitor: {}", monitor);
    Some(monitor)
}

/// Concatenate multiple MP4 segments into a single file using ffmpeg.
///
/// Used after pause/resume recording to merge segment files.
/// If there's only one segment, it's simply renamed.
pub fn concatenate_segments(segments: &[PathBuf], output_path: &str) -> Result<(), String> {
    if segments.is_empty() {
        return Err("No segments to concatenate".into());
    }

    if segments.len() == 1 {
        std::fs::rename(&segments[0], output_path)
            .or_else(|_| {
                // rename may fail across filesystems, fall back to copy
                std::fs::copy(&segments[0], output_path).map(|_| ()).map_err(|e| e.to_string())
            })
            .map_err(|e| format!("Failed to move segment to output: {}", e))?;
        let _ = std::fs::remove_file(&segments[0]);
        return Ok(());
    }

    println!(
        "[zureshot-linux] Concatenating {} segments into {}",
        segments.len(),
        output_path
    );

    // Create ffmpeg concat file list
    let list_path = std::env::temp_dir().join("zureshot_concat_list.txt");
    let list_content: String = segments
        .iter()
        .map(|p| format!("file '{}'", p.display()))
        .collect::<Vec<_>>()
        .join("\n");

    std::fs::write(&list_path, &list_content)
        .map_err(|e| format!("Failed to write concat list: {}", e))?;

    let output = Command::new("ffmpeg")
        .args([
            "-y",
            "-f", "concat",
            "-safe", "0",
            "-i", &list_path.to_string_lossy(),
            "-c", "copy",
            output_path,
        ])
        .output()
        .map_err(|e| format!("ffmpeg concat failed: {}", e))?;

    // Clean up
    let _ = std::fs::remove_file(&list_path);
    for seg in segments {
        let _ = std::fs::remove_file(seg);
    }

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("ffmpeg concat failed: {}", stderr));
    }

    println!("[zureshot-linux] Segments concatenated: {}", output_path);
    Ok(())
}

/// Compute recording bitrate (kbps) based on resolution and quality.
///
/// H.264 software encoding — slightly higher bitrates than macOS HEVC
/// to compensate for less efficient codec.
pub fn compute_bitrate(width: u32, height: u32, quality: &RecordingQuality) -> i32 {
    let pixels = (width as u64) * (height as u64);
    match quality {
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
    }
}

/// Generate a segment file path from the base output path and segment index.
pub fn segment_path(base_output: &str, index: u32) -> PathBuf {
    let base = Path::new(base_output);
    let stem = base.file_stem().unwrap_or_default().to_string_lossy();
    let ext = base.extension().unwrap_or_default().to_string_lossy();
    let parent = base.parent().unwrap_or(Path::new("."));
    parent.join(format!("{}.segment_{:03}.{}", stem, index, ext))
}
