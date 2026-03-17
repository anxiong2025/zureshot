//! Native macOS camera device listing and capture via AVFoundation.
//!
//! WKWebView cannot see iPhone Continuity Camera devices via `enumerateDevices()`,
//! so we use AVFoundation directly to enumerate camera devices and capture frames.

use objc2::rc::Retained;
use objc2_av_foundation::AVCaptureDevice;
use objc2_foundation::{NSArray, NSString};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// A camera device descriptor returned to the frontend.
#[derive(Clone, serde::Serialize)]
pub struct CameraDeviceInfo {
    pub device_id: String,
    pub label: String,
    pub model_id: String,
}

/// List all video capture devices visible to AVFoundation.
pub fn list_camera_devices() -> Vec<CameraDeviceInfo> {
    unsafe {
        use objc2::ClassType;

        let video_type = NSString::from_str("vide"); // AVMediaTypeVideo

        // +[AVCaptureDevice devicesWithMediaType:]
        let devices: Option<Retained<NSArray<AVCaptureDevice>>> =
            objc2::msg_send![AVCaptureDevice::class(), devicesWithMediaType: &*video_type];

        let Some(devices) = devices else {
            return vec![];
        };

        let count = devices.count();
        let mut result = Vec::new();
        for i in 0..count {
            let device: &AVCaptureDevice = &devices.objectAtIndex(i);
            let uid = device.uniqueID().to_string();
            let name = device.localizedName().to_string();
            let model = device.modelID().to_string();
            result.push(CameraDeviceInfo {
                device_id: uid,
                label: name,
                model_id: model,
            });
        }
        result
    }
}

/// Find the ffmpeg device index for a given AVFoundation unique ID.
/// Returns the index (e.g., "0", "1") suitable for `-i` in ffmpeg.
pub fn device_index_for_uid(uid: &str) -> Option<usize> {
    let devices = list_camera_devices();
    // Filter to only non-screen-capture devices (same order as ffmpeg listing)
    let video_devices: Vec<_> = devices
        .iter()
        .filter(|d| !d.label.contains("Capture screen"))
        .collect();
    video_devices
        .iter()
        .position(|d| d.device_id == uid)
}

/// Capture state for the native camera stream.
pub struct NativeCameraState {
    pub running: Arc<AtomicBool>,
}

impl Default for NativeCameraState {
    fn default() -> Self {
        Self {
            running: Arc::new(AtomicBool::new(false)),
        }
    }
}

/// Start capturing camera frames using ffmpeg and emit them as base64 JPEG via events.
/// This runs in a separate thread. Returns immediately.
pub fn start_native_camera_stream(
    app: &tauri::AppHandle,
    device_id: &str,
    state: &NativeCameraState,
) -> Result<(), String> {
    use base64::Engine;
    use tauri::Emitter;

    if state.running.load(Ordering::SeqCst) {
        return Ok(()); // Already running
    }

    // Find ffmpeg device index
    let idx = device_index_for_uid(device_id)
        .ok_or_else(|| format!("Camera device not found: {}", device_id))?;

    state.running.store(true, Ordering::SeqCst);
    let running = state.running.clone();
    let app = app.clone();

    std::thread::spawn(move || {
        println!("[camera-native] Starting ffmpeg capture on device index {}", idx);

        // Use ffmpeg to capture JPEG frames from the camera
        // Output one JPEG per frame to stdout at 15fps, 320x320 resolution
        let mut child = match Command::new("ffmpeg")
            .args([
                "-f", "avfoundation",
                "-framerate", "15",
                "-video_size", "320x320",
                "-i", &format!("{}:none", idx),
                "-vf", "fps=15,scale=320:320:force_original_aspect_ratio=increase,crop=320:320",
                "-f", "image2pipe",
                "-vcodec", "mjpeg",
                "-q:v", "5",
                "-",
            ])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .stdin(std::process::Stdio::null())
            .spawn()
        {
            Ok(child) => child,
            Err(e) => {
                println!("[camera-native] Failed to start ffmpeg: {}", e);
                running.store(false, Ordering::SeqCst);
                return;
            }
        };

        let stdout = child.stdout.take().unwrap();
        let mut reader = std::io::BufReader::new(stdout);

        // JPEG frames in mjpeg stream: each frame starts with FF D8 and ends with FF D9
        let mut buf = Vec::with_capacity(64 * 1024);
        let mut byte = [0u8; 1];
        use std::io::Read;

        while running.load(Ordering::SeqCst) {
            match reader.read(&mut byte) {
                Ok(0) => break, // EOF
                Ok(_) => {
                    buf.push(byte[0]);
                    // Check for JPEG end marker (FF D9)
                    if buf.len() >= 2 && buf[buf.len() - 2] == 0xFF && buf[buf.len() - 1] == 0xD9 {
                        // Found complete JPEG frame
                        // Find the start marker (FF D8)
                        if let Some(start) = buf.windows(2).position(|w| w[0] == 0xFF && w[1] == 0xD8) {
                            let jpeg_data = &buf[start..];
                            let b64 = base64::engine::general_purpose::STANDARD.encode(jpeg_data);
                            let data_url = format!("data:image/jpeg;base64,{}", b64);
                            let _ = app.emit("camera-native-frame", &data_url);
                        }
                        buf.clear();
                    }
                    // Prevent buffer from growing too large
                    if buf.len() > 512 * 1024 {
                        buf.clear();
                    }
                }
                Err(_) => break,
            }
        }

        // Kill ffmpeg
        let _ = child.kill();
        let _ = child.wait();
        running.store(false, Ordering::SeqCst);
        println!("[camera-native] Capture stopped");
    });

    Ok(())
}

/// Stop the native camera stream.
pub fn stop_native_camera_stream(state: &NativeCameraState) {
    state.running.store(false, Ordering::SeqCst);
}

