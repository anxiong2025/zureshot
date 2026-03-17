//! Scrolling long screenshot — capture a screen region repeatedly as the user
//! scrolls, detect overlap between consecutive frames, and stitch them into
//! one tall image.
//!
//! Flow:
//!   1. `ScrollCaptureSession::new(x,y,w,h)` — capture first frame
//!   2. `session.capture_frame()` — called periodically (~300ms), returns true if new content
//!   3. `session.finish(path)` — write stitched PNG, return dimensions + file size

use std::ffi::c_void;
use objc2_core_foundation::{CGPoint, CGRect, CGSize};

// ── Raw frame from CGWindowListCreateImage ───────────────────────────

struct RawFrame {
    pixels: Vec<u8>,
    width: usize,
    height: usize,
    bytes_per_row: usize,
    bitmap_info: u32,
}

// ── CoreGraphics FFI ─────────────────────────────────────────────────

extern "C" {
    fn CGWindowListCreateImage(
        screenBounds: CGRect,
        listOption: u32,
        windowID: u32,
        imageOption: u32,
    ) -> *const c_void;

    fn CGImageGetWidth(image: *const c_void) -> usize;
    fn CGImageGetHeight(image: *const c_void) -> usize;
    fn CGImageGetBytesPerRow(image: *const c_void) -> usize;
    fn CGImageGetBitmapInfo(image: *const c_void) -> u32;
    fn CGImageGetDataProvider(image: *const c_void) -> *const c_void;
    fn CGImageRelease(image: *const c_void);

    fn CGDataProviderCopyData(provider: *const c_void) -> *const c_void;
    fn CGDataProviderCreateWithData(
        info: *const c_void,
        data: *const c_void,
        size: usize,
        release_callback: *const c_void,
    ) -> *const c_void;
    fn CGDataProviderRelease(provider: *const c_void);

    fn CGImageCreate(
        width: usize,
        height: usize,
        bits_per_component: usize,
        bits_per_pixel: usize,
        bytes_per_row: usize,
        space: *const c_void,
        bitmap_info: u32,
        provider: *const c_void,
        decode: *const c_void,
        should_interpolate: bool,
        intent: u32,
    ) -> *const c_void;

    fn CGColorSpaceCreateDeviceRGB() -> *const c_void;
    fn CGColorSpaceRelease(cs: *const c_void);

    fn CGImageDestinationCreateWithURL(
        url: *const c_void,
        image_type: *const c_void,
        count: usize,
        options: *const c_void,
    ) -> *const c_void;
    fn CGImageDestinationAddImage(
        dest: *const c_void,
        image: *const c_void,
        properties: *const c_void,
    );
    fn CGImageDestinationFinalize(dest: *const c_void) -> bool;

    fn CFDataGetBytePtr(data: *const c_void) -> *const u8;
    fn CFDataGetLength(data: *const c_void) -> isize;
    fn CFRelease(cf: *const c_void);

    fn CFStringCreateWithBytes(
        alloc: *const c_void,
        bytes: *const u8,
        num_bytes: isize,
        encoding: u32,
        is_external: bool,
    ) -> *const c_void;

    fn CFURLCreateWithFileSystemPath(
        alloc: *const c_void,
        file_path: *const c_void,
        path_style: isize,
        is_directory: bool,
    ) -> *const c_void;
}

fn cfstring(s: &str) -> *const c_void {
    unsafe {
        CFStringCreateWithBytes(
            std::ptr::null(),
            s.as_ptr(),
            s.len() as isize,
            0x08000100, // kCFStringEncodingUTF8
            false,
        )
    }
}

fn cfurl_from_path(path: &str) -> *const c_void {
    let cf_path = cfstring(path);
    let url = unsafe { CFURLCreateWithFileSystemPath(std::ptr::null(), cf_path, 0, false) };
    unsafe { CFRelease(cf_path); }
    url
}

// ── Capture a screen region as raw pixels ────────────────────────────

