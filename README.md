<p align="center">
  <img src="src-tauri/icons/128x128@2x.png" width="128" height="128" alt="Zureshot">
</p>

<h1 align="center">Zureshot</h1>

<p align="center">
  <strong>Pixel-perfect screen recording for Mac.</strong><br>
  Built with Rust. Powered by Apple Silicon.
</p>

<p align="center">
  <img src="https://img.shields.io/badge/macOS-13%2B-black?logo=apple" alt="macOS 13+">
  <img src="https://img.shields.io/badge/Apple%20Silicon-M1%20|%20M2%20|%20M3%20|%20M4-blue?logo=apple" alt="Apple Silicon">
  <img src="https://img.shields.io/badge/Codec-HEVC%20H.265-green" alt="HEVC">
  <img src="https://img.shields.io/badge/License-MIT-yellow" alt="MIT">
</p>

<p align="center">
  <a href="README.md">English</a> · <a href="README.zh-CN.md">简体中文</a>
</p>

---

## Why Zureshot?

> **Z**ero-copy · p**ure** Rust · one **shot**

The name says it all — **Zero + Pure + Shot**.

**Zero** — zero-copy GPU pipeline. The CPU never touches a single pixel. 3% usage while recording.
**Pure** — pure Rust, no Electron bloat. 3.9 MB installer, 11 MB on disk. Nothing wasted.
**Shot** — one shot, instant capture. Click to record, done.

Most screen recorders treat your Mac like a 2015 laptop — copying pixels through CPU, bloating memory, spinning fans.

**Zureshot doesn't touch your pixels.** Every frame flows through a pure GPU pipeline, from capture to file. The result: recordings that look exactly like your screen, using almost no resources.

---

## ✨ Core Technology

### 🎯 True Retina Recording

Your Mac renders at 2× or 3× physical pixels. Most tools quietly downscale to save bandwidth. **Zureshot records every single physical pixel.**

> A 3200×2132 Retina display records at 3200×2132. Not 1600×1066. No exceptions.

Text stays razor-sharp. UI elements keep their crisp edges. What you see is what you get — pixel for pixel.

### 🚀 Zero-Copy GPU Pipeline

The entire recording path lives on the GPU. Pixel data **never enters your app's memory**.

```
ScreenCaptureKit → IOSurface (GPU) → VideoToolbox HEVC → MP4
                          ↑                    ↑
                    Zero CPU copy        Hardware encoder
```

- **ScreenCaptureKit** captures frames as GPU-resident IOSurfaces
- **VideoToolbox** hardware-encodes directly from those surfaces
- **AVAssetWriter** muxes the encoded NALUs into MP4

No `memcpy`. No `Vec<u8>`. No frame buffers in RAM. The CPU barely knows a recording is happening.

### 🧊 Absurdly Low Resource Usage

| Metric | Zureshot | Typical Screen Recorder |
|--------|----------|------------------------|
| Extra RAM during recording | **~30-50 MB** | 200-500 MB |
| CPU usage | **< 3%** | 15-40% |
| GPU overhead | **< 5%** | 10-25% |
| Fan noise | **Silent** | Often audible |

Your Mac stays cool. Your battery lasts longer. Your other apps don't stutter.

### 🎨 Color-Accurate Output

Every recording is tagged with the full **BT.709 color pipeline**:

- **Color Primaries**: ITU-R BT.709 — matches sRGB displays
- **Transfer Function**: BT.709 — correct gamma curve
- **YCbCr Matrix**: BT.709 — precise luma/chroma separation
- **Capture Color Space**: sRGB — no implicit P3→709 conversion

Play your recording on any device and the colors will match your screen exactly.

### ⚡ HEVC (H.265) Hardware Encoding

Zureshot uses **HEVC Main profile** with Apple Silicon's dedicated media engine:

- **40-50% smaller** files than H.264 at equal quality
- **Adaptive bitrate** — up to 36 Mbps for 4K, tuned for screen content
- **Quality-targeted VBR** — encoder prioritizes text sharpness over file size
- **No frame reordering** — minimal latency, instant stop
- **2-second keyframes** — smooth seeking in any player

A 60-second Retina recording at 60fps: **~135 MB** (vs 200+ MB with H.264).

---

## 🎬 Features

- **📹 Full Screen Recording** — native Retina resolution, one click from tray
- **🔲 Region Recording** — drag to select any area, pixel-perfect cropping
- **⏸ Pause / Resume** — zero-overhead atomic flag, no encoding gaps
- **🔊 System Audio** — capture app sounds via ScreenCaptureKit
- **🎤 Microphone** — separate AAC track, hardware-encoded
- **🖱 Cursor Capture** — rendered by macOS compositor, zero CPU cost
- **🎯 Window Exclusion** — automatically hides Zureshot's own UI from recordings
- **⌨️ Keyboard Shortcuts** — `⌘⇧R` to record, `⌘⇧A` for region select
- **🌗 Quality Presets** — Standard (30fps) and High (60fps)

