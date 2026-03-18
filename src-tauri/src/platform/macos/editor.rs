//! Video editor backend — post-recording editing powered by AVFoundation.
//!
//! Leverages Apple Silicon hardware acceleration:
//! - Media Engine: HEVC decode (AVAssetReader) & encode (AVAssetWriter)
//! - GPU: Core Image compositing (background, rounded corners, shadow)
//! - Accelerate: vDSP for audio waveform generation
//!
//! Phase E1: Video metadata, timeline thumbnails, audio waveform, stream-copy trim.

use serde::{Deserialize, Serialize};
use std::process::Command;

/// Video metadata returned to the frontend.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VideoMetadata {
    pub path: String,
    pub duration_secs: f64,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub codec: String,
    pub file_size_bytes: u64,
    pub has_audio: bool,
    pub audio_channels: u32,
    pub audio_sample_rate: u32,
    pub creation_date: String,
}

/// A single timeline thumbnail entry.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TimelineThumbnail {
    pub time_secs: f64,
    pub index: usize,
    /// Base64-encoded JPEG data (with data URL prefix)
    pub data_url: String,
}

/// Audio waveform data for the timeline.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WaveformData {
    /// Normalized amplitude values (0.0 - 1.0), one per time bucket
    pub samples: Vec<f32>,
    /// Duration of each sample bucket in seconds
    pub bucket_duration: f64,
    pub total_duration: f64,
}

/// Export progress info.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExportProgress {
    pub progress: f64,   // 0.0 - 1.0
    pub stage: String,   // "trimming", "encoding", "done", "error"
    pub output_path: Option<String>,
    pub error: Option<String>,
}

/// Background configuration for the editor.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Background {
    #[serde(rename = "gradient")]
    Gradient { colors: Vec<String>, angle: f64 },
    #[serde(rename = "solid")]
    SolidColor { color: String },
    #[serde(rename = "wallpaper")]
    Wallpaper { name: String },
    #[serde(rename = "transparent")]
    Transparent,
}

/// Zoom keyframe for auto-zoom animations.
/// Each keyframe represents a zoom "region": zoom-in → hold → zoom-out.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ZoomKeyframe {
    pub time: f64,        // seconds — center of the zoom region
    pub zoom: f64,        // 1.0 = original, 2.0 = 2x
    pub center_x: f64,    // normalized 0.0 - 1.0
    pub center_y: f64,    // normalized 0.0 - 1.0
    pub easing: String,   // "linear", "ease-in-out", "spring"
    #[serde(default = "default_hold")]
    pub hold: f64,        // seconds to stay at peak zoom (0.3 = click, 0.8+ = dwell)
}

fn default_hold() -> f64 { 0.5 }

/// Cursor overlay settings for export.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CursorSettings {
    pub enabled: bool,
    pub style: String,         // "dot", "ring", "crosshair"
    pub size: f64,             // pixels
    pub show_highlight: bool,
    pub show_click_ripple: bool,
    pub color: String,         // hex color like "#ff5050"
}

impl Default for CursorSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            style: "dot".to_string(),
            size: 20.0,
            show_highlight: true,
            show_click_ripple: true,
            color: "#ff5050".to_string(),
        }
    }
}

/// A segment range (for multi-cut export).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Segment {
    pub start: f64,
    pub end: f64,
}

/// Full video edit project descriptor.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VideoEditProject {
    pub source_path: String,
    pub trim_start: f64,
    pub trim_end: f64,
    /// If non-empty, export only these segments (cut/delete workflow).
    /// If empty, use trim_start..trim_end as a single range.
    #[serde(default)]
    pub segments: Vec<Segment>,
    pub background: Background,
    pub padding: f64,
    pub corner_radius: f64,
    pub shadow_radius: f64,
    pub shadow_opacity: f64,
    pub zoom_keyframes: Vec<ZoomKeyframe>,
    #[serde(default)]
    pub cursor: Option<CursorSettings>,
    pub output_format: String,  // "mp4", "gif"
}

// ═══════════════════════════════════════════════════════════════════════
//  Video Metadata (via mdls / AVURLAsset — no ffprobe dependency)
// ═══════════════════════════════════════════════════════════════════════

/// Get video metadata using macOS Spotlight (mdls) with AVURLAsset JXA fallback.
/// No external tools required — works on any stock macOS installation.
pub fn get_video_metadata(path: &str) -> Result<VideoMetadata, String> {
    // Try Spotlight first (fast, no decode)
    if let Ok(meta) = get_video_metadata_mdls(path) {
        if meta.duration_secs > 0.0 && meta.width > 0 {
            return Ok(meta);
        }
    }
    // Spotlight may not have indexed a freshly recorded file — fall back to AVURLAsset via JXA
    get_video_metadata_jxa(path)
}

fn get_video_metadata_mdls(path: &str) -> Result<VideoMetadata, String> {
    // Force Spotlight to index the file — important for freshly recorded files
    let _ = Command::new("mdimport").arg(path).output();

    let output = Command::new("mdls")
        .args([
            "-name", "kMDItemDurationSeconds",
            "-name", "kMDItemPixelWidth",
            "-name", "kMDItemPixelHeight",
            "-name", "kMDItemCodecs",
            "-name", "kMDItemAudioChannelCount",
            "-name", "kMDItemAudioSampleRate",
            "-name", "kMDItemVideoFrameRate",
            "-name", "kMDItemFSSize",
            "-name", "kMDItemContentCreationDate",
            path,
        ])
        .output()
        .map_err(|e| format!("mdls failed: {}", e))?;

    let text = String::from_utf8_lossy(&output.stdout);

    let duration = mdls_float(&text, "kMDItemDurationSeconds").unwrap_or(0.0);
    let width    = mdls_u32(&text, "kMDItemPixelWidth").unwrap_or(0);
    let height   = mdls_u32(&text, "kMDItemPixelHeight").unwrap_or(0);
    let fps      = mdls_float(&text, "kMDItemVideoFrameRate").unwrap_or(30.0);
    let audio_channels   = mdls_u32(&text, "kMDItemAudioChannelCount").unwrap_or(0);
    let audio_sample_rate = mdls_u32(&text, "kMDItemAudioSampleRate").unwrap_or(0);
    let creation_date = mdls_string(&text, "kMDItemContentCreationDate").unwrap_or_default();
    let codec    = mdls_codec(&text);
    let file_size = mdls_u64(&text, "kMDItemFSSize")
        .unwrap_or_else(|| std::fs::metadata(path).map(|m| m.len()).unwrap_or(0));

    Ok(VideoMetadata {
        path: path.to_string(),
        duration_secs: duration,
        width,
        height,
        fps,
        codec,
        file_size_bytes: file_size,
        has_audio: audio_channels > 0,
        audio_channels,
        audio_sample_rate,
        creation_date,
    })
}

fn mdls_raw_value(text: &str, key: &str) -> Option<String> {
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with(key) {
            if let Some(eq) = trimmed.find('=') {
                return Some(trimmed[eq + 1..].trim().to_string());
            }
        }
    }
    None
}

fn mdls_float(text: &str, key: &str) -> Option<f64> {
    let v = mdls_raw_value(text, key)?;
    if v == "(null)" { return None; }
    v.parse().ok()
}

fn mdls_u32(text: &str, key: &str) -> Option<u32> {
    let v = mdls_raw_value(text, key)?;
    if v == "(null)" { return None; }
    // mdls may return floats for integer fields on some macOS versions
    v.parse::<u32>().ok().or_else(|| v.parse::<f64>().ok().map(|f| f as u32))
}

fn mdls_u64(text: &str, key: &str) -> Option<u64> {
    let v = mdls_raw_value(text, key)?;
    if v == "(null)" { return None; }
    v.parse().ok()
}

fn mdls_string(text: &str, key: &str) -> Option<String> {
    let v = mdls_raw_value(text, key)?;
    if v == "(null)" { return None; }
    Some(v.trim_matches('"').to_string())
}

