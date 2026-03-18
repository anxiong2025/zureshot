//! Linux screen capture — XDG Desktop Portal (primary) + external tool fallbacks.
//!
//! Priority:
//!   1. ashpd Screenshot portal — pure Rust, works on GNOME/KDE/sway Wayland & X11
//!   2. grim — Wayland-native (wlroots/sway), no portal needed
//!   3. gnome-screenshot + image-crate crop — GNOME fallback
//!   4. ImageMagick import — X11 fallback

use image::GenericImageView;

/// Take a screenshot of a specific screen region.
pub fn take_screenshot_region(
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    output_path: &str,
) -> Result<(usize, usize, u64), String> {
    // 1. XDG Portal Screenshot (preferred — no external tools)
    if let Ok(result) = take_via_portal(x, y, width, height, output_path) {
        return Ok(result);
    }

    // 2. grim (Wayland / wlroots)
    if let Ok(result) = take_via_grim(x, y, width, height, output_path) {
        return Ok(result);
    }

    // 3. gnome-screenshot + crop
    if let Ok(result) = take_via_gnome_screenshot(x, y, width, height, output_path) {
        return Ok(result);
    }

    // 4. ImageMagick import (X11)
    take_via_imagemagick(x, y, width, height, output_path)
}

// ── 1. XDG Portal ────────────────────────────────────────────────────

fn take_via_portal(
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    output_path: &str,
) -> Result<(usize, usize, u64), String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("tokio runtime: {e}"))?;

    let uri: String = runtime.block_on(async {
        use ashpd::desktop::screenshot::Screenshot;
        let proxy = Screenshot::new()
            .await
            .map_err(|e| format!("Screenshot portal unavailable: {e}"))?;
        let response = proxy
            .screenshot(None, false)
            .await
            .map_err(|e| format!("Screenshot portal call failed: {e}"))?
            .response()
            .map_err(|e| format!("Screenshot portal response error: {e}"))?;
        Ok::<String, String>(response.uri().to_string())
    })?;

    // URI is file:///tmp/screenshot-XXXXX.png
    let src_path = uri
        .strip_prefix("file://")
        .unwrap_or(uri.as_str())
        .to_string();

    let result = crop_and_save(&src_path, output_path, x, y, width, height);

    // Clean up the portal's temp file
    let _ = std::fs::remove_file(&src_path);

    result
}

// ── 2. grim ──────────────────────────────────────────────────────────

fn take_via_grim(
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    output_path: &str,
) -> Result<(usize, usize, u64), String> {
    let output = std::process::Command::new("grim")
        .args([
            "-g",
            &format!("{},{} {}x{}", x as i32, y as i32, width as i32, height as i32),
            output_path,
        ])
        .output()
        .map_err(|e| format!("grim not found: {e}"))?;

    if !output.status.success() {
        return Err(format!(
            "grim failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    png_dimensions(output_path)
}

// ── 3. gnome-screenshot ──────────────────────────────────────────────

fn take_via_gnome_screenshot(
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    output_path: &str,
) -> Result<(usize, usize, u64), String> {
    let full_path = format!("{}.full.png", output_path);

    let output = std::process::Command::new("gnome-screenshot")
        .args(["-f", &full_path])
        .output()
        .map_err(|e| format!("gnome-screenshot not found: {e}"))?;

    if !output.status.success() {
        let _ = std::fs::remove_file(&full_path);
        return Err(format!(
            "gnome-screenshot failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let result = crop_and_save(&full_path, output_path, x, y, width, height);
    let _ = std::fs::remove_file(&full_path);
    result
}

// ── 4. ImageMagick ───────────────────────────────────────────────────

fn take_via_imagemagick(
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    output_path: &str,
) -> Result<(usize, usize, u64), String> {
    let geometry = format!(
        "{}x{}+{}+{}",
        width as i32, height as i32, x as i32, y as i32
    );

    let output = std::process::Command::new("import")
        .args(["-window", "root", "-crop", &geometry, output_path])
        .output()
        .map_err(|_| {
            "Screenshot failed: install grim (Wayland) or imagemagick (X11): \
             sudo apt install grim  OR  sudo apt install imagemagick"
                .to_string()
        })?;

    if !output.status.success() {
        return Err(format!(
            "import failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    png_dimensions(output_path)
}

// ── Helpers ──────────────────────────────────────────────────────────

/// Crop a full-screen PNG to the requested region using the `image` crate.
fn crop_and_save(
    src: &str,
    dst: &str,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<(usize, usize, u64), String> {
    let img = image::open(src).map_err(|e| format!("Failed to open screenshot: {e}"))?;

    let img_w = img.width();
    let img_h = img.height();

    // Clamp region to image bounds
    let cx = (x as u32).min(img_w.saturating_sub(1));
    let cy = (y as u32).min(img_h.saturating_sub(1));
    let cw = (width as u32).min(img_w.saturating_sub(cx));
    let ch = (height as u32).min(img_h.saturating_sub(cy));

    if cw == 0 || ch == 0 {
        // Degenerate region — return full screenshot
        img.save(dst).map_err(|e| format!("Failed to save screenshot: {e}"))?;
        let file_size = std::fs::metadata(dst).map(|m| m.len()).unwrap_or(0);
        return Ok((img_w as usize, img_h as usize, file_size));
    }

    let cropped = img.crop_imm(cx, cy, cw, ch);
    cropped
        .save_with_format(dst, image::ImageFormat::Png)
        .map_err(|e| format!("Failed to save cropped screenshot: {e}"))?;

    let file_size = std::fs::metadata(dst).map(|m| m.len()).unwrap_or(0);
    Ok((cropped.width() as usize, cropped.height() as usize, file_size))
}

/// Read PNG dimensions by parsing the file header (no external deps).
fn png_dimensions(path: &str) -> Result<(usize, usize, u64), String> {
    let file_size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);

    if let Ok(data) = std::fs::read(path) {
        if data.len() >= 24 && &data[0..8] == b"\x89PNG\r\n\x1a\n" {
            let w = u32::from_be_bytes([data[16], data[17], data[18], data[19]]) as usize;
            let h = u32::from_be_bytes([data[20], data[21], data[22], data[23]]) as usize;
            return Ok((w, h, file_size));
        }
    }

    Ok((0, 0, file_size))
}
