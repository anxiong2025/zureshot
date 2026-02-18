//! Linux screen capture — XDG Desktop Portal.
//!
//! Uses the `org.freedesktop.portal.Screenshot` D-Bus interface
//! which works on both Wayland and X11 (via xdg-desktop-portal-gnome).
//!
//! Phase 1: Screenshot via Portal
//! Phase 2: ScreenCast via Portal + PipeWire for recording

/// Take a screenshot of a specific screen region.
///
/// On Linux, we use the XDG Desktop Portal Screenshot interface which
/// captures the entire screen, then we crop to the requested region
/// using the `image` crate.
///
/// The Portal may show a system permission dialog the first time.
pub fn take_screenshot_region(
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    output_path: &str,
) -> Result<(usize, usize, u64), String> {
    // Strategy: use `gnome-screenshot` or `grim` to capture the screen,
    // then crop to the requested region.
    //
    // For MVP, we'll try multiple tools in order of preference:
    // 1. grim (Wayland-native, sway/wlroots)
    // 2. gnome-screenshot (GNOME)
    // 3. import (ImageMagick, X11 fallback)

    // Try grim first (Wayland)
    let grim_result = std::process::Command::new("grim")
        .args([
            "-g",
            &format!("{},{} {}x{}", x as i32, y as i32, width as i32, height as i32),
            output_path,
        ])
        .output();

    match grim_result {
        Ok(output) if output.status.success() => {
            return file_dimensions(output_path);
        }
        _ => {}
    }

    // Try gnome-screenshot (full screen capture, then crop)
    let temp_full = format!("{}.full.png", output_path);
    let gnome_result = std::process::Command::new("gnome-screenshot")
        .args(["-f", &temp_full])
        .output();

    match gnome_result {
        Ok(output) if output.status.success() => {
            // Crop using convert (ImageMagick) or custom code
            let crop_result = crop_image(&temp_full, output_path, x, y, width, height);
            let _ = std::fs::remove_file(&temp_full);
            return crop_result;
        }
        _ => {
            let _ = std::fs::remove_file(&temp_full);
        }
    }

    // Try import (ImageMagick, X11)
    let import_result = std::process::Command::new("import")
        .args([
            "-window",
            "root",
            "-crop",
            &format!(
                "{}x{}+{}+{}",
                width as i32, height as i32, x as i32, y as i32
            ),
            output_path,
        ])
        .output();

    match import_result {
        Ok(output) if output.status.success() => {
            return file_dimensions(output_path);
        }
        _ => {}
    }

    Err(
        "Screenshot failed: none of grim, gnome-screenshot, or import (ImageMagick) are available. \
         Install one of them: sudo apt install grim (Wayland) or sudo apt install imagemagick (X11)"
            .into(),
    )
}

/// Crop an image using ImageMagick's `convert` command.
fn crop_image(
    input: &str,
    output: &str,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<(usize, usize, u64), String> {
    let geometry = format!(
        "{}x{}+{}+{}",
        width as i32, height as i32, x as i32, y as i32
    );

    let result = std::process::Command::new("convert")
        .args([input, "-crop", &geometry, "+repage", output])
        .output()
        .map_err(|e| format!("ImageMagick convert failed: {}", e))?;

    if !result.status.success() {
        return Err(format!(
            "Image crop failed: {}",
            String::from_utf8_lossy(&result.stderr)
        ));
    }

    file_dimensions(output)
}

/// Read image dimensions and file size from a PNG file.
fn file_dimensions(path: &str) -> Result<(usize, usize, u64), String> {
    let file_size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);

    // Use `identify` (ImageMagick) to get dimensions, or parse PNG header
    let identify_result = std::process::Command::new("identify")
        .args(["-format", "%w %h", path])
        .output();

    match identify_result {
        Ok(output) if output.status.success() => {
            let dims = String::from_utf8_lossy(&output.stdout);
            let parts: Vec<&str> = dims.trim().split_whitespace().collect();
            if parts.len() >= 2 {
                let w: usize = parts[0].parse().unwrap_or(0);
                let h: usize = parts[1].parse().unwrap_or(0);
                return Ok((w, h, file_size));
            }
        }
        _ => {}
    }

    // Fallback: parse PNG header (bytes 16-23 contain width and height as big-endian u32)
    if let Ok(data) = std::fs::read(path) {
        if data.len() >= 24 && &data[0..8] == b"\x89PNG\r\n\x1a\n" {
            let w = u32::from_be_bytes([data[16], data[17], data[18], data[19]]) as usize;
            let h = u32::from_be_bytes([data[20], data[21], data[22], data[23]]) as usize;
            return Ok((w, h, file_size));
        }
    }

    // Last resort: return 0x0 dimensions
    Ok((0, 0, file_size))
}