/// Parse video codec from mdls array: kMDItemCodecs = ("AAC", "H.265")
fn mdls_codec(text: &str) -> String {
    let mut in_block = false;
    let mut codecs: Vec<String> = Vec::new();
    for line in text.lines() {
        let t = line.trim_start();
        if t.starts_with("kMDItemCodecs") && t.contains('=') {
            in_block = true;
            // values may be on the same line: kMDItemCodecs = ("H.265")
            if let Some(start) = t.find('(') {
                let rest = &t[start + 1..];
                let content = rest.split(')').next().unwrap_or(rest);
                for part in content.split(',') {
                    let c = part.trim().trim_matches('"').to_string();
                    if !c.is_empty() { codecs.push(c); }
                }
                if t.contains(')') { in_block = false; }
            }
        } else if in_block {
            if t.contains(')') { in_block = false; }
            let c = t.trim_matches('"').to_string();
            if !c.is_empty() && c != "(" && c != ")" { codecs.push(c); }
        }
    }
    for c in &codecs {
        let l = c.to_lowercase();
        if l.contains("265") || l.contains("hevc") { return "hevc".to_string(); }
        if l.contains("264") || l.contains("avc")  { return "h264".to_string(); }
        if l.contains("prores")                    { return "prores".to_string(); }
    }
    codecs.into_iter().next().unwrap_or_else(|| "hevc".to_string())
}

