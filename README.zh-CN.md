<p align="center">
  <img src="src-tauri/icons/128x128@2x.png" width="128" height="128" alt="Zureshot">
</p>

<h1 align="center">Zureshot</h1>

<p align="center">
  <strong>像素级精准的 Mac 屏幕录制工具。</strong><br>
  Rust 构建，为 Apple Silicon 而生。
</p>

<p align="center">
  <img src="https://img.shields.io/badge/macOS-13%2B-black?logo=apple" alt="macOS 13+">
  <img src="https://img.shields.io/badge/Apple%20Silicon-M1%20|%20M2%20|%20M3%20|%20M4-blue?logo=apple" alt="Apple Silicon">
  <img src="https://img.shields.io/badge/编码器-HEVC%20H.265-green" alt="HEVC">
  <img src="https://img.shields.io/badge/开源协议-MIT-yellow" alt="MIT">
</p>

<p align="center">
  <a href="README.md">English</a> · <a href="README.zh-CN.md">简体中文</a>
</p>

---

## 为什么选择 Zureshot？

大多数录屏工具把你的 Mac 当 2015 年的老电脑使——像素在 CPU 里来回拷贝，内存疯狂膨胀，风扇呼呼转。

**Zureshot 从不触碰你的像素。** 每一帧都在纯 GPU 管线中流转，从采集到写入文件，全程不经过 CPU。结果：录出来的画面和你屏幕上看到的一模一样，而系统几乎毫无感知。

---

## ✨ 核心技术

### 🎯 真 · Retina 录制

你的 Mac 以 2× 甚至 3× 物理像素渲染画面。大多数工具悄悄降分辨率来省事。**Zureshot 录下每一个物理像素。**

> 3200×2132 的 Retina 屏幕，录出来就是 3200×2132。不是 1600×1066。没有例外。

文字始终锐利。UI 元素保持清晰边缘。所见即所录——像素级精准。

### 🚀 零拷贝 GPU 管线

整条录制链路都在 GPU 上完成。像素数据**从未进入你的应用内存**。

```
ScreenCaptureKit → IOSurface (GPU 显存) → VideoToolbox HEVC 硬编码 → MP4
                          ↑                         ↑
                    零 CPU 拷贝                 硬件编码器
```

- **ScreenCaptureKit** 采集帧为 GPU 驻留的 IOSurface
- **VideoToolbox** 直接从 IOSurface 硬件编码
- **AVAssetWriter** 将编码后的 NALU 封装进 MP4

没有 `memcpy`，没有 `Vec<u8>`，没有帧缓冲区驻留 RAM。CPU 几乎不知道在录屏。

### 🧊 极致低资源占用

| 指标 | Zureshot | 同类录屏工具 |
|------|----------|------------|
| 录制时额外内存 | **~30-50 MB** | 200-500 MB |
| CPU 占用 | **< 3%** | 15-40% |
| GPU 开销 | **< 5%** | 10-25% |
| 风扇噪音 | **静音** | 经常可闻 |

Mac 保持冰凉。电池续航更长。其他应用不卡顿。

### 🎨 精准色彩还原

每段录像都标记了完整的 **BT.709 色彩管线**：

- **色彩基色**：ITU-R BT.709 —— 匹配 sRGB 显示器
- **传输函数**：BT.709 —— 正确的伽马曲线
- **YCbCr 矩阵**：BT.709 —— 精确的亮度/色度分离
- **采集色彩空间**：sRGB —— 无隐式 P3→709 转换

在任何设备上播放，色彩都与你的屏幕精确一致。

### ⚡ HEVC (H.265) 硬件编码

Zureshot 使用 **HEVC Main 档** + Apple Silicon 专用媒体引擎：

- 同画质下文件比 H.264 **小 40-50%**
- **自适应码率** —— 4K 最高 36 Mbps，针对屏幕内容调优
- **质量目标 VBR** —— 编码器优先保证文字锐度而非压缩文件
- **禁用帧重排** —— 最小延迟，即停即存
- **2 秒关键帧** —— 任何播放器都能流畅拖拽

Retina 分辨率 60fps 录制 60 秒：**约 135 MB**（H.264 则超过 200 MB）。

### 🖥 全系 Apple Silicon 支持

| 芯片 | 支持状态 | 说明 |
|------|---------|------|
| **M1 / M1 Pro / M1 Max / M1 Ultra** | ✅ 完整支持 | 硬件 HEVC + 零拷贝管线 |
| **M2 / M2 Pro / M2 Max / M2 Ultra** | ✅ 完整支持 | 同上 |
| **M3 / M3 Pro / M3 Max** | ✅ 完整支持 | 同上 |
| **M4 / M4 Pro / M4 Max** | ✅ 完整支持 | 同上，Metal 4 |
| **Intel Mac (T2 芯片)** | ⚠️ 可用 | 硬件 HEVC 可用，但无统一内存优势 |
| **Intel Mac (无 T2)** | ⚠️ 可用 | 回退到软件编码，CPU 占用较高 |

