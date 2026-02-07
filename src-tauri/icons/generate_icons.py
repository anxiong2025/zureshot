#!/usr/bin/env python3
"""
Generate Apple-style app icons for Zureshot.

Design: macOS-style squircle with a screen-recording themed icon.
- Deep indigo-to-blue gradient background
- Stylised monitor / screen with a red recording dot
- Viewfinder corner brackets
- Subtle inner highlight and bottom shadow for depth

Outputs:
  32x32.png, 128x128.png, 128x128@2x.png  — Tauri app icon
  icon.icns  — macOS bundle icon
  icon.ico   — Windows bundle icon (multi-size)
  tray.png   — 22×22 menu-bar tray icon (template-style)
"""

import math
import struct
import os
import io

from PIL import Image, ImageDraw, ImageFilter

ICON_DIR = os.path.dirname(os.path.abspath(__file__))

# ─────────────────── Color palette ───────────────────
BG_TOP = (25, 30, 85)         # deep navy
BG_BOTTOM = (45, 90, 210)     # vivid blue
REC_DOT = (255, 59, 48)       # Apple-red
FG_WHITE = (255, 255, 255)


# ─────────────────── Apple squircle (superellipse) ───────────────────

def superellipse_mask(size, n=5.0):
    """Create a superellipse (squircle) alpha mask — Apple's icon shape."""
    img = Image.new("L", (size, size), 0)
    cx, cy = size / 2, size / 2
    r = size / 2 - 1

    for y in range(size):
        for x in range(size):
            nx = (x - cx) / r
            ny = (y - cy) / r
            val = abs(nx) ** n + abs(ny) ** n
            if val <= 1.0:
                edge_dist = 1.0 - val
                alpha = min(255, int(edge_dist * r * 2.5))
                alpha = max(0, min(255, alpha))
                img.putpixel((x, y), alpha)
    return img


def draw_gradient_bg(size):
    """Create a vertical gradient from BG_TOP to BG_BOTTOM."""
    img = Image.new("RGB", (size, size))
    for y in range(size):
        t = y / max(1, size - 1)
        r = int(BG_TOP[0] + (BG_BOTTOM[0] - BG_TOP[0]) * t)
        g = int(BG_TOP[1] + (BG_BOTTOM[1] - BG_TOP[1]) * t)
        b = int(BG_TOP[2] + (BG_BOTTOM[2] - BG_TOP[2]) * t)
        for x in range(size):
            img.putpixel((x, y), (r, g, b))
    return img