/// Fallback metadata extraction via JXA + AVURLAsset.
/// Works for freshly recorded files before Spotlight has indexed them.
fn get_video_metadata_jxa(path: &str) -> Result<VideoMetadata, String> {
    let safe_path = path.replace('\\', "\\\\").replace('\'', "\\'");
    let script = format!(
        r#"ObjC.import('AVFoundation');
ObjC.import('Foundation');
ObjC.import('CoreMedia');

var url = $.NSURL.fileURLWithPath('{safe_path}');
var assetCls = $.NSClassFromString('AVURLAsset');
var asset = assetCls.alloc.initWithURLOptions(url, $());

var duration = $.CMTimeGetSeconds(asset.duration);

var width = 0, height = 0, fps = 30.0;
var vt = asset.tracksWithMediaType('vide');
if (vt && vt.count > 0) {{
    var t = vt.objectAtIndex(0);
    var sz = t.naturalSize;
    width  = Math.round(sz.width);
    height = Math.round(sz.height);
    fps    = t.nominalFrameRate;
}}

var hasAudio = false;
var at = asset.tracksWithMediaType('soun');
if (at && at.count > 0) {{ hasAudio = true; }}

var fileSize = 0;
try {{
    var attrs = $.NSFileManager.defaultManager.attributesOfItemAtPathError(url.path, null);
    if (attrs && !attrs.isNil()) {{ fileSize = attrs.objectForKey('NSFileSize').longLongValue; }}
}} catch(e) {{}}

JSON.stringify({{duration: duration, width: width, height: height, fps: fps,
                 hasAudio: hasAudio, fileSize: fileSize}});"#
    );

    let output = Command::new("osascript")
        .args(["-l", "JavaScript", "-e", &script])
        .output()
        .map_err(|e| format!("osascript failed: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let parsed: serde_json::Value = serde_json::from_str(&stdout)
        .map_err(|e| format!("AVURLAsset metadata parse error: {} (raw: {})", e, &stdout[..stdout.len().min(300)]))?;

    let duration  = parsed["duration"].as_f64().unwrap_or(0.0);
    let width     = parsed["width"].as_u64().unwrap_or(0) as u32;
    let height    = parsed["height"].as_u64().unwrap_or(0) as u32;
    let fps       = parsed["fps"].as_f64().unwrap_or(30.0);
    let has_audio = parsed["hasAudio"].as_bool().unwrap_or(false);
    let file_size = parsed["fileSize"].as_u64()
        .unwrap_or_else(|| std::fs::metadata(path).map(|m| m.len()).unwrap_or(0));

    Ok(VideoMetadata {
        path: path.to_string(),
        duration_secs: duration,
        width,
        height,
        fps,
        codec: "hevc".to_string(),
        file_size_bytes: file_size,
        has_audio,
        audio_channels:   if has_audio { 2 } else { 0 },
        audio_sample_rate: if has_audio { 44100 } else { 0 },
        creation_date: String::new(),
    })
}

// ═══════════════════════════════════════════════════════════════════════
//  Timeline Thumbnails (via AVAssetImageGenerator — no ffmpeg needed)
// ═══════════════════════════════════════════════════════════════════════

/// Generate timeline thumbnail strip via AVAssetImageGenerator (JXA bridge).
/// Uses VideoToolbox hardware-accelerated HEVC decoding. No external tools required.
pub fn generate_timeline_thumbnails(
    path: &str,
    count: usize,
    thumb_height: u32,
) -> Result<Vec<TimelineThumbnail>, String> {
    let meta = get_video_metadata(path)?;
    if meta.duration_secs <= 0.0 {
        return Err("Video has zero duration".into());
    }

    let count = count.max(2).min(120);
    let interval = meta.duration_secs / count as f64;
    let thumb_h = thumb_height.max(40).min(200);

    let timestamps: Vec<f64> = (0..count)
        .map(|i| (interval * i as f64 + interval * 0.5).min(meta.duration_secs - 0.01))
        .collect();

    let timestamps_json = serde_json::to_string(&timestamps).unwrap_or_else(|_| "[]".to_string());
    let safe_path = path.replace('\\', "\\\\").replace('\'', "\\'");

    let script = format!(
        r#"ObjC.import('AVFoundation');
ObjC.import('AppKit');
ObjC.import('CoreMedia');
ObjC.import('Foundation');

var videoPath = '{safe_path}';
var timestamps = {timestamps_json};
var thumbH = {thumb_h};

var url = $.NSURL.fileURLWithPath(videoPath);
var assetCls = $.NSClassFromString('AVURLAsset');
var asset = assetCls.alloc.initWithURLOptions(url, $());

var natW = 1920, natH = 1080;
var vt = asset.tracksWithMediaType('vide');
if (vt && vt.count > 0) {{
    var sz = vt.objectAtIndex(0).naturalSize;
    natW = Math.round(sz.width);
    natH = Math.round(sz.height);
}}
var thumbW = Math.round(thumbH * natW / Math.max(natH, 1) / 2) * 2;

var genCls = $.NSClassFromString('AVAssetImageGenerator');
var gen = genCls.assetImageGeneratorWithAsset(asset);
gen.appliesPreferredTrackTransform = true;
gen.maximumSize = {{width: thumbW, height: thumbH}};

var results = [];
for (var i = 0; i < timestamps.length; i++) {{
    var t = timestamps[i];
    try {{
        var cmTime = $.CMTimeMakeWithSeconds(t, 600);
        var cgImage = gen.copyCGImageAtTimeActualTimeError(cmTime, null, null);
        if (cgImage) {{
            var nsImg = $.NSImage.alloc.initWithCGImageSize(cgImage, {{width: thumbW, height: thumbH}});
            var tiffData = nsImg.TIFFRepresentation;
            var bmpRep = $.NSBitmapImageRep.imageRepWithData(tiffData);
            var props = $.NSDictionary.dictionaryWithObjectForKey(
                $.NSNumber.numberWithFloat(0.75), 'NSImageCompressionFactor'
            );
            var jpegData = bmpRep.representationUsingTypeProperties(
                $.NSBitmapImageFileTypeJPEG, props
            );
            if (jpegData && jpegData.length > 0) {{
                var b64 = ObjC.unwrap(jpegData.base64EncodedStringWithOptions(0));
                results.push({{time_secs: t, index: i, data_url: 'data:image/jpeg;base64,' + b64}});
            }}
        }}
    }} catch(e) {{}}
}}

JSON.stringify(results);"#
    );

    let output = Command::new("osascript")
        .args(["-l", "JavaScript", "-e", &script])
        .output()
        .map_err(|e| format!("thumbnail generation failed: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if stdout.is_empty() {
        return Ok(vec![]); // graceful empty — frontend handles missing thumbnails
    }

    let parsed: serde_json::Value = serde_json::from_str(&stdout)
        .map_err(|e| format!("thumbnail parse error: {} (raw: {})", e, &stdout[..stdout.len().min(200)]))?;

    let thumbnails: Vec<TimelineThumbnail> = parsed.as_array()
        .map(|arr| {
            arr.iter().filter_map(|item| {
                Some(TimelineThumbnail {
                    time_secs: item["time_secs"].as_f64()?,
                    index:     item["index"].as_u64()? as usize,
                    data_url:  item["data_url"].as_str()?.to_string(),
                })
            }).collect()
        })
        .unwrap_or_default();

    println!(
        "[editor] Generated {} thumbnails for {} ({:.1}s)",
        thumbnails.len(), path, meta.duration_secs
    );

    Ok(thumbnails)
}

// ═══════════════════════════════════════════════════════════════════════
//  Audio Waveform
// ═══════════════════════════════════════════════════════════════════════

/// Generate audio waveform data for timeline display.
/// Outputs normalized amplitude samples binned into buckets.
/// Falls back to a flat (silent) waveform if ffmpeg is not available.
pub fn generate_waveform(path: &str, num_samples: usize) -> Result<WaveformData, String> {
    let meta = get_video_metadata(path)?;
    if !meta.has_audio || meta.duration_secs <= 0.0 {
        return Ok(WaveformData {
            samples: vec![0.0; num_samples],
            bucket_duration: meta.duration_secs / num_samples as f64,
            total_duration: meta.duration_secs,
        });
    }

    let num_samples = num_samples.max(50).min(2000);

    // Extract raw PCM audio using ffmpeg (optional — falls back to flat waveform)
    let output = match Command::new("ffmpeg")
        .args([
            "-i", path,
            "-ac", "1",
            "-ar", "8000",
            "-f", "s16le",
            "-acodec", "pcm_s16le",
            "-",
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
    {
        Ok(o) => o,
        Err(_) => {
            // ffmpeg not available — return flat waveform (editor still usable)
            return Ok(WaveformData {
                samples: vec![0.0; num_samples],
                bucket_duration: meta.duration_secs / num_samples as f64,
                total_duration: meta.duration_secs,
            });
        }
    };

    let pcm_data = &output.stdout;
    let sample_count = pcm_data.len() / 2; // 16-bit samples

    if sample_count == 0 {
        return Ok(WaveformData {
            samples: vec![0.0; num_samples],
            bucket_duration: meta.duration_secs / num_samples as f64,
            total_duration: meta.duration_secs,
        });
    }

    // Convert bytes to i16 samples
    let samples: Vec<i16> = pcm_data
        .chunks_exact(2)
        .map(|chunk| i16::from_le_bytes([chunk[0], chunk[1]]))
        .collect();

    // Bin into buckets and compute RMS amplitude
    let bucket_size = (samples.len() as f64 / num_samples as f64).max(1.0) as usize;
    let mut waveform = Vec::with_capacity(num_samples);
    let mut max_rms: f64 = 0.0;

    for i in 0..num_samples {
        let start = i * bucket_size;
        let end = ((i + 1) * bucket_size).min(samples.len());
        if start >= samples.len() {
            waveform.push(0.0);
            continue;
        }

        // RMS amplitude
        let sum_sq: f64 = samples[start..end]
            .iter()
            .map(|&s| (s as f64) * (s as f64))
            .sum();
        let rms = (sum_sq / (end - start).max(1) as f64).sqrt();
        max_rms = max_rms.max(rms);
        waveform.push(rms as f32);
    }

    // Normalize to 0.0 - 1.0
    if max_rms > 0.0 {
        for s in &mut waveform {
            *s = (*s as f64 / max_rms) as f32;
        }
    }

    Ok(WaveformData {
        samples: waveform,
        bucket_duration: meta.duration_secs / num_samples as f64,
        total_duration: meta.duration_secs,
    })
}

// ═══════════════════════════════════════════════════════════════════════
//  Trim (Stream Copy — no re-encode)
// ═══════════════════════════════════════════════════════════════════════

/// Trim video using ffmpeg stream copy (zero re-encode, near-instant).
/// This leverages Apple's Media Engine for HEVC demux/mux only.
pub fn trim_video(
    input_path: &str,
    start_secs: f64,
    end_secs: f64,
    output_path: &str,
) -> Result<String, String> {
    if start_secs >= end_secs {
        return Err("Start time must be before end time".into());
    }

    let duration = end_secs - start_secs;

    println!(
        "[editor] Trimming {} → {} ({:.2}s - {:.2}s = {:.2}s)",
        input_path, output_path, start_secs, end_secs, duration
    );

    // Remove existing output
    let _ = std::fs::remove_file(output_path);

    let output = Command::new("ffmpeg")
        .args([
            "-ss", &format!("{:.3}", start_secs),
            "-i", input_path,
            "-t", &format!("{:.3}", duration),
            "-c", "copy",        // Stream copy — no re-encode!
            "-avoid_negative_ts", "make_zero",
            "-movflags", "+faststart",  // Web-friendly MP4
            "-y",
            output_path,
        ])
        .output()
        .map_err(|e| format!("ffmpeg trim failed: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("ffmpeg trim error: {}", stderr));
    }

    let file_size = std::fs::metadata(output_path)
        .map(|m| m.len())
        .unwrap_or(0);

    println!(
        "[editor] Trim complete: {} ({:.1} MB)",
        output_path,
        file_size as f64 / 1_048_576.0
    );

    Ok(output_path.to_string())
}

// ═══════════════════════════════════════════════════════════════════════
//  Render Single Preview Frame
// ═══════════════════════════════════════════════════════════════════════

/// Render a single preview frame at the given timestamp with editor effects applied.
/// Returns a base64-encoded JPEG data URL.
pub fn render_preview_frame(
    path: &str,
    time_secs: f64,
    width: u32,
    height: u32,
    _corner_radius: f64,
    _padding: f64,
    _background: &Background,
) -> Result<String, String> {
    use base64::Engine;

    // Preview frames are rendered client-side (CSS) for real-time feedback.
    // This function extracts a raw frame for edge cases / server-side rendering.
    let output = Command::new("ffmpeg")
        .args([
            "-hwaccel", "videotoolbox",
            "-ss", &format!("{:.3}", time_secs),
            "-i", path,
            "-vframes", "1",
            "-vf", &format!("scale={}:{}", width, height),
            "-f", "image2pipe",
            "-vcodec", "mjpeg",
            "-q:v", "3",
            "-",
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("ffmpeg preview frame failed: {}", e))?;

    if output.stdout.is_empty() {
        return Err("Failed to extract preview frame".into());
    }

    let b64 = base64::engine::general_purpose::STANDARD.encode(&output.stdout);
    Ok(format!("data:image/jpeg;base64,{}", b64))
}

// ═══════════════════════════════════════════════════════════════════════
//  Zoom Filter (ffmpeg crop+scale from keyframes)
// ═══════════════════════════════════════════════════════════════════════

/// Build piecewise-linear ffmpeg expressions for zoom, center_x, center_y.
///
/// Uses the same "breathe" model as the frontend preview:
///   enter (0.30s) → hold (keyframe.hold, default 0.5s) → exit (0.35s) → rest at 1x
/// Nearby keyframes (gap < 0.8s) are merged into groups that share a single
/// enter/exit, with smooth interpolation between inner keyframes.
///
/// Each expression uses nested `if(between(T,start,end), lerp, fallback)`
/// where T = `n/fps` (frame number / framerate) because the `t` variable
/// is NaN in ffmpeg's crop filter.
///
/// `time_offset` is subtracted from keyframe times to align with trimmed video.
fn build_zoom_expressions(
    keyframes: &[ZoomKeyframe],
    time_offset: f64,
    _fps: f64,
    time_var: &str,
) -> (String, String, String) {
    if keyframes.is_empty() {
        return ("1".to_string(), "0.5".to_string(), "0.5".to_string());
    }

    let mut sorted = keyframes.to_vec();
    sorted.sort_by(|a, b| a.time.partial_cmp(&b.time).unwrap_or(std::cmp::Ordering::Equal));

    // Adjust times for trim offset
    for kf in &mut sorted {
        kf.time = (kf.time - time_offset).max(0.0);
    }

    // ─── Constants MUST match frontend (VideoEditor.svelte) exactly ───
    const TRANSITION_IN: f64 = 0.55;
    const TRANSITION_OUT: f64 = 0.60;
    const DEFAULT_HOLD: f64 = 0.6;
    const MERGE_GAP: f64 = 1.0;

    // ─── Group nearby keyframes ───
    struct Group {
        kfs: Vec<ZoomKeyframe>,
    }
    let mut groups: Vec<Group> = Vec::new();
    let mut grp = Group { kfs: vec![sorted[0].clone()] };
    for i in 1..sorted.len() {
        let prev = grp.kfs.last().unwrap();
        let prev_end = prev.time + if prev.hold > 0.0 { prev.hold } else { DEFAULT_HOLD };
        if sorted[i].time - prev_end < MERGE_GAP {
            grp.kfs.push(sorted[i].clone());
        } else {
            groups.push(grp);
            grp = Group { kfs: vec![sorted[i].clone()] };
        }
    }
    groups.push(grp);

    // ─── Build segments from groups ───
    struct Seg {
        start: f64,
        end: f64,
        z0: f64, z1: f64,
        cx0: f64, cx1: f64,
        cy0: f64, cy1: f64,
    }

    let mut segs: Vec<Seg> = Vec::new();

    for g in &groups {
        let first = &g.kfs[0];
        let last = &g.kfs[g.kfs.len() - 1];

        // Enter: ramp from 1x to first keyframe zoom
        let enter_start = (first.time - TRANSITION_IN).max(0.0);
        let enter_end = first.time;
        if enter_end > enter_start + 0.005 {
            segs.push(Seg {
                start: enter_start, end: enter_end,
                z0: 1.0, z1: first.zoom,
                cx0: 0.5, cx1: first.center_x,
                cy0: 0.5, cy1: first.center_y,
            });
        }

        // Hold + inter-keyframe transitions within the group
        for (i, kf) in g.kfs.iter().enumerate() {
            let hold_dur = if kf.hold > 0.0 { kf.hold } else { DEFAULT_HOLD };
            let hold_end = kf.time + hold_dur;

            // Hold at this keyframe's zoom level
            segs.push(Seg {
                start: kf.time, end: hold_end,
                z0: kf.zoom, z1: kf.zoom,
                cx0: kf.center_x, cx1: kf.center_x,
                cy0: kf.center_y, cy1: kf.center_y,
            });

            // Transition to next keyframe in group (if any)
            if i < g.kfs.len() - 1 {
                let next = &g.kfs[i + 1];
                if next.time > hold_end + 0.005 {
                    segs.push(Seg {
                        start: hold_end, end: next.time,
                        z0: kf.zoom, z1: next.zoom,
                        cx0: kf.center_x, cx1: next.center_x,
                        cy0: kf.center_y, cy1: next.center_y,
                    });
                }
            }
        }

        // Exit: ramp from last keyframe zoom back to 1x
        let last_hold = if last.hold > 0.0 { last.hold } else { DEFAULT_HOLD };
        let exit_start = last.time + last_hold;
        let exit_end = exit_start + TRANSITION_OUT;
        segs.push(Seg {
            start: exit_start, end: exit_end,
            z0: last.zoom, z1: 1.0,
            cx0: last.center_x, cx1: 0.5,
            cy0: last.center_y, cy1: 0.5,
        });
    }

    // Helper: build one expression (z, cx, or cy) from segments.
    // Uses linear interpolation in ffmpeg (smoothstep would triple expression
    // length via p*p*(3-2*p) and exceed ffmpeg's expression parser limits).
    // The eval=frame on crop is the critical fix; easing is a minor visual diff.
    fn expr_for(segs: &[Seg], field: usize, default: &str, t_var: &str) -> String {
        let mut out = default.to_string();
        for seg in segs.iter().rev() {
            let (v0, v1) = match field {
                0 => (seg.z0, seg.z1),
                1 => (seg.cx0, seg.cx1),
                _ => (seg.cy0, seg.cy1),
            };
            let span = seg.end - seg.start;
            if span < 0.001 || (v0 - v1).abs() < 0.0005 {
                // Constant hold
                out = format!(
                    "if(between({t},{:.3},{:.3}),{:.4},{})",
                    seg.start, seg.end, v0, out,
                    t = t_var,
                );
            } else {
                // Linear interpolation: v0 + (v1-v0) * (T-start) / span
                out = format!(
                    "if(between({t},{:.3},{:.3}),{:.4}+{:.4}*({t}-{:.3})/{:.3},{})",
                    seg.start, seg.end,
                    v0, v1 - v0, seg.start, span,
                    out,
                    t = t_var,
                );
            }
        }
        out
    }

    let z  = expr_for(&segs, 0, "1", time_var);
    let cx = expr_for(&segs, 1, "0.5", time_var);
    let cy = expr_for(&segs, 2, "0.5", time_var);

    (z, cx, cy)
}

/// Build an ffmpeg zoom filter using `zoompan`.
///
/// zoompan is purpose-built for dynamic zoom+pan: it crops a region of
/// (iw/z × ih/z) from the input, then scales it up to the output size.
/// Unlike scale+crop, zoompan's `iw`/`ih` always refer to the original
/// input dimensions, so panning works correctly at all zoom levels.
///
/// Time variable: `in_time` (input timestamp in seconds). With `-ss`
/// before `-i`, timestamps start from ~0, matching our offset-adjusted
/// keyframe times. The `n` variable resets per-frame when d=1, so we
/// avoid it.
///
/// The `zoom` variable in x/y holds the current frame's computed z.
///
/// Returns empty string if no keyframes.
fn build_zoom_filter_segment(
    keyframes: &[ZoomKeyframe],
    src_w: u32,
    src_h: u32,
    time_offset: f64,
    fps: f64,
) -> String {
    if keyframes.is_empty() {
        return String::new();
    }

    // Use in_time (seconds) as time variable — works reliably in zoompan
    let t_var = "in_time";
    let (z_expr, cx_expr, cy_expr) = build_zoom_expressions(keyframes, time_offset, fps, t_var);

    // Ensure even output dimensions
    let w = (src_w / 2) * 2;
    let h = (src_h / 2) * 2;

    // zoompan: z = zoom level, d = 1 output frame per input frame
    // Visible area = (iw/z × ih/z), x/y = top-left of visible area.
    // To center at (cx, cy): x = cx*iw - iw/(2*z), y = cy*ih - ih/(2*z)
    // Clamped to [0, iw - iw/z] and [0, ih - ih/z].
    // 'zoom' in x/y refers to the current frame's computed z value.
    format!(
        "zoompan=z='{z}':\
x='clip(iw*({cx})-iw/(2*zoom),0,iw-iw/zoom)':\
y='clip(ih*({cy})-ih/(2*zoom),0,ih-ih/zoom)':\
d=1:s={w}x{h}:fps={fps}",
        z = z_expr,
        cx = cx_expr,
        cy = cy_expr,
        w = w,
        h = h,
        fps = fps as u32,
    )
}

/// Prepend a zoom (scale+crop) filter to an existing composite filter string.
///
/// If zoom keyframes exist and filter is non-empty:
///   [0:v]scale+crop[zoomed]; composite filter with [zoomed] replacing [0:v] → [out]
/// If zoom only (no composite):
///   [0:v]scale+crop[out]
/// If no zoom: returns filter unchanged.
fn prepend_zoom_to_filter(
    filter: &str,
    keyframes: &[ZoomKeyframe],
    src_w: u32,
    src_h: u32,
    time_offset: f64,
    fps: f64,
) -> String {
    if keyframes.is_empty() {
        return filter.to_string();
    }

    // Debug: log keyframe values to diagnose export zoom issues
    for (i, kf) in keyframes.iter().enumerate() {
        println!(
            "[zoom-export] kf[{}]: time={:.2}s zoom={:.2}x center=({:.3}, {:.3}) hold={:.2}s",
            i, kf.time, kf.zoom, kf.center_x, kf.center_y, kf.hold
        );
    }

    let zoom_seg = build_zoom_filter_segment(keyframes, src_w, src_h, time_offset, fps);
    println!("[zoom-export] ffmpeg filter: {}", &zoom_seg[..zoom_seg.len().min(500)]);

    if filter.is_empty() {
        // Zoom only — no background effects
        format!("[0:v]{}[out]", zoom_seg)
    } else {
        // Replace [0:v] in composite with [zoomed], prepend zoom stage
        let patched = filter.replace("[0:v]", "[zoomed]");
        format!("[0:v]{}[zoomed];{}", zoom_seg, patched)
    }
}

// ═══════════════════════════════════════════════════════════════════════
//  Cursor Overlay Filter (ffmpeg drawbox for cursor dot + click highlight)
// ═══════════════════════════════════════════════════════════════════════

/// Build cursor position expressions from mouse track samples.
///
/// Generates piecewise-constant ffmpeg expressions that step through
/// the mouse samples, linearly interpolating between adjacent points.
/// Uses `n/fps` as time variable because `t` is NaN in many ffmpeg filters.
///
/// To keep expression length manageable, we downsample to ~10 Hz.
fn build_cursor_position_exprs(
    video_path: &str,
    time_offset: f64,
    _vid_w: u32,
    _vid_h: u32,
    fps: f64,
) -> Option<(String, String, Vec<(f64, f64, f64)>)> {
    let track = super::mouse_tracker::load_mouse_track(video_path).ok()?;

    if track.samples.len() < 10 {
        return None;
    }

    // Downsample to ~10 Hz for manageable expression length
    let step = 3.max(1); // every 3rd sample from 30Hz → ~10Hz
    let samples: Vec<_> = track.samples.iter().step_by(step).collect();

    // Build piecewise-linear expression for X and Y
    let mut x_expr = format!("{:.0}", samples.last().map(|s| s.x).unwrap_or(0.0));
    let mut y_expr = format!("{:.0}", samples.last().map(|s| s.y).unwrap_or(0.0));

    // Collect click events for ripple rendering
    let mut clicks: Vec<(f64, f64, f64)> = Vec::new(); // (time, x, y) in video coords
    for s in &track.samples {
        if s.clicked {
            let t = (s.time - time_offset).max(0.0);
            // Debounce: skip if too close to previous click
            if clicks.last().map(|c: &(f64, f64, f64)| t - c.0 > 0.2).unwrap_or(true) {
                clicks.push((t, s.x, s.y));
            }
        }
    }

    // Build X/Y expressions from end to start (nested if/between)
    // Use T = n/fps as time variable since `t` is NaN in ffmpeg filters
    let t_var = format!("n/{:.2}", fps);
    for i in (0..samples.len() - 1).rev() {
        let a = samples[i];
        let b = samples[i + 1];
        let t0 = (a.time - time_offset).max(0.0);
        let t1 = (b.time - time_offset).max(0.0);

        if t1 - t0 < 0.001 { continue; }

        let span = t1 - t0;
        // Linear interp: a.x + (b.x - a.x) * (T - t0) / span
        x_expr = format!(
            "if(between({t},{:.2},{:.2}),{:.0}+{:.0}*({t}-{:.2})/{:.2},{})",
            t0, t1, a.x, b.x - a.x, t0, span, x_expr,
            t = t_var,
        );
        y_expr = format!(
            "if(between({t},{:.2},{:.2}),{:.0}+{:.0}*({t}-{:.2})/{:.2},{})",
            t0, t1, a.y, b.y - a.y, t0, span, y_expr,
            t = t_var,
        );
    }

    // If expressions get too long (>8000 chars), fallback to fewer segments
    if x_expr.len() > 8000 {
        // Ultra-downsample: every 10th original sample
        let step2 = 10;
        let samples2: Vec<_> = track.samples.iter().step_by(step2).collect();
        x_expr = format!("{:.0}", samples2.last().map(|s| s.x).unwrap_or(0.0));
        y_expr = format!("{:.0}", samples2.last().map(|s| s.y).unwrap_or(0.0));

        for i in (0..samples2.len() - 1).rev() {
            let a = samples2[i];
            let b = samples2[i + 1];
            let t0 = (a.time - time_offset).max(0.0);
            let t1 = (b.time - time_offset).max(0.0);
            if t1 - t0 < 0.001 { continue; }
            let span = t1 - t0;
            x_expr = format!(
                "if(between({t},{:.2},{:.2}),{:.0}+{:.0}*({t}-{:.2})/{:.2},{})",
                t0, t1, a.x, b.x - a.x, t0, span, x_expr,
                t = t_var,
            );
            y_expr = format!(
                "if(between({t},{:.2},{:.2}),{:.0}+{:.0}*({t}-{:.2})/{:.2},{})",
                t0, t1, a.y, b.y - a.y, t0, span, y_expr,
                t = t_var,
            );
        }
    }

    Some((x_expr, y_expr, clicks))
}

/// Build an ffmpeg filter segment to draw cursor dot + click highlights.
///
/// Uses multiple drawbox filters layered to approximate a circle:
///   - Outer glow (highlight)
///   - Inner cursor dot
///   - Per-click expanding highlight rings
///
/// The cursor position tracks the mouse at screen coordinates, scaled by
/// video dimensions. After zoom crop, the cursor needs to be in post-crop space.
fn build_cursor_filter(
    video_path: &str,
    cursor: &CursorSettings,
    time_offset: f64,
    vid_w: u32,
    vid_h: u32,
    fps: f64,
) -> String {
    let exprs = build_cursor_position_exprs(video_path, time_offset, vid_w, vid_h, fps);

    let (x_expr, y_expr, clicks) = match exprs {
        Some(e) => e,
        None => return String::new(),
    };

    // Use cursor settings for size and color
    let r = (cursor.size / 2.5).max(4.0) as i32; // dot radius in pixels
    let color = &cursor.color; // hex like "#ff5050"
    // Convert hex to ffmpeg-friendly format (0xRRGGBB)
    let ffmpeg_color = if color.starts_with('#') {
        format!("0x{}", &color[1..])
    } else {
        "0xff5050".to_string()
    };

    let mut filters = Vec::new();

    // Draw cursor dot: a small filled rectangle (approximates circle)
    // drawbox: x, y, w, h, color, thickness (fill = max)
    filters.push(format!(
        "drawbox=x='clip({x}-{r},0,iw-{d})':y='clip({y}-{r},0,ih-{d})':w={d}:h={d}:color={c}@0.85:t=fill",
        x = x_expr,
        y = y_expr,
        r = r,
        d = r * 2,
        c = ffmpeg_color,
    ));

    // Draw click highlights: brief flash of a larger box at click positions
    if cursor.show_click_ripple {
        let t_var = format!("n/{:.2}", fps);
        for (_i, (ct, cx, cy)) in clicks.iter().enumerate().take(50) {
            // Visible for 0.3 seconds
            let t_end = ct + 0.3;
            let size = (cursor.size * 1.5) as i32;
            let half = size / 2;
            filters.push(format!(
                "drawbox=x='if(between({t},{t0:.2},{t1:.2}),clip({cx:.0}-{half},0,iw-{size}),0)':y='if(between({t},{t0:.2},{t1:.2}),clip({cy:.0}-{half},0,ih-{size}),0)':w='if(between({t},{t0:.2},{t1:.2}),{size},0)':h='if(between({t},{t0:.2},{t1:.2}),{size},0)':color={c}@0.4:t=4",
                t = t_var,
                t0 = ct,
                t1 = t_end,
                cx = cx,
                cy = cy,
                half = half,
                size = size,
                c = ffmpeg_color,
            ));
        }
    }

    filters.join(",")
}

/// Append cursor filter to an existing filter string.
///
/// Cursor is drawn AFTER zoom and AFTER composite — it goes on the
/// final [out] label or on the raw stream.
fn append_cursor_to_filter(
    filter: &str,
    video_path: &str,
    cursor: &CursorSettings,
    time_offset: f64,
    vid_w: u32,
    vid_h: u32,
    fps: f64,
) -> String {
    if !cursor.enabled {
        return filter.to_string();
    }

    let cursor_filter = build_cursor_filter(video_path, cursor, time_offset, vid_w, vid_h, fps);
    if cursor_filter.is_empty() {
        return filter.to_string();
    }

    if filter.is_empty() {
        // No existing filter — apply cursor directly
        format!("[0:v]{cursor}[out]", cursor = cursor_filter)
    } else if filter.contains("[out]") {
        // Has [out] label — rename [out] to [pre_cursor], apply cursor on it
        let patched = filter.replacen("[out]", "[pre_cursor]", 1);
        // Find last occurrence of [out] in output mapping (there might be multiple)
        // Just replace the output label
        format!("{patched};[pre_cursor]{cursor}[out]",
            patched = patched,
            cursor = cursor_filter,
        )
    } else {
        // No labels — just chain
        format!("{},{}", filter, cursor_filter)
    }
}

// ═══════════════════════════════════════════════════════════════════════
//  Background Compositing Helpers (ffmpeg filter_complex)
// ═══════════════════════════════════════════════════════════════════════

/// Build an ffmpeg filter_complex string for compositing video onto a background.
/// The pipeline:
///   1. Scale video to fit within canvas (canvas_w - 2*padding) × (canvas_h - 2*padding)
///   2. Apply rounded corners via alpha mask (geq filter)
///   3. Generate background (gradient or solid color)
///   4. Composite with shadow (optional)
///   5. Overlay video on background with centering
fn build_composite_filter(
    src_w: u32,
    src_h: u32,
    background: &Background,
    padding: f64,
    corner_radius: f64,
    shadow_radius: f64,
    shadow_opacity: f64,
) -> (String, u32, u32) {
    let pad = padding.max(0.0) as u32;
    let cr = corner_radius.max(0.0) as u32;

    // Video dimensions within the canvas
    let vid_w = src_w;
    let vid_h = src_h;

    // Canvas = video + padding on all sides
    let canvas_w = vid_w + pad * 2;
    let canvas_h = vid_h + pad * 2;

    // Ensure all dimensions are even (ffmpeg requirement for most codecs)
    let canvas_w = (canvas_w / 2) * 2;
    let canvas_h = (canvas_h / 2) * 2;

    let mut filters = Vec::new();

    if pad == 0 && cr == 0 {
        // No background effects — just pass through
        return ("".to_string(), vid_w, vid_h);
    }

    // ─── Step 1: Round corners on the video ───
    if cr > 0 {
        // Use geq filter to apply rounded rectangle alpha mask
        // This creates a smooth rounded corner effect
        filters.push(format!(
            "[0:v]format=yuva420p,geq=\
lum='lum(X,Y)':\
cb='cb(X,Y)':\
cr='cr(X,Y)':\
a='if(gt(pow(max(0,{cr}-X),2)+pow(max(0,{cr}-Y),2),pow({cr},2)),0,\
if(gt(pow(max(0,X-W+{cr}),2)+pow(max(0,{cr}-Y),2),pow({cr},2)),0,\
if(gt(pow(max(0,{cr}-X),2)+pow(max(0,Y-H+{cr}),2),pow({cr},2)),0,\
if(gt(pow(max(0,X-W+{cr}),2)+pow(max(0,Y-H+{cr}),2),pow({cr},2)),0,\
255))))'[rounded]",
            cr = cr
        ));
    } else {
        filters.push("[0:v]copy[rounded]".to_string());
    }

    // ─── Step 2: Generate background ───
    let bg_color = match background {
        Background::SolidColor { color } => {
            // Parse hex color for ffmpeg
            let c = color.trim_start_matches('#');
            format!("0x{}FF", c)
        }
        Background::Gradient { colors, angle: _ } => {
            // ffmpeg can't natively do gradients; we'll use gradients filter
            // or simply use the first color as base + second as top overlay.
            // For simplicity, use the first color. Phase E4 will use Core Image for true gradients.
            let c = colors.first()
                .map(|s| s.trim_start_matches('#'))
                .unwrap_or("6c5ce7");
            format!("0x{}FF", c)
        }
        Background::Transparent | Background::Wallpaper { .. } => {
            "0x000000FF".to_string()
        }
    };

    // For gradients, use a two-color approach with ffmpeg's gradients filter
    match background {
        Background::Gradient { colors, angle } => {
            let c0 = colors.first().map(|s| s.as_str()).unwrap_or("#6c5ce7");
            let c1 = colors.get(1).map(|s| s.as_str()).unwrap_or("#a29bfe");
            // Convert hex to ffmpeg-compatible format
            let fc0 = hex_to_ffmpeg_color(c0);
            let fc1 = hex_to_ffmpeg_color(c1);

            // Determine gradient direction based on angle
            let (x0, y0, x1, y1) = angle_to_gradient_points(*angle);

            filters.push(format!(
                "color=c=black:s={cw}x{ch}:d=1,format=rgba,\
geq=r='clip(({r0}*(1-p)+{r1}*p),0,255)':\
g='clip(({g0}*(1-p)+{g1}*p),0,255)':\
b='clip(({b0}*(1-p)+{b1}*p),0,255)':\
a=255[bg]",
                cw = canvas_w,
                ch = canvas_h,
                r0 = fc0.0, g0 = fc0.1, b0 = fc0.2,
                r1 = fc1.0, g1 = fc1.1, b1 = fc1.2,
            ));

            // Actually, the geq gradient needs a position variable.
            // Let's use a simpler approach: two solid colors blended with gradients filter
            // OR use the 'gradients' source filter if available.
            // Simplest reliable approach: use a vertical gradient via geq with normalized Y.
            let _ = (x0, y0, x1, y1);
            // Override: use an actual gradient via geq
            filters.pop(); // remove the previous attempt
            filters.push(format!(
                "color=c=black:s={cw}x{ch}:d=1,format=rgb24,\
geq=\
r='clip({r0}+(({r1}-{r0})*({a}*X/{cw}+{b}*Y/{ch}))/({a}+{b}+0.001),0,255)':\
g='clip({g0}+(({g1}-{g0})*({a}*X/{cw}+{b}*Y/{ch}))/({a}+{b}+0.001),0,255)':\
b='clip({b0}+(({b1}-{b0})*({a}*X/{cw}+{b}*Y/{ch}))/({a}+{b}+0.001),0,255)'[bg]",
                cw = canvas_w,
                ch = canvas_h,
                r0 = fc0.0, g0 = fc0.1, b0 = fc0.2,
                r1 = fc1.0, g1 = fc1.1, b1 = fc1.2,
                a = x1, b = y1,
            ));
        }
        _ => {
            filters.push(format!(
                "color=c={}:s={}x{}:d=1[bg]",
                bg_color.replace("0x", "#").trim_end_matches("FF"),
                canvas_w, canvas_h
            ));
        }
    }

    // ─── Step 3: Shadow (optional) ───
    if shadow_opacity > 0.0 && shadow_radius > 0.0 {
        // Create shadow: duplicate the rounded video, tint black, blur, and place under
        let blur_sigma = (shadow_radius / 2.0).max(1.0);
        let _shadow_alpha = (shadow_opacity * 255.0).min(255.0) as u32;
        filters.push(format!(
            "[rounded]split[vid][shadow_src];\
[shadow_src]colorchannelmixer=aa={alpha_f:.2},\
colorchannelmixer=rr=0:gg=0:bb=0:ra=0:ga=0:ba=0,\
gblur=sigma={sigma:.1}[shadow];\
[bg][shadow]overlay=(W-w)/2+0:(H-h)/2+4:format=auto[bg_shadow];\
[bg_shadow][vid]overlay=(W-w)/2:(H-h)/2:format=auto[out]",
            alpha_f = shadow_opacity.min(1.0),
            sigma = blur_sigma,
        ));
    } else {
        // No shadow — direct overlay
        filters.push(format!(
            "[bg][rounded]overlay=(W-w)/2:(H-h)/2:format=auto[out]"
        ));
    }

    let filter_str = filters.join(";");
    (filter_str, canvas_w, canvas_h)
}

/// Convert hex color string (#RRGGBB) to (R, G, B) tuple
fn hex_to_ffmpeg_color(hex: &str) -> (u32, u32, u32) {
    let hex = hex.trim_start_matches('#');
    if hex.len() >= 6 {
        let r = u32::from_str_radix(&hex[0..2], 16).unwrap_or(0);
        let g = u32::from_str_radix(&hex[2..4], 16).unwrap_or(0);
        let b = u32::from_str_radix(&hex[4..6], 16).unwrap_or(0);
        (r, g, b)
    } else {
        (0, 0, 0)
    }
}

/// Convert gradient angle (degrees) to normalized direction weights for X and Y
/// 0° = bottom to top, 90° = left to right, 135° = top-left to bottom-right, etc.
fn angle_to_gradient_points(angle: f64) -> (f64, f64, f64, f64) {
    let rad = angle.to_radians();
    let x = rad.sin(); // horizontal component
    let y = rad.cos(); // vertical component
    // Return weights for X and Y blending
    // Positive = goes right/down in the gradient direction
    (0.0, 0.0, x.abs().max(0.01), y.abs().max(0.01))
}

// ═══════════════════════════════════════════════════════════════════════
//  Export with Effects (Phase E2+)
// ═══════════════════════════════════════════════════════════════════════

/// Export video with editor effects — background, rounded corners, zoom, etc.
/// For Phase E1 (no background): trim + stream copy.
/// For Phase E2 (background): ffmpeg filter_complex GPU compositing.
/// Clean up source video and its mouse track file after export.
fn cleanup_source_and_track(source_path: &str) {
    let video = std::path::Path::new(source_path);
    let dir = video.parent().unwrap_or(std::path::Path::new("."));
    let stem = video.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let track_path = dir.join(format!(".{}.mousetrack.json", stem));

    if let Err(e) = std::fs::remove_file(source_path) {
        println!("[editor] Could not delete original: {}", e);
    } else {
        println!("[editor] Deleted original: {}", source_path);
    }
    if track_path.exists() {
        let _ = std::fs::remove_file(&track_path);
    }
}

/// Export multiple segments by concatenating them with ffmpeg.
/// Each segment is trimmed and then joined using the concat demuxer.
fn export_multi_segment(
    input_path: &str,
    segments: &[Segment],
    output_path: &str,
    as_gif: bool,
) -> Result<String, String> {
    let tmp_dir = std::env::temp_dir().join("zureshot_cuts");
    let _ = std::fs::create_dir_all(&tmp_dir);

    // Step 1: Export each segment as a separate file
    let mut part_paths: Vec<String> = Vec::new();
    for (i, seg) in segments.iter().enumerate() {
        let part_path = tmp_dir.join(format!("part_{}.mp4", i)).to_string_lossy().to_string();
        let dur = seg.end - seg.start;

        let output = Command::new("ffmpeg")
            .args([
                "-ss", &format!("{:.3}", seg.start),
                "-i", input_path,
                "-t", &format!("{:.3}", dur),
                "-c:v", "hevc_videotoolbox",
                "-q:v", "60",
                "-tag:v", "hvc1",
                "-c:a", "aac",
                "-b:a", "128k",
                "-y", &part_path,
            ])
            .output()
            .map_err(|e| format!("ffmpeg segment {} failed: {}", i, e))?;

        if !output.status.success() {
            // Fallback to software encoding
            let output = Command::new("ffmpeg")
                .args([
                    "-ss", &format!("{:.3}", seg.start),
                    "-i", input_path,
                    "-t", &format!("{:.3}", dur),
                    "-c:v", "libx264", "-preset", "fast", "-crf", "20",
                    "-pix_fmt", "yuv420p",
                    "-c:a", "aac", "-b:a", "128k",
                    "-y", &part_path,
                ])
                .output()
                .map_err(|e| format!("ffmpeg segment {} SW failed: {}", i, e))?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(format!("Segment {} export failed: {}", i, stderr));
            }
        }
        part_paths.push(part_path);
    }

    // Step 2: Create concat file list
    let list_path = tmp_dir.join("concat.txt").to_string_lossy().to_string();
    let list_content: String = part_paths.iter()
        .map(|p| format!("file '{}'", p))
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(&list_path, &list_content)
        .map_err(|e| format!("Failed to write concat list: {}", e))?;

    // Step 3: Concatenate
    let _ = std::fs::remove_file(output_path);

    if as_gif {
        let output = Command::new("ffmpeg")
            .args([
                "-f", "concat", "-safe", "0", "-i", &list_path,
                "-vf", "fps=15,scale='min(640,iw)':-1:flags=lanczos,split[s0][s1];[s0]palettegen[p];[s1][p]paletteuse",
                "-y", output_path,
            ])
            .output()
            .map_err(|e| format!("ffmpeg concat gif failed: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("GIF concat failed: {}", stderr));
        }
    } else {
        let output = Command::new("ffmpeg")
            .args([
                "-f", "concat", "-safe", "0", "-i", &list_path,
                "-c", "copy",
                "-movflags", "+faststart",
                "-y", output_path,
            ])
            .output()
            .map_err(|e| format!("ffmpeg concat failed: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("Concat failed: {}", stderr));
        }
    }

    // Step 4: Clean up temp files
    for p in &part_paths {
        let _ = std::fs::remove_file(p);
    }
    let _ = std::fs::remove_file(&list_path);
    let _ = std::fs::remove_dir(&tmp_dir);

    let file_size = std::fs::metadata(output_path).map(|m| m.len()).unwrap_or(0);
    println!(
        "[editor] Multi-segment export: {} segments → {} ({:.1} MB)",
        segments.len(), output_path, file_size as f64 / 1_048_576.0
    );

    Ok(output_path.to_string())
}

pub fn export_video(
    project: &VideoEditProject,
    app: &tauri::AppHandle,
) -> Result<String, String> {
    use tauri::Emitter;

    let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
    let base = dirs::download_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let zureshot_dir = base.join("Zureshot");
    let _ = std::fs::create_dir_all(&zureshot_dir);

    let ext = match project.output_format.as_str() {
        "gif" => "gif",
        _ => "mp4",
    };
    let output_path = zureshot_dir
        .join(format!("zureshot_edited_{}.{}", timestamp, ext))
        .to_string_lossy()
        .to_string();

    // Emit progress: starting
    let _ = app.emit("editor-export-progress", ExportProgress {
        progress: 0.0,
        stage: "preparing".into(),
        output_path: None,
        error: None,
    });

    let meta = get_video_metadata(&project.source_path)?;
    let has_segments = !project.segments.is_empty();
    let needs_trim = project.trim_start > 0.1 || (meta.duration_secs - project.trim_end).abs() > 0.1;
    let has_bg_effects = !matches!(project.background, Background::Transparent)
        && (project.padding > 0.0 || project.corner_radius > 0.0);
    let has_zoom = !project.zoom_keyframes.is_empty();
    let has_cursor = project.cursor.as_ref().map(|c| c.enabled).unwrap_or(false);
    let has_effects = has_bg_effects || has_zoom || has_cursor;

    // ─── Multi-segment export (cuts) ───
    if has_segments && project.segments.len() > 1 {
        let _ = app.emit("editor-export-progress", ExportProgress {
            progress: 0.1,
            stage: "cutting".into(),
            output_path: None,
            error: None,
        });

        let result = export_multi_segment(
            &project.source_path,
            &project.segments,
            &output_path,
            project.output_format == "gif",
        )?;

        // Clean up
        cleanup_source_and_track(&project.source_path);

        let _ = app.emit("editor-export-progress", ExportProgress {
            progress: 1.0,
            stage: "done".into(),
            output_path: Some(result.clone()),
            error: None,
        });

        return Ok(result);
    }

    let result_path = if has_effects {
        // ─── Phase E2/E3: Composited export with background + zoom ───
        let _ = app.emit("editor-export-progress", ExportProgress {
            progress: 0.1,
            stage: if has_zoom { "zooming" } else { "compositing" }.into(),
            output_path: None,
            error: None,
        });

        if project.output_format == "gif" {
            export_composited_gif(
                &project.source_path,
                project.trim_start,
                project.trim_end,
                &project.background,
                project.padding,
                project.corner_radius,
                project.shadow_radius,
                project.shadow_opacity,
                &project.zoom_keyframes,
                project.cursor.as_ref(),
                &output_path,
                meta.width,
                meta.height,
                meta.fps,
            )?
        } else {
            export_composited_video(
                &project.source_path,
                project.trim_start,
                project.trim_end,
                &project.background,
                project.padding,
                project.corner_radius,
                project.shadow_radius,
                project.shadow_opacity,
                &project.zoom_keyframes,
                project.cursor.as_ref(),
                &output_path,
                meta.width,
                meta.height,
                meta.fps,
            )?
        }
    } else if needs_trim {
        // ─── Phase E1: Simple trim ───
        let _ = app.emit("editor-export-progress", ExportProgress {
            progress: 0.1,
            stage: "trimming".into(),
            output_path: None,
            error: None,
        });

        if project.output_format == "gif" {
            export_as_gif(
                &project.source_path,
                project.trim_start,
                project.trim_end,
                &output_path,
            )?
        } else {
            trim_video(
                &project.source_path,
                project.trim_start,
                project.trim_end,
                &output_path,
            )?
        }
    } else if project.output_format == "gif" {
        export_as_gif(
            &project.source_path,
            0.0,
            meta.duration_secs,
            &output_path,
        )?
    } else {
        // No trim, no effects — copy source file
        std::fs::copy(&project.source_path, &output_path)
            .map_err(|e| format!("Failed to copy file: {}", e))?;
        output_path.clone()
    };

    cleanup_source_and_track(&project.source_path);

    // Emit progress: done
    let _ = app.emit("editor-export-progress", ExportProgress {
        progress: 1.0,
        stage: "done".into(),
        output_path: Some(result_path.clone()),
        error: None,
    });

    Ok(result_path)
}

/// Export video with background compositing using VideoToolbox hardware encoding.
fn export_composited_video(
    input_path: &str,
    start_secs: f64,
    end_secs: f64,
    background: &Background,
    padding: f64,
    corner_radius: f64,
    shadow_radius: f64,
    shadow_opacity: f64,
    zoom_keyframes: &[ZoomKeyframe],
    cursor: Option<&CursorSettings>,
    output_path: &str,
    src_w: u32,
    src_h: u32,
    fps: f64,
) -> Result<String, String> {
    let duration = end_secs - start_secs;

    let (base_filter, canvas_w, canvas_h) = build_composite_filter(
        src_w, src_h, background, padding, corner_radius, shadow_radius, shadow_opacity,
    );

    // Prepend zoom crop+scale filter if keyframes exist
    let filter = prepend_zoom_to_filter(&base_filter, zoom_keyframes, src_w, src_h, start_secs, fps);

    // Append cursor overlay if enabled
    let filter = if let Some(cs) = cursor {
        append_cursor_to_filter(&filter, input_path, cs, start_secs, src_w, src_h, fps)
    } else {
        filter
    };

    println!(
        "[editor] Composited export: {}x{} → {}x{}, zoom_kf={}, cursor={}, fps={}",
        src_w, src_h, canvas_w, canvas_h, zoom_keyframes.len(),
        cursor.map(|c| c.enabled).unwrap_or(false),
        fps,
    );

    let _ = std::fs::remove_file(output_path);

    let mut args = vec![
        "-hwaccel".to_string(), "videotoolbox".to_string(),
        "-ss".to_string(), format!("{:.3}", start_secs),
        "-i".to_string(), input_path.to_string(),
        "-t".to_string(), format!("{:.3}", duration),
    ];

    if !filter.is_empty() {
        args.push("-filter_complex".to_string());
        args.push(filter);
        args.push("-map".to_string());
        args.push("[out]".to_string());
        // Explicitly map audio (if present) since -map [out] only maps video
        args.push("-map".to_string());
        args.push("0:a?".to_string());
    }

    // Use VideoToolbox HEVC hardware encoder for fast export on Apple Silicon
    args.extend_from_slice(&[
        "-c:v".to_string(), "hevc_videotoolbox".to_string(),
        "-q:v".to_string(), "60".to_string(),     // Quality (1-100, higher = better)
        "-tag:v".to_string(), "hvc1".to_string(),  // QuickTime compatible
        "-movflags".to_string(), "+faststart".to_string(),
    ]);

    // Copy audio if present
    args.extend_from_slice(&[
        "-c:a".to_string(), "aac".to_string(),
        "-b:a".to_string(), "128k".to_string(),
        "-y".to_string(),
        output_path.to_string(),
    ]);

    let output = Command::new("ffmpeg")
        .args(&args)
        .output()
        .map_err(|e| format!("ffmpeg composited export failed: {}", e))?;

    let _stderr = String::from_utf8_lossy(&output.stderr);

    if !output.status.success() {
        // Fallback: try software encoding if hardware encoding fails
        println!("[editor] Hardware encoding failed, falling back to software");
        return export_composited_video_sw(
            input_path, start_secs, end_secs, background,
            padding, corner_radius, shadow_radius, shadow_opacity,
            zoom_keyframes, cursor, output_path, src_w, src_h, fps,
        );
    }

    let file_size = std::fs::metadata(output_path).map(|m| m.len()).unwrap_or(0);
    println!(
        "[editor] Composited export complete: {} ({:.1} MB)",
        output_path, file_size as f64 / 1_048_576.0
    );

    Ok(output_path.to_string())
}

/// Software fallback for composited export (libx264).
fn export_composited_video_sw(
    input_path: &str,
    start_secs: f64,
    end_secs: f64,
    background: &Background,
    padding: f64,
    corner_radius: f64,
    shadow_radius: f64,
    shadow_opacity: f64,
    zoom_keyframes: &[ZoomKeyframe],
    cursor: Option<&CursorSettings>,
    output_path: &str,
    src_w: u32,
    src_h: u32,
    fps: f64,
) -> Result<String, String> {
    let duration = end_secs - start_secs;
    let (base_filter, _canvas_w, _canvas_h) = build_composite_filter(
        src_w, src_h, background, padding, corner_radius, shadow_radius, shadow_opacity,
    );
    let filter = prepend_zoom_to_filter(&base_filter, zoom_keyframes, src_w, src_h, start_secs, fps);
    let filter = if let Some(cs) = cursor {
        append_cursor_to_filter(&filter, input_path, cs, start_secs, src_w, src_h, fps)
    } else {
        filter
    };

    let _ = std::fs::remove_file(output_path);

    let mut args = vec![
        "-ss".to_string(), format!("{:.3}", start_secs),
        "-i".to_string(), input_path.to_string(),
        "-t".to_string(), format!("{:.3}", duration),
    ];

    if !filter.is_empty() {
        args.push("-filter_complex".to_string());
        args.push(filter);
        args.push("-map".to_string());
        args.push("[out]".to_string());
        args.push("-map".to_string());
        args.push("0:a?".to_string());
    }

    args.extend_from_slice(&[
        "-c:v".to_string(), "libx264".to_string(),
        "-preset".to_string(), "fast".to_string(),
        "-crf".to_string(), "20".to_string(),
        "-pix_fmt".to_string(), "yuv420p".to_string(),
        "-movflags".to_string(), "+faststart".to_string(),
        "-c:a".to_string(), "aac".to_string(),
        "-b:a".to_string(), "128k".to_string(),
        "-y".to_string(),
        output_path.to_string(),
    ]);

    let output = Command::new("ffmpeg")
        .args(&args)
        .output()
        .map_err(|e| format!("ffmpeg SW composited export failed: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Composited export failed (SW): {}", stderr));
    }

    Ok(output_path.to_string())
}

/// Export composited video as GIF.
fn export_composited_gif(
    input_path: &str,
    start_secs: f64,
    end_secs: f64,
    background: &Background,
    padding: f64,
    corner_radius: f64,
    shadow_radius: f64,
    shadow_opacity: f64,
    zoom_keyframes: &[ZoomKeyframe],
    cursor: Option<&CursorSettings>,
    output_path: &str,
    src_w: u32,
    src_h: u32,
    fps: f64,
) -> Result<String, String> {
    let duration = end_secs - start_secs;
    let (base_filter, canvas_w, _canvas_h) = build_composite_filter(
        src_w, src_h, background, padding, corner_radius, shadow_radius, shadow_opacity,
    );
    let filter = prepend_zoom_to_filter(&base_filter, zoom_keyframes, src_w, src_h, start_secs, fps);
    let filter = if let Some(cs) = cursor {
        append_cursor_to_filter(&filter, input_path, cs, start_secs, src_w, src_h, fps)
    } else {
        filter
    };

    // Scale down for GIF
    let gif_scale = if canvas_w > 640 { 640.0 / canvas_w as f64 } else { 1.0 };
    let gif_w = ((canvas_w as f64 * gif_scale) as u32 / 2) * 2;

    let _ = std::fs::remove_file(output_path);

    // Build composite + GIF filter chain
    let mut full_filter = filter.clone();
    if !full_filter.is_empty() {
        full_filter.push_str(&format!(
            ";[out]fps=15,scale={}:-1:flags=lanczos,split[s0][s1];[s0]palettegen[p];[s1][p]paletteuse",
            gif_w
        ));
    } else {
        full_filter = format!(
            "fps=15,scale='min(640,iw)':-1:flags=lanczos,split[s0][s1];[s0]palettegen[p];[s1][p]paletteuse"
        );
    }

    let output = Command::new("ffmpeg")
        .args([
            "-ss", &format!("{:.3}", start_secs),
            "-i", input_path,
            "-t", &format!("{:.3}", duration),
            "-filter_complex", &full_filter,
            "-y",
            output_path,
        ])
        .output()
        .map_err(|e| format!("ffmpeg composited GIF failed: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Composited GIF export error: {}", stderr));
    }

    Ok(output_path.to_string())
}

/// Export a video segment as an optimized GIF.
fn export_as_gif(
    input_path: &str,
    start_secs: f64,
    end_secs: f64,
    output_path: &str,
) -> Result<String, String> {
    let duration = end_secs - start_secs;

    // Two-pass palette-optimized GIF (same quality as existing GIF recording)
    let vf = format!(
        "fps=15,scale='min(640,iw)':-1:flags=lanczos,split[s0][s1];[s0]palettegen[p];[s1][p]paletteuse"
    );

    let _ = std::fs::remove_file(output_path);
    let output = Command::new("ffmpeg")
        .args([
            "-ss", &format!("{:.3}", start_secs),
            "-i", input_path,
            "-t", &format!("{:.3}", duration),
            "-vf", &vf,
            "-y",
            output_path,
        ])
        .output()
        .map_err(|e| format!("ffmpeg GIF export failed: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("GIF export error: {}", stderr));
    }

    Ok(output_path.to_string())
}