> 推荐 M 系列芯片以获得最佳体验。所有 M 系列 Mac（包括 MacBook Air/Pro、Mac mini、iMac、Mac Studio、Mac Pro）均完整支持。

---

## 🎬 功能一览

- **📹 全屏录制** —— 原生 Retina 分辨率，菜单栏一键启动
- **🔲 区域录制** —— 拖拽选区，像素级精准裁切
- **⏸ 暂停 / 恢复** —— 原子操作零开销，无编码空隙
- **🔊 系统声音** —— 通过 ScreenCaptureKit 采集应用音频
- **🎤 麦克风** —— 独立 AAC 音轨，硬件编码
- **🖱 光标录制** —— macOS 合成器绘制，零 CPU 开销
- **🎯 窗口排除** —— 自动隐藏 Zureshot 自身界面
- **⌨️ 快捷键** —— `⌘⇧R` 录屏，`⌘⇧A` 区域选择
- **🌗 画质预设** —— 标准 (30fps) 和 高清 (60fps)

---

## 🏗 核心架构

### 系统总览

```
┌─────────────────────────────────────────────────────────────────────┐
│                           Zureshot                                  │
├─────────────────┬───────────────────────────────────────────────────┤
│   UI 层         │                引擎 (Rust)                        │
│   Svelte 5      │                                                   │
│                 │  ┌──────────────────────────────────────────────┐ │
│  菜单栏图标     │  │             采集管线                         │ │
│  区域选择器     │  │                                              │ │
│  录制控制条     │  │  SCK ──→ IOSurface ──→ VideoToolbox ──→ MP4  │ │
│  暗化遮罩       │  │  (GPU)   (GPU/显存)    (媒体引擎)      (SSD) │ │
│                 │  │                                              │ │
│                 │  │  音频: SCK ──→ CMSampleBuffer ──→ AAC ──┘   │ │
│                 │  └──────────────────────────────────────────────┘ │
│                 │                                                   │
│                 │  ┌─────────┐ ┌──────────┐ ┌───────────────────┐  │
│                 │  │ capture │ │  writer  │ │    commands       │  │
│                 │  │   .rs   │ │   .rs    │ │      .rs          │  │
│                 │  │ SCK 接口│ │ AVAsset  │ │ Tauri IPC 桥接    │  │
│                 │  │ 帧委托  │ │ 编码写入 │ │ 状态管理          │  │
│                 │  └─────────┘ └──────────┘ └───────────────────┘  │
├─────────────────┴───────────────────────────────────────────────────┤
│                    Tauri v2 + objc2 FFI                             │
├─────────────────────────────────────────────────────────────────────┤
│  macOS: ScreenCaptureKit │ VideoToolbox │ AVFoundation │ CoreMedia  │
└─────────────────────────────────────────────────────────────────────┘
```

### 数据流向 —— 零拷贝路径

```
                            Apple Silicon SoC
                    ┌───────────────────────────────┐
                    │                               │
  显示器输出 ───────┤  Window Server 合成画面        │
                    │  写入 IOSurface               │
                    │         │                     │
                    │         ▼                     │
                    │  ScreenCaptureKit 投递         │
                    │  CMSampleBuffer（IOSurface    │
                    │  句柄——不是像素数据）          │
                    │         │                     │
                    │         ▼                     │
                    │  VideoToolbox 读取 IOSurface   │
                    │  通过 Apple Media Engine       │
                    │  （专用 HEVC 硬件）            │
                    │         │                     │
                    │         ▼                     │
                    │  编码后的 H.265 NALU           │
                    │  （极小的压缩数据包）          │
                    │                               │
                    └───────────┬───────────────────┘
                                │
                                ▼
                    AVAssetWriter → MP4 文件写入磁盘
```

**核心要点**：像素数据（例如 3200×2132 × 1.5 字节/像素 = ~10 MB/帧）始终停留在统一 GPU 内存中。只有极小的压缩 NALU（~50-100 KB/帧）经过 CPU 内存写入磁盘。

### 源文件结构

| 文件 | 行数 | 职责 |
|------|------|------|
| `capture.rs` | ~650 | SCK 流配置、SCStreamOutput 委托、帧路由、PTS 单调性保证 |
| `writer.rs` | ~470 | AVAssetWriter 创建、HEVC 编码参数、BT.709 色彩、文件终结 |
| `commands.rs` | ~820 | Tauri IPC 命令、录制状态机、窗口管理 |
| `tray.rs` | ~250 | 系统托盘图标、右键菜单、快捷键处理 |
| `lib.rs` | ~60 | 应用引导、插件注册 |