fn capture_region_pixels(x: f64, y: f64, w: f64, h: f64) -> Result<RawFrame, String> {
    let rect = CGRect::new(CGPoint::new(x, y), CGSize::new(w, h));

    // kCGWindowListOptionOnScreenOnly = 1, kCGWindowImageDefault = 0
    let image = unsafe { CGWindowListCreateImage(rect, 1, 0, 0) };
    if image.is_null() {
        return Err("CGWindowListCreateImage returned null".into());
    }

    let width = unsafe { CGImageGetWidth(image) };
    let height = unsafe { CGImageGetHeight(image) };
    let bytes_per_row = unsafe { CGImageGetBytesPerRow(image) };
    let bitmap_info = unsafe { CGImageGetBitmapInfo(image) };

    let provider = unsafe { CGImageGetDataProvider(image) };
    if provider.is_null() {
        unsafe { CGImageRelease(image); }
        return Err("CGImageGetDataProvider returned null".into());
    }

    let data = unsafe { CGDataProviderCopyData(provider) };
    if data.is_null() {
        unsafe { CGImageRelease(image); }
        return Err("CGDataProviderCopyData returned null".into());
    }

    let ptr = unsafe { CFDataGetBytePtr(data) };
    let len = unsafe { CFDataGetLength(data) } as usize;
    let pixels = unsafe { std::slice::from_raw_parts(ptr, len) }.to_vec();

    unsafe {
        CFRelease(data);
        CGImageRelease(image);
    }

    Ok(RawFrame { pixels, width, height, bytes_per_row, bitmap_info })
}

// ── Overlap detection via SAD (Sum of Absolute Differences) ──────────
//
// Takes the bottom `strip_height` rows of the accumulated image and slides
// them across the top portion of the new frame. The position with the
// minimum SAD is the overlap offset.

fn find_overlap(
    acc: &[u8],
    acc_height: usize,
    acc_bpr: usize,
    new_pixels: &[u8],
    new_height: usize,
    new_bpr: usize,
    compare_width_bytes: usize,
) -> usize {
    let strip_height = 40.min(acc_height / 2).min(new_height / 2);
    if strip_height < 4 {
        return 0;
    }

    let strip_start = (acc_height - strip_height) * acc_bpr;
    let strip = &acc[strip_start..];

    let max_search = new_height.saturating_sub(strip_height);

    let mut best_pos = 0usize;
    let mut best_sad = u64::MAX;

    // Step by 4 pixels (16 bytes) for speed; compare every row
    let step = 16.min(compare_width_bytes);

    for pos in 0..=max_search {
        let mut sad: u64 = 0;
        let mut bailed = false;

        for row in 0..strip_height {
            let s_off = row * acc_bpr;
            let n_off = (pos + row) * new_bpr;

            let mut col = 0;
            while col < compare_width_bytes {
                let a = strip[s_off + col] as i64;
                let b = new_pixels[n_off + col] as i64;
                sad += (a - b).unsigned_abs();
                col += step;
            }

            if sad > best_sad {
                bailed = true;
                break;
            }
        }

        if !bailed && sad < best_sad {
            best_sad = sad;
            best_pos = pos;
        }
    }

    // If no good match, return 0 (append entire frame)
    let samples = strip_height * (compare_width_bytes / step + 1);
    let threshold = samples as u64 * 12;
    if best_sad > threshold {
        0
    } else {
        best_pos + strip_height
    }
}

// ── Save raw pixel buffer as PNG via ImageIO ─────────────────────────

