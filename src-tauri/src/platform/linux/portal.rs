//! XDG Desktop Portal ScreenCast interaction.
//!
//! Uses an embedded Python helper script to communicate with the
//! `org.freedesktop.portal.ScreenCast` D-Bus interface.
//!
//! The portal creates a PipeWire stream for screen capture and returns
//! the PipeWire node_id that GStreamer's `pipewiresrc` can connect to.
//!
//! Dependencies (Ubuntu 24.04 desktop has these by default):
//!   - python3
//!   - python3-dbus (dbus-python)
//!   - python3-gi (PyGObject + GLib mainloop)

use std::process::Command;

/// Result of a successful ScreenCast portal session.
#[derive(Debug, Clone)]
pub struct ScreencastSession {
    /// PipeWire node ID for the screen capture stream.
    pub node_id: u32,
    /// Session handle (D-Bus object path) — needed to close the session.
    pub session_handle: String,
    /// Restore token — can be used to skip the permission dialog next time.
    pub restore_token: String,
    /// Captured stream width (if reported by portal).
    pub width: Option<u32>,
    /// Captured stream height (if reported by portal).
    pub height: Option<u32>,
}

/// Embedded Python script for XDG Portal ScreenCast interaction.
///
/// This script handles the async D-Bus signal flow required by the portal:
///   CreateSession → (Response) → SelectSources → (Response) → Start → (Response)
///
/// The user sees a system permission dialog where they choose which
/// monitor/window to share. The script outputs JSON with the PipeWire node_id.
const PORTAL_HELPER_SCRIPT: &str = r#"#!/usr/bin/env python3
"""Zureshot XDG Portal ScreenCast helper.

Requests screen capture access via the XDG Desktop Portal and outputs
a JSON object with the PipeWire node_id on success.

Output (stdout): {"node_id": 42, "restore_token": "...", "session_handle": "...", "width": 1920, "height": 1080}
Error:           {"error": "description"}
"""
import json, sys, os

def main():
    try:
        import dbus
        from dbus.mainloop.glib import DBusGMainLoop
        from gi.repository import GLib
    except ImportError as e:
        json.dump({
            "error": f"Missing dependency: {e}. "
                     "Install: sudo apt install python3-dbus python3-gi gir1.2-glib-2.0"
        }, sys.stdout)
        sys.exit(1)

    DBusGMainLoop(set_as_default=True)
    bus = dbus.SessionBus()
    sender = bus.get_unique_name().replace('.', '_').lstrip(':')

    portal_obj = bus.get_object(
        'org.freedesktop.portal.Desktop',
        '/org/freedesktop/portal/desktop'
    )
    screencast = dbus.Interface(portal_obj, 'org.freedesktop.portal.ScreenCast')

    loop = GLib.MainLoop()
    result = {}
    session_handle = [None]
    counter = [0]

    def req_path(token):
        return f'/org/freedesktop/portal/desktop/request/{sender}/{token}'

    def on_create(response, results):
        if response != 0:
            result['error'] = f'CreateSession denied (response={response})'
            loop.quit()
            return
        session_handle[0] = str(results.get('session_handle', ''))

        # Step 2: SelectSources
        counter[0] += 1
        token = f'zs_select_{counter[0]}'
        path = req_path(token)
        opts = {
            'handle_token': dbus.String(token),
            'types': dbus.UInt32(1),       # 1 = Monitor
            'multiple': dbus.Boolean(False),
            'persist_mode': dbus.UInt32(2), # Persist until explicitly revoked
        }
        restore = os.environ.get('ZURESHOT_RESTORE_TOKEN', '')
        if restore:
            opts['restore_token'] = dbus.String(restore)

        bus.add_signal_receiver(
            on_select, signal_name='Response',
            dbus_interface='org.freedesktop.portal.Request',
            path=path
        )
        screencast.SelectSources(dbus.ObjectPath(session_handle[0]), opts)

    def on_select(response, results):
        if response != 0:
            result['error'] = f'SelectSources denied (response={response})'
            loop.quit()
            return

        # Step 3: Start (triggers system permission dialog)
        counter[0] += 1
        token = f'zs_start_{counter[0]}'
        path = req_path(token)
        bus.add_signal_receiver(
            on_start, signal_name='Response',
            dbus_interface='org.freedesktop.portal.Request',
            path=path
        )
        screencast.Start(
            dbus.ObjectPath(session_handle[0]), '',
            {'handle_token': dbus.String(token)}
        )

    def on_start(response, results):
        if response != 0:
            result['error'] = f'User cancelled or Start failed (response={response})'
            loop.quit()
            return

        streams = results.get('streams', [])
        if not streams:
            result['error'] = 'No streams returned by portal'
            loop.quit()
            return

        node_id = int(streams[0][0])
        props = dict(streams[0][1]) if len(streams[0]) > 1 else {}

        result['node_id'] = node_id
        result['restore_token'] = str(results.get('restore_token', ''))
        result['session_handle'] = session_handle[0]

        if 'size' in props:
            s = props['size']
            result['width'] = int(s[0])
            result['height'] = int(s[1])

        loop.quit()

    # Step 1: CreateSession
    counter[0] += 1
    token = f'zs_create_{counter[0]}'
    path = req_path(token)

    # Register signal handler BEFORE making the call (avoids race condition)
    bus.add_signal_receiver(
        on_create, signal_name='Response',
        dbus_interface='org.freedesktop.portal.Request',
        path=path
    )
    screencast.CreateSession({
        'handle_token': dbus.String(token),
        'session_handle_token': dbus.String('zureshot_session'),
    })

    # Timeout: 120 seconds (user needs time to pick a monitor and click Share)
    GLib.timeout_add_seconds(120, lambda: (
        result.setdefault('error', 'Timeout waiting for portal response'),
        loop.quit()
    ))

    loop.run()

    json.dump(result, sys.stdout)
    sys.exit(1 if 'error' in result else 0)

