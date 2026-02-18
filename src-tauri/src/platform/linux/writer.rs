//! Linux video writer — GStreamer encoding pipeline.
//!
//! Phase 2 TODO: Build a GStreamer pipeline:
//!   pipewiresrc → videoconvert → x264enc → mp4mux → filesink
//!   (optional) pulsesrc → audioconvert → faac → mp4mux
//!
//! For Phase 1, this module is a stub — recording is not yet supported on Linux.

/// Placeholder: will create a GStreamer pipeline for recording.
pub fn create_pipeline(
    _output_path: &str,
    _width: usize,
    _height: usize,
    _fps: i32,
) -> Result<(), String> {
    Err("GStreamer recording pipeline not yet implemented (Phase 2)".into())
}
