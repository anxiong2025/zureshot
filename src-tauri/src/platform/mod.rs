//! Platform abstraction layer for Zureshot.
//!
//! Each platform provides:
//!   - `RecordingHandle` — owns all recording state (stream, encoder, etc.)
//!   - `start_recording()` — set up capture pipeline and begin recording
//!   - `take_screenshot_region()` — capture a screen region to PNG
//!   - System integration helpers (file reveal, clipboard, dialogs)

use serde::{Deserialize, Serialize};

// ── Common types shared across all platforms ─────────────────────────

/// Recording quality presets (used by both macOS and Linux).
#[derive(Clone, Copy, Debug, Serialize, Deserialize, Default, PartialEq)]
pub enum RecordingQuality {
    /// Standard: 30 fps. Sharp text, smooth enough for most content.
    #[default]
    Standard,
    /// High: 60 fps. Butter-smooth scrolling and animations.
    High,
}

/// Configuration passed to `start_recording()`.
pub struct StartRecordingConfig {
    pub output_path: String,
    pub region: Option<CaptureRegion>,
    pub quality: RecordingQuality,
    pub capture_system_audio: bool,
    pub capture_microphone: bool,
}

/// Region definition for region-based capture (web coordinates: top-left origin, CSS pixels).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CaptureRegion {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

// ── Platform-specific modules ────────────────────────────────────────

#[cfg(target_os = "macos")]
pub mod macos;

#[cfg(target_os = "linux")]
pub mod linux;

// Re-export the active platform as `platform::imp` so call-sites
// can write `platform::imp::start_recording(...)` etc.
#[cfg(target_os = "macos")]
pub use macos as imp;

#[cfg(target_os = "linux")]
pub use linux as imp;
