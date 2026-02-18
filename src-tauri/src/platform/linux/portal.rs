//! XDG Desktop Portal ScreenCast interaction — pure Rust via `ashpd`.
//!
//! Replaces the Phase 2 python3-dbus subprocess approach with native Rust
//! D-Bus communication. This eliminates:
//!   - python3 runtime dependency
//!   - ~200ms subprocess startup overhead
//!   - Temp file I/O for the helper script
//!
//! Architecture:
//!   ashpd::Screencast → zbus D-Bus → XDG Portal → PipeWire fd + node_id
//!
//! The portal creates a PipeWire stream and returns an fd + node_id.
//! GStreamer's `pipewiresrc` connects via this fd.

use std::os::fd::OwnedFd;

use ashpd::desktop::screencast::{CursorMode, Screencast, SourceType};
use ashpd::desktop::PersistMode;

/// Result of a successful ScreenCast portal session.
///
/// Owns the PipeWire fd, the ashpd D-Bus session (via a boxed trait object),
/// and a tokio Runtime (drives the D-Bus event loop).
///
/// **Must stay alive for the entire duration of recording.**
/// When dropped, the D-Bus session closes and PipeWire stops streaming.
///
/// We use `Box<dyn SessionCloser>` to avoid the `Session<'a, Screencast<'a>>`
/// lifetime propagating into the struct (ashpd 0.10.x requires it).
pub struct ScreencastSession {
    /// PipeWire node ID for the screen capture stream.
    pub node_id: u32,
    /// PipeWire remote fd — passed to GStreamer's `pipewiresrc`.
    pub fd: OwnedFd,
    /// D-Bus session identifier (for logging / debugging).
    pub session_handle: String,
    /// Restore token — reuse to skip the permission dialog next time.
    pub restore_token: String,
    /// Captured stream width (if reported by portal).
    pub width: Option<u32>,
    /// Captured stream height (if reported by portal).
    pub height: Option<u32>,
    /// Tokio runtime driving the zbus D-Bus event loop.
    /// **Must be dropped LAST** (after _session_closer).
    _runtime: tokio::runtime::Runtime,
}

impl ScreencastSession {
    /// Explicitly close the portal session and release PipeWire resources.
    ///
    /// Called during recording finalization. The D-Bus session is already
    /// closed inside `request_screencast_async` scope; this drops the
    /// tokio runtime and PipeWire fd.
    pub fn close(self) {
        println!("[zureshot-linux] Closing portal session: {}", self.session_handle);
        // _runtime and fd drop here, shutting down the D-Bus connection and PipeWire
        println!("[zureshot-linux] Portal session closed");
    }
}

/// Request screen capture access via XDG Desktop Portal.
///
/// This uses the `ashpd` crate for pure-Rust D-Bus communication with the
/// portal. The user will see a system permission dialog where they choose
/// which monitor/window to share.
///
/// `restore_token`: Optional token from a previous session. If valid, the
/// portal may skip the permission dialog and reuse the previous selection.
///
/// Returns a `ScreencastSession` that **must be kept alive** for the
/// duration of recording (it owns the PipeWire fd and D-Bus session).
pub fn request_screencast(restore_token: Option<&str>) -> Result<ScreencastSession, String> {
    println!("[zureshot-linux] Requesting screen capture via XDG Portal (ashpd)...");
    println!("[zureshot-linux] (A system dialog may appear — select a monitor and click Share)");

    // Create a dedicated single-threaded tokio runtime for D-Bus.
    // This runtime stays alive in the ScreencastSession to keep the
    // zbus connection active (processes D-Bus heartbeat messages).
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("Failed to create async runtime: {e}"))?;

    let restore = restore_token.map(|s| s.to_string());

    // Run the async portal interaction on the runtime.
    // The async fn returns only plain data (no Session lifetime).
    let result = runtime.block_on(request_screencast_async(restore.as_deref()));

    match result {
        Ok((node_id, fd, session_handle, restore_token, width, height)) => {
            println!(
                "[zureshot-linux] Portal granted (ashpd): node_id={}, size={:?}x{:?}, session={}",
                node_id, width, height, session_handle
            );

            Ok(ScreencastSession {
                node_id,
                fd,
                session_handle,
                restore_token,
                width,
                height,
                _runtime: runtime,
            })
        }
        Err(e) => Err(e),
    }
}

/// Async implementation of the portal interaction.
///
/// Portal flow:
///   CreateSession → SelectSources (user picks monitor) → Start → OpenPipeWireRemote
///
/// The `Session` object stays alive within this async scope (keeping the
/// D-Bus session open). We extract the PipeWire fd + node_id before
/// returning — the PipeWire stream continues independently of the
/// D-Bus session once the fd is obtained.
///
/// Note: In ashpd 0.10.x, `Session<'a, Screencast<'a>>` has lifetimes
/// that make it impossible to store in a plain struct. We keep the
/// session alive by holding the tokio runtime in ScreencastSession.
async fn request_screencast_async(
    restore_token: Option<&str>,
) -> Result<
    (
        u32,              // node_id
        OwnedFd,          // PipeWire fd
        String,           // session_handle
        String,           // restore_token
        Option<u32>,      // width
        Option<u32>,      // height
    ),
    String,
> {
    // Connect to the ScreenCast portal
    let proxy = Screencast::new()
        .await
        .map_err(|e| format!("Portal connection failed: {e}. Is xdg-desktop-portal running?"))?;

    // Step 1: Create a session
    let session = proxy
        .create_session()
        .await
        .map_err(|e| format!("CreateSession failed: {e}"))?;

    // Step 2: Configure what to capture
    proxy
        .select_sources(
            &session,
            CursorMode::Embedded,        // Cursor baked into stream
            SourceType::Monitor.into(),  // Capture full monitor
            false,                       // Single source only
            restore_token,               // Reuse previous selection
            PersistMode::ExplicitlyRevoked, // Remember until user revokes
        )
        .await
        .map_err(|e| format!("SelectSources failed: {e}"))?;

    // Step 3: Start — triggers the system permission dialog
    let response = proxy
        .start(&session, None)
        .await
        .map_err(|e| format!("Start failed (user cancelled?): {e}"))?
        .response()
        .map_err(|e| format!("Start response error: {e}"))?;

    // Extract stream info
    let stream = response
        .streams()
        .first()
        .ok_or_else(|| "No screen capture streams returned by portal".to_string())?
        .to_owned();

    // Step 4: Get PipeWire fd for direct data access
    let fd = proxy
        .open_pipe_wire_remote(&session)
        .await
        .map_err(|e| format!("OpenPipeWireRemote failed: {e}"))?;

    let node_id = stream.pipe_wire_node_id();
    let size = stream.size();
    let restore_token_out = response
        .restore_token()
        .unwrap_or_default()
        .to_string();
    // session.path() is pub(crate) in ashpd 0.10.x — use Debug format instead
    let session_handle = format!("{:?}", session);

    // Note: session drops here, but the PipeWire fd keeps the stream alive.
    // The PipeWire connection is independent of the D-Bus session once
    // the fd has been obtained via open_pipe_wire_remote.

    Ok((
        node_id,
        fd,
        session_handle,
        restore_token_out,
        size.map(|(w, _)| w as u32),
        size.map(|(_, h)| h as u32),
    ))
}