def draw_icon(size):
    """Render the full icon at the given size."""
    # ── Background: gradient + squircle mask ──
    bg = draw_gradient_bg(size)
    mask = superellipse_mask(size, n=5.0)
    icon = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    icon.paste(bg, (0, 0), mask)

    # ── Top highlight for gloss / depth ──
    highlight = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    for y in range(int(size * 0.15)):
        alpha = int(45 * (1 - y / (size * 0.15)))
        for x in range(size):
            if mask.getpixel((x, y)) > 128:
                highlight.putpixel((x, y), (255, 255, 255, alpha))
    icon = Image.alpha_composite(icon, highlight)

    draw = ImageDraw.Draw(icon)
    cx, cy = size / 2, size / 2

    # ── Monitor / screen shape ──
    screen_w = size * 0.52
    screen_h = size * 0.36
    screen_left = cx - screen_w / 2
    screen_top = cy - screen_h / 2 - size * 0.05
    screen_right = cx + screen_w / 2
    screen_bottom = cy + screen_h / 2 - size * 0.05
    screen_radius = size * 0.035

    bezel_w = max(1, int(size * 0.022))
    draw.rounded_rectangle(
        [screen_left, screen_top, screen_right, screen_bottom],
        radius=int(screen_radius),
        outline=FG_WHITE,
        width=bezel_w,
    )

    # ── Monitor stand ──
    neck_w = max(1, int(size * 0.02))
    neck_top = screen_bottom
    neck_bottom = screen_bottom + size * 0.07
    draw.line(
        [(cx, neck_top), (cx, neck_bottom)],
        fill=FG_WHITE, width=neck_w,
    )

    base_w = size * 0.22
    base_h = max(1, int(size * 0.018))
    draw.rounded_rectangle(
        [cx - base_w / 2, neck_bottom, cx + base_w / 2, neck_bottom + base_h],
        radius=max(1, int(base_h / 2)),
        fill=FG_WHITE,
    )

    # ── Red recording dot (inside the screen) ──
    dot_r = size * 0.065
    dot_cx = cx
    dot_cy = (screen_top + screen_bottom) / 2

    # Glow
    glow_r = dot_r * 2.2
    glow_img = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    glow_draw = ImageDraw.Draw(glow_img)
    glow_draw.ellipse(
        [dot_cx - glow_r, dot_cy - glow_r, dot_cx + glow_r, dot_cy + glow_r],
        fill=(255, 59, 48, 45),
    )
    glow_img = glow_img.filter(ImageFilter.GaussianBlur(radius=max(1, int(size * 0.04))))
    icon = Image.alpha_composite(icon, glow_img)
    draw = ImageDraw.Draw(icon)

    # The dot
    draw.ellipse(
        [dot_cx - dot_r, dot_cy - dot_r, dot_cx + dot_r, dot_cy + dot_r],
        fill=REC_DOT,
    )

    # Specular highlight on the dot
    spec_r = dot_r * 0.35
    spec_cx = dot_cx - dot_r * 0.22
    spec_cy = dot_cy - dot_r * 0.22
    spec_img = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    spec_draw = ImageDraw.Draw(spec_img)
    spec_draw.ellipse(
        [spec_cx - spec_r, spec_cy - spec_r, spec_cx + spec_r, spec_cy + spec_r],
        fill=(255, 255, 255, 90),
    )
    spec_img = spec_img.filter(ImageFilter.GaussianBlur(radius=max(1, int(size * 0.012))))
    icon = Image.alpha_composite(icon, spec_img)
    draw = ImageDraw.Draw(icon)

    # ── Viewfinder corner brackets ──
    corner_len = size * 0.07
    corner_thick = max(1, int(size * 0.018))
    ci = size * 0.035  # corner inset from screen edges

    vf_left = screen_left - ci
    vf_top = screen_top - ci
    vf_right = screen_right + ci
    vf_bottom = screen_bottom + ci * 1.8

    bracket_color = (255, 255, 255, 130)
    corners = [
        # Top-left
        ((vf_left, vf_top, vf_left + corner_len, vf_top + corner_thick),
         (vf_left, vf_top, vf_left + corner_thick, vf_top + corner_len)),
        # Top-right
        ((vf_right - corner_len, vf_top, vf_right, vf_top + corner_thick),
         (vf_right - corner_thick, vf_top, vf_right, vf_top + corner_len)),
        # Bottom-left
        ((vf_left, vf_bottom - corner_thick, vf_left + corner_len, vf_bottom),
         (vf_left, vf_bottom - corner_len, vf_left + corner_thick, vf_bottom)),
        # Bottom-right
        ((vf_right - corner_len, vf_bottom - corner_thick, vf_right, vf_bottom),
         (vf_right - corner_thick, vf_bottom - corner_len, vf_right, vf_bottom)),
    ]
    for h_rect, v_rect in corners:
        draw.rectangle(h_rect, fill=bracket_color)
        draw.rectangle(v_rect, fill=bracket_color)

    # ── Bottom inner shadow for depth ──
    shadow_layer = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    for y in range(int(size * 0.82), size):
        t = (y - size * 0.82) / (size * 0.18)
        alpha = int(35 * t)
        for x in range(size):
            if mask.getpixel((x, y)) > 128:
                shadow_layer.putpixel((x, y), (0, 0, 0, alpha))
    icon = Image.alpha_composite(icon, shadow_layer)

    return icon