---

## 🏗 Architecture

### System Overview

```
┌─────────────────────────────────────────────────────────────────────┐
│                         Zureshot                                    │
├─────────────────┬───────────────────────────────────────────────────┤
│   UI Layer      │              Engine (Rust)                        │
│   Svelte 5      │                                                   │
│                 │  ┌──────────────────────────────────────────────┐ │
│  Tray Menu      │  │           Capture Pipeline                   │ │
│  Region Select  │  │                                              │ │
│  Recording Bar  │  │  SCK ──→ IOSurface ──→ VideoToolbox ──→ MP4  │ │
│  Dim Overlay    │  │  (GPU)    (GPU/VRAM)    (Media Engine)  (SSD) │ │
│                 │  │                                              │ │
│                 │  │  Audio: SCK ──→ CMSampleBuffer ──→ AAC ──┘   │ │
│                 │  └──────────────────────────────────────────────┘ │
│                 │                                                   │
│                 │  ┌─────────┐ ┌──────────┐ ┌───────────────────┐  │
│                 │  │ capture │ │  writer  │ │    commands       │  │
│                 │  │   .rs   │ │   .rs    │ │      .rs          │  │
│                 │  │ SCK API │ │ AVAsset  │ │ Tauri IPC bridge  │  │
│                 │  │ Delegate│ │ Writer   │ │ State management  │  │
│                 │  └─────────┘ └──────────┘ └───────────────────┘  │
├─────────────────┴───────────────────────────────────────────────────┤
│                    Tauri v2 + objc2 FFI                             │
├─────────────────────────────────────────────────────────────────────┤
│  macOS: ScreenCaptureKit │ VideoToolbox │ AVFoundation │ CoreMedia  │
└─────────────────────────────────────────────────────────────────────┘
```

### Data Flow — Zero-Copy Path

```
                              Apple Silicon SoC
                    ┌───────────────────────────────┐
                    │                               │
  Display Output ───┤  Window Server composites     │
                    │  frame into IOSurface         │
                    │         │                     │
                    │         ▼                     │
                    │  ScreenCaptureKit delivers    │
                    │  CMSampleBuffer (IOSurface    │
                    │  handle — NOT pixel data)     │
                    │         │                     │
                    │         ▼                     │
                    │  VideoToolbox reads IOSurface │
                    │  via Apple Media Engine       │
                    │  (dedicated HEVC hardware)    │
                    │         │                     │
                    │         ▼                     │
                    │  Encoded H.265 NALUs          │
                    │  (tiny compressed packets)    │
                    │                               │
                    └───────────┬───────────────────┘
                                │
                                ▼
                    AVAssetWriter → MP4 file on disk
```

**Key insight**: The pixel data (e.g., 3200×2132 × 1.5 bytes/pixel = ~10 MB/frame) stays entirely in unified GPU memory. Only the tiny compressed NALUs (~50-100 KB/frame) pass through CPU memory on the way to disk.

### Source Files

| File | Lines | Responsibility |
|------|-------|----------------|
| `capture.rs` | ~650 | SCK stream setup, SCStreamOutput delegate, frame routing, PTS enforcement |
| `writer.rs` | ~470 | AVAssetWriter creation, HEVC encoding settings, BT.709 color, finalization |
| `commands.rs` | ~820 | Tauri IPC commands, recording state machine, window management |
| `tray.rs` | ~250 | System tray icon, context menu, shortcut handling |
| `lib.rs` | ~60 | App bootstrap, plugin registration |

**Rust** handles all capture, encoding, and file I/O. The UI is a thin Svelte layer (~5 components) for tray menus, region selection, and recording controls. Tauri v2 bridges the two with type-safe IPC.

### Tech Stack

| Layer | Technology | Why |
|-------|-----------|-----|
| Capture | ScreenCaptureKit (macOS 12.3+) | Next-gen capture API, GPU-native IOSurface output |
| Pixel Format | NV12 (`420v`) | Native format for HEVC encoder — zero color conversion |
| Color Space | sRGB capture → BT.709 encoding | Lossless metadata match, no implicit conversion |
| Encoding | VideoToolbox HEVC Main | Apple Media Engine hardware, ~3% CPU |
| Container | AVAssetWriter → MP4 | Native Apple muxer, proper moov atom, instant seek |
| Audio | AAC 48kHz stereo, 128kbps | System audio + microphone, dual track |
| FFI | objc2 0.6 + block2 0.6 | Type-safe Rust ↔ Objective-C bridge |
| App Shell | Tauri v2 | Lightweight native wrapper, ~3 MB binary |
| Frontend | Svelte 5 + Vite | Minimal UI for overlays and controls |

---

## 🚀 Quick Start

```bash
# Prerequisites: Rust, Node.js, pnpm
git clone https://github.com/anxiong2025/zureshot.git
cd zureshot
pnpm install
pnpm tauri dev
```