fn save_pixels_as_png(
    pixels: &[u8],
    width: usize,
    height: usize,
    bytes_per_row: usize,
    bitmap_info: u32,
    output_path: &str,
) -> Result<(usize, usize, u64), String> {
    unsafe {
        let colorspace = CGColorSpaceCreateDeviceRGB();

        let provider = CGDataProviderCreateWithData(
            std::ptr::null(),
            pixels.as_ptr() as *const c_void,
            pixels.len(),
            std::ptr::null(), // no release callback — we own the buffer
        );

        let image = CGImageCreate(
            width,
            height,
            8,               // bitsPerComponent
            32,              // bitsPerPixel
            bytes_per_row,
            colorspace,
            bitmap_info,
            provider,
            std::ptr::null(),
            false,
            0, // kCGRenderingIntentDefault
        );

        if image.is_null() {
            CGDataProviderRelease(provider);
            CGColorSpaceRelease(colorspace);
            return Err("CGImageCreate failed for stitched image".into());
        }

        let url = cfurl_from_path(output_path);
        let png_type = cfstring("public.png");
        let dest = CGImageDestinationCreateWithURL(url, png_type, 1, std::ptr::null());

        let ok = if !dest.is_null() {
            CGImageDestinationAddImage(dest, image, std::ptr::null());
            let finalized = CGImageDestinationFinalize(dest);
            CFRelease(dest);
            finalized
        } else {
            false
        };

        CFRelease(url);
        CFRelease(png_type);
        CGImageRelease(image);
        CGDataProviderRelease(provider);
        CGColorSpaceRelease(colorspace);

        if !ok {
            return Err("Failed to write stitched PNG".into());
        }
    }

    let file_size = std::fs::metadata(output_path).map(|m| m.len()).unwrap_or(0);
    Ok((width, height, file_size))
}

// ── Public session API ───────────────────────────────────────────────

pub struct ScrollCaptureSession {
    region: (f64, f64, f64, f64),
    accumulated: Vec<u8>,
    acc_width: usize,
    acc_height: usize,
    bytes_per_row: usize,
    bitmap_info: u32,
    frame_count: usize,
}

impl ScrollCaptureSession {
    /// Start a new session by capturing the first frame of the given region.
    pub fn new(x: f64, y: f64, w: f64, h: f64) -> Result<Self, String> {
        let frame = capture_region_pixels(x, y, w, h)?;
        println!(
            "[scroll-capture] First frame: {}x{} (bpr={}, bitmap_info=0x{:x})",
            frame.width, frame.height, frame.bytes_per_row, frame.bitmap_info
        );

        Ok(Self {
            region: (x, y, w, h),
            acc_width: frame.width,
            acc_height: frame.height,
            bytes_per_row: frame.bytes_per_row,
            bitmap_info: frame.bitmap_info,
            accumulated: frame.pixels,
            frame_count: 1,
        })
    }

    /// Capture the current screen region and stitch if new content is detected.
    /// Returns `true` if new rows were appended.
    pub fn capture_frame(&mut self) -> Result<bool, String> {
        let (x, y, w, h) = self.region;
        let frame = capture_region_pixels(x, y, w, h)?;

        if frame.width != self.acc_width {
            return Err(format!(
                "Frame width changed: expected {}, got {}",
                self.acc_width, frame.width
            ));
        }

        let compare_bytes = self.acc_width * 4; // 4 bytes per pixel

        let overlap = find_overlap(
            &self.accumulated,
            self.acc_height,
            self.bytes_per_row,
            &frame.pixels,
            frame.height,
            frame.bytes_per_row,
            compare_bytes,
        );

        // If overlap covers (almost) the entire new frame, nothing changed
        if overlap >= frame.height.saturating_sub(2) {
            return Ok(false);
        }

        let new_rows = frame.height - overlap;
        let start_byte = overlap * frame.bytes_per_row;
        self.accumulated.extend_from_slice(&frame.pixels[start_byte..]);
        self.acc_height += new_rows;
        self.frame_count += 1;

        println!(
            "[scroll-capture] Frame #{}: overlap={}px, added={}px, total={}px",
            self.frame_count, overlap, new_rows, self.acc_height
        );

        Ok(true)
    }

    pub fn frame_count(&self) -> usize {
        self.frame_count
    }

    pub fn total_height(&self) -> usize {
        self.acc_height
    }

    pub fn width(&self) -> usize {
        self.acc_width
    }

    /// Finalize the session: write the stitched image as PNG.
    pub fn finish(self, output_path: &str) -> Result<(usize, usize, u64), String> {
        println!(
            "[scroll-capture] Saving {}x{} ({} frames, {:.1} MB raw) → {}",
            self.acc_width,
            self.acc_height,
            self.frame_count,
            self.accumulated.len() as f64 / 1_048_576.0,
            output_path
        );

        save_pixels_as_png(
            &self.accumulated,
            self.acc_width,
            self.acc_height,
            self.bytes_per_row,
            self.bitmap_info,
            output_path,
        )
    }
}