def draw_tray_icon(size=22):
    """macOS menu-bar tray icon — template style (dark on transparent)."""
    # Render at 2x for quality, then downscale
    s = size * 2
    img = Image.new("RGBA", (s, s), (0, 0, 0, 0))
    draw = ImageDraw.Draw(img)
    cx, cy = s / 2, s / 2

    # Small monitor outline
    sw = s * 0.6
    sh = s * 0.4
    sl = cx - sw / 2
    st = cy - sh / 2 - s * 0.04
    sr = cx + sw / 2
    sb = cy + sh / 2 - s * 0.04
    lw = max(1, int(s * 0.07))
    rr = max(1, int(s * 0.06))

    draw.rounded_rectangle([sl, st, sr, sb], radius=rr, outline=(0, 0, 0, 220), width=lw)

    # Stand
    neck_w = max(1, int(s * 0.07))
    draw.line([(cx, sb), (cx, sb + s * 0.08)], fill=(0, 0, 0, 220), width=neck_w)
    base = s * 0.18
    draw.line(
        [(cx - base, sb + s * 0.08), (cx + base, sb + s * 0.08)],
        fill=(0, 0, 0, 220), width=neck_w,
    )

    # Red dot inside
    dot_r = max(2, s * 0.1)
    dot_cy = (st + sb) / 2
    draw.ellipse(
        [cx - dot_r, dot_cy - dot_r, cx + dot_r, dot_cy + dot_r],
        fill=REC_DOT,
    )

    # Downscale
    img = img.resize((size, size), Image.Resampling.LANCZOS)
    return img


# ─────────────────── .icns builder ───────────────────

def build_icns(images_dict):
    """Build a macOS .icns file from {ostype: png_bytes}."""
    entries = b""
    for ostype, data in images_dict.items():
        entry_size = 8 + len(data)
        entries += ostype.encode("ascii") + struct.pack(">I", entry_size) + data
    total = 8 + len(entries)
    return b"icns" + struct.pack(">I", total) + entries


def build_ico(images):
    """Build a .ico file from a list of PIL RGBA images."""
    entries = []
    data_offset = 6 + 16 * len(images)
    image_data_list = []

    for img in images:
        w, h = img.size
        buf = io.BytesIO()
        img.save(buf, format="PNG")
        png_data = buf.getvalue()
        image_data_list.append(png_data)

        entry = struct.pack(
            "<BBBBHHII",
            w if w < 256 else 0,
            h if h < 256 else 0,
            0, 0, 1, 32,
            len(png_data),
            data_offset,
        )
        entries.append(entry)
        data_offset += len(png_data)

    header = struct.pack("<HHH", 0, 1, len(images))
    return header + b"".join(entries) + b"".join(image_data_list)


# ─────────────────── Main ───────────────────

def main():
    os.chdir(ICON_DIR)

    print("🎨 Rendering master icon at 1024×1024...")
    master = draw_icon(1024)

    # Required PNGs for Tauri
    sizes = {
        "32x32.png": 32,
        "128x128.png": 128,
        "128x128@2x.png": 256,
    }

    png_cache = {}

    for filename, sz in sizes.items():
        img = master.resize((sz, sz), Image.Resampling.LANCZOS)
        img.save(os.path.join(ICON_DIR, filename), "PNG")
        buf = io.BytesIO()
        img.save(buf, "PNG")
        png_cache[sz] = buf.getvalue()
        print(f"  ✓ {filename} ({sz}×{sz})")

    # Extra sizes for .icns
    for sz in [16, 64, 512, 1024]:
        if sz not in png_cache:
            img = master.resize((sz, sz), Image.Resampling.LANCZOS)
            buf = io.BytesIO()
            img.save(buf, "PNG")
            png_cache[sz] = buf.getvalue()

    # .icns
    icns_types = {
        "ic07": png_cache[128],
        "ic08": png_cache[256],
        "ic09": png_cache[512],
        "ic10": png_cache[1024],
        "ic11": png_cache[32],
        "ic12": png_cache[64],
        "ic13": png_cache[256],
        "ic14": png_cache[512],
    }
    icns_data = build_icns(icns_types)
    with open("icon.icns", "wb") as f:
        f.write(icns_data)
    print(f"  ✓ icon.icns ({len(icns_data):,} bytes)")

    # .ico
    ico_images = []
    for sz in [16, 32, 48, 64, 128, 256]:
        ico_images.append(master.resize((sz, sz), Image.Resampling.LANCZOS))
    ico_data = build_ico(ico_images)
    with open("icon.ico", "wb") as f:
        f.write(ico_data)
    print(f"  ✓ icon.ico ({len(ico_data):,} bytes)")

    # Tray icon
    tray = draw_tray_icon(22)
    tray.save("tray.png", "PNG")
    print("  ✓ tray.png (22×22)")

    print("\n✅ All Apple-style icons generated!")


if __name__ == "__main__":
    main()