**Rust** 处理所有采集、编码和文件 I/O。UI 是轻量的 Svelte 层（约 5 个组件），负责菜单栏、区域选择和录制控制。Tauri v2 通过类型安全的 IPC 连接两者。

### 技术栈

| 层 | 技术 | 选型理由 |
|----|------|----------|
| 采集 | ScreenCaptureKit (macOS 12.3+) | 新一代采集 API，原生 GPU IOSurface 输出 |
| 像素格式 | NV12 (`420v`) | HEVC 编码器原生格式——零色彩空间转换 |
| 色彩空间 | sRGB 采集 → BT.709 编码 | 无损元数据匹配，无隐式转换 |
| 编码 | VideoToolbox HEVC Main | Apple Media Engine 硬件编码，约 3% CPU |
| 封装 | AVAssetWriter → MP4 | 原生 Apple 封装器，正确的 moov atom，即时拖拽 |
| 音频 | AAC 48kHz 立体声，128kbps | 系统声音 + 麦克风，双轨录制 |
| FFI | objc2 0.6 + block2 0.6 | 类型安全的 Rust ↔ Objective-C 桥接 |
| 应用外壳 | Tauri v2 | 轻量原生包装，约 3 MB 二进制 |
| 前端 | Svelte 5 + Vite | 极简 UI，仅用于遮罩和控件 |

---

## 🚀 快速开始

```bash
# 前置条件：Rust、Node.js、pnpm
git clone https://github.com/anxiong2025/zureshot.git
cd zureshot
pnpm install
pnpm tauri dev
```

> **首次启动**：macOS 会请求屏幕录制权限。前往 **系统设置 → 隐私与安全性 → 屏幕录制** 中授权，然后重启应用。

---

## 🔧 生产构建

```bash
pnpm tauri build
```

`.dmg` 安装包会在 `src-tauri/target/release/bundle/dmg/` 目录下。

---

## � 兼容设备

### Apple Silicon（推荐 —— 完整零拷贝管线）

| Mac 机型 | 芯片 | 采集 | 编码 | 备注 |
|---------|------|------|------|------|
| **MacBook Air** | M1 / M2 / M3 / M4 | ✅ SCK 零拷贝 | ✅ 硬件 HEVC | 无风扇设计——录屏真正静音 |
| **MacBook Pro** 14" 16" | M1 Pro/Max — M4 Pro/Max | ✅ SCK 零拷贝 | ✅ 硬件 HEVC | Pro/Max 芯片有多个媒体引擎 |
| **Mac mini** | M1 / M2 / M2 Pro / M4 / M4 Pro | ✅ SCK 零拷贝 | ✅ 硬件 HEVC | 适合桌面录屏场景 |
| **iMac** | M1 / M3 / M4 | ✅ SCK 零拷贝 | ✅ 硬件 HEVC | 4.5K/5K Retina 完整支持 |
| **Mac Studio** | M1 Max/Ultra / M2 Max/Ultra / M4 Max | ✅ SCK 零拷贝 | ✅ 硬件 HEVC | 多编码器实例，最高吞吐 |
| **Mac Pro** | M2 Ultra | ✅ SCK 零拷贝 | ✅ 硬件 HEVC | 最多 4 个媒体引擎 |

> **所有 M 系列芯片共享相同的 Apple Media Engine 架构。** M1 到 M4（含 Pro / Max / Ultra 变体）的零拷贝录制品质完全一致。更高端的芯片只是拥有更多编码器实例用于并行任务。

### Intel Mac（支持，有限制）

| 配置 | 采集 | 编码 | 限制 |
|------|------|------|------|
| Intel + **T2 芯片**（2018-2020 机型） | ✅ SCK | ✅ T2 硬件 HEVC | 无统一内存——CPU↔GPU 间有额外拷贝 |
| Intel **无 T2**（2018 年前） | ✅ SCK | ⚠️ 软件 HEVC | CPU 占用较高（15-30%），可能影响性能 |

### 系统要求

| 要求 | 最低 | 推荐 |
|------|------|------|
| **macOS** | 13.0 Ventura | 14.0+ Sonoma |
| **内存** | 8 GB | 16 GB |
| **磁盘** | ~200 MB/分钟（标准画质） | 推荐 SSD |
| **显示器** | 任意分辨率 | Retina (2x) 画质最佳 |

---

## 🗺 路线图

- [ ] 多显示器选择
- [ ] GIF / WebM 导出
- [ ] 标注工具（箭头、文字、高亮）
- [ ] 自动上传云端
- [ ] 停止时缩略图预览
- [ ] 全局设置面板

---

## 📄 开源协议

MIT © [Zureshot](https://github.com/anxiong2025/zureshot)