> **First launch**: macOS will ask for Screen Recording permission. Grant it in **System Settings → Privacy & Security → Screen Recording**, then restart the app.

---

## 🔧 Build for Production

```bash
pnpm tauri build
```

The `.dmg` installer will be in `src-tauri/target/release/bundle/dmg/`.

---

## � Compatible Devices

### Apple Silicon (Recommended — Full Zero-Copy Pipeline)

| Mac | Chips | Capture | Encoding | Notes |
|-----|-------|---------|----------|-------|
| **MacBook Air** | M1 / M2 / M3 / M4 | ✅ SCK Zero-Copy | ✅ Hardware HEVC | Fanless — truly silent recording |
| **MacBook Pro** 14" 16" | M1 Pro/Max — M4 Pro/Max | ✅ SCK Zero-Copy | ✅ Hardware HEVC | Multiple media engines on Pro/Max |
| **Mac mini** | M1 / M2 / M2 Pro / M4 / M4 Pro | ✅ SCK Zero-Copy | ✅ Hardware HEVC | Great for desktop recording setups |
| **iMac** | M1 / M3 / M4 | ✅ SCK Zero-Copy | ✅ Hardware HEVC | 4.5K/5K Retina fully supported |
| **Mac Studio** | M1 Max/Ultra / M2 Max/Ultra / M4 Max | ✅ SCK Zero-Copy | ✅ Hardware HEVC | Multi-encoder for highest throughput |
| **Mac Pro** | M2 Ultra | ✅ SCK Zero-Copy | ✅ Hardware HEVC | Up to 4 media engines |

> **All M-series chips share the same Apple Media Engine architecture.** M1 through M4 (including Pro / Max / Ultra variants) deliver identical zero-copy recording quality. Higher-tier chips simply have more encoder instances for parallel workloads.

### Intel Macs (Supported with Limitations)

| Configuration | Capture | Encoding | Limitations |
|---------------|---------|----------|-------------|
| Intel + **T2 chip** (2018-2020 models) | ✅ SCK | ✅ Hardware HEVC via T2 | No unified memory — extra copy between CPU↔GPU |
| Intel **without T2** (pre-2018) | ✅ SCK | ⚠️ Software HEVC | Higher CPU usage (15-30%), may impact performance |

### System Requirements

| Requirement | Minimum | Recommended |
|-------------|---------|-------------|
| **macOS** | 13.0 Ventura | 14.0+ Sonoma |
| **RAM** | 8 GB | 16 GB |
| **Disk** | ~200 MB/min (Standard) | SSD recommended |
| **Display** | Any resolution | Retina (2x) for best quality |

---

## 🗺 Roadmap

### v0.2 — Trim & Export
- [ ] Post-recording preview window
- [ ] Drag-to-trim: start/end range slider
- [ ] Stream copy export (no re-encoding, instant)
- [ ] Optional hardware-accelerated 4K → 1080p transcode

### v0.3 — Polish & Quality of Life
- [ ] Multi-display selection
- [ ] Thumbnail preview on recording stop
- [ ] Global settings panel (output path, format, quality)
- [ ] Auto-open in Finder after export
- [ ] Recording countdown timer (3-2-1)

### v0.4 — Export Formats
- [ ] GIF export (palette-optimized, max 30fps)
- [ ] WebM / VP9 export
- [ ] Screenshot mode (full screen / region)
- [ ] Copy to clipboard

### v0.5 — Annotation & Overlay
- [ ] On-screen annotation: arrows, rectangles, text
- [ ] Highlight / spotlight effect (dim outside cursor area)
- [ ] Webcam overlay (picture-in-picture circle)
- [ ] Customizable watermark

### v1.0 — Presentation Mode 🎯
- [ ] **Auto Zoom**: camera automatically follows cursor and zooms into the focused area — perfect for tutorials, demos, and walkthroughs
- [ ] **Click Ripple**: visual ripple effect on mouse click to highlight interactions
- [ ] **Keystroke Overlay**: display pressed keys on screen for shortcut demonstrations
- [ ] **Spotlight Mode**: dim everything except a configurable radius around the cursor
- [ ] **Smooth Pan**: cinematic camera movement with configurable easing curves
- [ ] **Scene Presets**: save and switch between zoom levels / focus areas

> **Vision**: Zureshot aims to become the go-to tool for developers and creators who record tutorials, product demos, and technical walkthroughs — combining pixel-perfect capture quality with intelligent presentation features that make every recording look professionally produced.

### Future
- [ ] Real-time LUT / color filters (Core Image or Metal Compute)
- [ ] Auto-upload to cloud (S3, R2, custom endpoint)
- [ ] Plugin system for custom post-processing
- [ ] Apple Shortcuts integration
- [ ] Menu bar recording indicator with live waveform

---

## 📄 License

MIT © [Zureshot](https://github.com/anxiong2025/zureshot)