if __name__ == '__main__':
    main()
"#;

/// Request screen capture access via XDG Desktop Portal.
///
/// This spawns a Python helper that communicates with the ScreenCast portal.
/// The portal will show a system permission dialog where the user selects
/// which monitor/window to share.
///
/// `restore_token`: Optional token from a previous session. If valid, the
/// portal may skip the permission dialog and reuse the previous selection.
pub fn request_screencast(restore_token: Option<&str>) -> Result<ScreencastSession, String> {
    // Write helper script to temp dir
    let script_path = std::env::temp_dir().join("zureshot_portal_helper.py");
    std::fs::write(&script_path, PORTAL_HELPER_SCRIPT)
        .map_err(|e| format!("Failed to write portal helper script: {}", e))?;

    let mut cmd = Command::new("python3");
    cmd.arg(&script_path);

    if let Some(token) = restore_token {
        if !token.is_empty() {
            cmd.env("ZURESHOT_RESTORE_TOKEN", token);
        }
    }

    println!("[zureshot-linux] Requesting screen capture via XDG Portal...");
    println!("[zureshot-linux] (A system dialog may appear — select a monitor and click Share)");

    let output = cmd
        .output()
        .map_err(|e| {
            format!(
                "Failed to run portal helper: {}. \
                 Is python3 installed? (sudo apt install python3 python3-dbus python3-gi)",
                e
            )
        })?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    if !stderr.is_empty() {
        println!("[zureshot-linux] Portal helper stderr: {}", stderr.trim());
    }

    // Parse JSON response
    let json: serde_json::Value = serde_json::from_str(stdout.trim())
        .map_err(|e| {
            format!(
                "Failed to parse portal response: {}. stdout={}, stderr={}",
                e,
                stdout.trim(),
                stderr.trim()
            )
        })?;

    if let Some(error) = json.get("error").and_then(|e| e.as_str()) {
        return Err(format!("Portal error: {}", error));
    }

    let node_id = json
        .get("node_id")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| format!("Missing node_id in portal response: {}", stdout.trim()))?
        as u32;

    let restore_token = json
        .get("restore_token")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let session_handle = json
        .get("session_handle")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let width = json.get("width").and_then(|v| v.as_u64()).map(|v| v as u32);
    let height = json.get("height").and_then(|v| v.as_u64()).map(|v| v as u32);

    println!(
        "[zureshot-linux] Portal granted: node_id={}, size={:?}x{:?}, session={}",
        node_id, width, height, session_handle
    );

    // Clean up temp script
    let _ = std::fs::remove_file(&script_path);

    Ok(ScreencastSession {
        node_id,
        session_handle,
        restore_token,
        width,
        height,
    })
}

/// Close an active ScreenCast portal session.
///
/// This releases the PipeWire stream and associated resources.
pub fn close_session(session_handle: &str) {
    if session_handle.is_empty() {
        return;
    }
    println!("[zureshot-linux] Closing portal session: {}", session_handle);

    let result = Command::new("gdbus")
        .args([
            "call",
            "--session",
            "--dest",
            "org.freedesktop.portal.Desktop",
            "--object-path",
            session_handle,
            "--method",
            "org.freedesktop.portal.Session.Close",
        ])
        .output();

    match result {
        Ok(output) if output.status.success() => {
            println!("[zureshot-linux] Portal session closed");
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            println!("[zureshot-linux] Portal session close warning: {}", stderr.trim());
        }
        Err(e) => {
            println!("[zureshot-linux] Failed to close portal session: {}", e);
        }
    }
}
