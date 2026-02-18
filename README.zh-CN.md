<p align="center">
  <img src="src-tauri/icons/128x128@2x.png" width="128" height="128" alt="Zureshot">
</p>

<h1 align="center">Zureshot</h1>

<p align="center">
  <strong>像素级精准的屏幕录制工具，支持 Mac 和 Linux。</strong><br>
  Rust 构建，原生 API 驱动。
</p>

<p align="center">
  <img src="https://img.shields.io/badge/macOS-13%2B-black?logo=apple" alt="macOS 13+">
  <img src="https://img.shields.io/badge/Linux-Ubuntu%2024.04%2B-orange?logo=ubuntu" alt="Ubuntu 24.04+">
  <img src="https://img.shields.io/badge/Apple%20Silicon-M1%20|%20M2%20|%20M3%20|%20M4-blue?logo=apple" alt="Apple Silicon">
  <img src="https://img.shields.io/badge/编码器-HEVC%20H.265-green" alt="HEVC">
  <img src="https://img.shields.io/badge/开源协议-MIT-yellow" alt="MIT">
</p>

<p align="center">
  <a href="README.md">English</a> · <a href="README.zh-CN.md">简体中文</a> · <a href="#wechat">💬 加微信</a>
</p>

---

## 为什么选择 Zureshot？

> **Z**ero-copy · p**ure** Rust · one **shot**

名字即宣言 —— **Zero + Pure + Shot**。

**Zero** —— 零拷贝 GPU 管线，CPU 全程不碰像素，录制时仅 3% 占用。
**Pure** —— 纯 Rust 铸造，无 Electron 膨胀。3.9 MB 安装包，11 MB 磁盘占用，零冗余。
**Shot** —— 一击即中，点击即录，录完即走。

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
- **🐧 Linux 支持** —— Ubuntu 24.04+，XDG Portal + GStreamer 管线（beta）

---

## 🏗 核心架构

### 系统总览

```
┌─────────────────────────────────────────────────────────────────────────┐
│                           Zureshot                                      │
├─────────────────┬───────────────────────────────────────────────────────┤
│   UI 层         │                  引擎 (Rust)                          │
│   Svelte 5      │                                                       │
│   (100% 共享)   │  ┌──────────────────────────────────────────────────┐ │
│                 │  │            平台抽象层 platform/mod.rs             │ │
│  菜单栏图标     │  │                                                  │ │
│  区域选择器     │  │  ┌──── macOS ─────┐   ┌──── Linux ──────────┐    │ │
│  录制控制条     │  │  │ SCK→IOSurf→    │   │ XDG Portal→         │    │ │
│  暗化遮罩       │  │  │ VideoToolbox   │   │ PipeWire→GStreamer  │    │ │
│  截图预览       │  │  │ →HEVC→MP4      │   │ →x264→MP4           │    │ │
│                 │  │  └────────────────┘   └─────────────────────┘    │ │
│                 │  └──────────────────────────────────────────────────┘ │
│                 │                                                       │
│                 │  ┌──────────┐ ┌──────────┐ ┌────────────────┐        │
│                 │  │ commands │ │   tray   │ │     lib        │        │
│                 │  │   .rs    │ │   .rs    │ │     .rs        │        │
│                 │  │ IPC 命令 │ │ 菜单栏   │ │ 引导启动       │        │
│                 │  └──────────┘ └──────────┘ └────────────────┘        │
├─────────────────┴───────────────────────────────────────────────────────┤
│                           Tauri v2                                      │
├──────────────────────────────────┬──────────────────────────────────────┤
│  macOS: ScreenCaptureKit        │  Linux: XDG Portal + GStreamer       │
│  VideoToolbox + AVFoundation    │  PipeWire + x264 + ffmpeg            │
│  objc2 FFI                      │  子进程管线（零外部 crate）          │
└──────────────────────────────────┴──────────────────────────────────────┘
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

### 数据流向 —— Linux（GStreamer 管线）

```
  用户点击录制
        │
        ▼
  XDG Desktop Portal (D-Bus)
    └─ CreateSession → SelectSources → Start
    └─ 用户在系统弹窗中确认共享屏幕/窗口
    └─ 返回 PipeWire node_id
        │
        ▼
  gst-launch-1.0 子进程
    └─ pipewiresrc（捕获 PipeWire 流）
    └─ videoconvert → videoscale → videocrop
    └─ x264enc（H.264 软件编码）
    └─ mp4mux → filesink（MP4 输出）
    └─ （可选）pulsesrc → audioconvert → faac
        │
        ▼
  暂停: SIGINT → EOS → 保存段文件
  恢复: 新 gst-launch 进程 → 新段
  停止: ffmpeg 拼接 → 最终 MP4
```

> **注意**：Linux 当前使用软件 H.264 编码。硬件加速编码（VA-API/NVENC，HEVC）计划在 v0.6.0 中实现。

### 源文件结构

| 文件 | 行数 | 职责 |
|------|------|------|
| `platform/mod.rs` | ~56 | 平台抽象：共享类型、条件编译 |
| `platform/macos/mod.rs` | ~340 | macOS RecordingHandle、生命周期、系统集成 |
| `platform/macos/capture.rs` | ~650 | SCK 流、SCStreamOutput 委托、帧路由、PTS |
| `platform/macos/writer.rs` | ~470 | AVAssetWriter、HEVC 编码、BT.709、文件终结 |
| `platform/linux/mod.rs` | ~430 | Linux RecordingHandle、生命周期、系统集成 |
| `platform/linux/portal.rs` | ~315 | XDG Desktop Portal ScreenCast（D-Bus via Python）|
| `platform/linux/writer.rs` | ~360 | GStreamer 管线管理（gst-launch 子进程）|
| `platform/linux/capture.rs` | ~90 | 截屏（gnome-screenshot / grim）|
| `commands.rs` | ~940 | Tauri IPC 命令、录制状态机、窗口管理 |
| `tray.rs` | ~520 | 系统托盘、菜单、自动启动、更新检查 |
| `lib.rs` | ~70 | 应用引导、插件注册 |

**Rust** 处理所有采集、编码和文件 I/O。UI 是轻量的 Svelte 层（约 5 个组件），负责菜单栏、区域选择和录制控制。Tauri v2 通过类型安全的 IPC 连接两者。

### 技术栈

| 层 | macOS | Linux |
|----|-------|-------|
| 采集 | ScreenCaptureKit（GPU 零拷贝） | XDG Desktop Portal + PipeWire |
| 编码 | VideoToolbox HEVC（硬件编码） | x264 H.264（软件编码，VA-API 计划中） |
| 封装 | AVAssetWriter → MP4 | GStreamer mp4mux → MP4 |
| 音频 | ScreenCaptureKit AAC | PulseAudio + GStreamer AAC |
| 截屏 | CGWindowListCreateImage | gnome-screenshot / grim |
| FFI | objc2 0.6 + block2 0.6 | 子进程（零原生 crate 依赖）|
| 对话框 | osascript（AppleScript） | zenity / kdialog |
| 应用外壳 | Tauri v2 | Tauri v2 |
| 前端 | Svelte 5 + Vite（100% 共享） | Svelte 5 + Vite（100% 共享） |

---

## 🚀 快速开始

### macOS

```bash
# 前置条件：Rust、Node.js、pnpm
git clone https://github.com/anxiong2025/zureshot.git
cd zureshot
pnpm install
pnpm tauri dev
```

> **首次启动**：macOS 会请求屏幕录制权限。前往 **系统设置 → 隐私与安全性 → 屏幕录制** 中授权，然后重启应用。

### Linux (Ubuntu 24.04+)

```bash
# 安装系统依赖
sudo apt-get install -y \
  libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf \
  xdg-desktop-portal zenity python3 python3-dbus python3-gi \
  gir1.2-glib-2.0 gstreamer1.0-tools gstreamer1.0-plugins-base \
  gstreamer1.0-plugins-good gstreamer1.0-plugins-ugly \
  gstreamer1.0-pipewire ffmpeg

# 从源码构建
git clone https://github.com/anxiong2025/zureshot.git
cd zureshot
pnpm install
pnpm tauri dev
```

或从 Release 安装：

```bash
# .deb 包
sudo dpkg -i Zureshot_*.deb

# 或 AppImage
chmod +x Zureshot_*.AppImage
./Zureshot_*.AppImage
```

> **首次启动**：首次录屏或截图时，桌面环境会弹出 Portal 对话框，询问共享哪个屏幕/窗口。这是标准的 Linux 安全机制。

> ⚠️ **Linux 支持目前为 beta 状态。** 录屏使用 XDG Desktop Portal + GStreamer。已在 Ubuntu 24.04 GNOME (Wayland) 上测试。欢迎反馈和提 bug！

---

## 🔧 生产构建

```bash
pnpm tauri build
```

**macOS：** `.dmg` 安装包在 `src-tauri/target/release/bundle/dmg/` 目录下。

**Linux：** `.deb` 和 `.AppImage` 在 `src-tauri/target/release/bundle/deb/` 和 `src-tauri/target/release/bundle/appimage/` 目录下。

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
| **Linux** | Ubuntu 24.04 (Wayland) | GNOME 桌面 |
| **内存** | 8 GB | 16 GB |
| **磁盘** | ~200 MB/分钟（标准画质） | 推荐 SSD |
| **显示器** | 任意分辨率 | Retina (2x) 画质最佳 |

### Linux 环境要求（beta）

| 组件 | 必需 | 说明 |
|------|------|------|
| **桌面** | GNOME（推荐 Wayland） | X11 也支持 |
| **显示服务** | PipeWire | 屏幕采集传输 |
| **Portal** | XDG Desktop Portal | 权限管理 & 源选择 |
| **编码** | GStreamer + x264 | 视频编码（H.264）|
| **音频** | PulseAudio / PipeWire-Pulse | 系统音频采集 |
| **截屏** | gnome-screenshot / grim | 区域截图 |
| **对话框** | zenity | 原生对话框 |

> 🐧 Linux 录屏使用 GStreamer 子进程管线 + H.264 编码。硬件加速编码（VA-API、NVENC）计划在未来版本中实现。

---

## 🗺 路线图

### ✅ v0.4 — 当前版本
- [x] 全屏 & 区域录制（macOS: HEVC 零拷贝）
- [x] 暂停 / 恢复
- [x] 系统声音 + 麦克风采集
- [x] GIF 导出（调色板优化，最高 30fps）
- [x] 截图模式（全屏 / 区域）+ 预览
- [x] 复制到剪贴板
- [x] 录制倒计时（3-2-1）
- [x] 录制停止时缩略图预览
- [x] 画质预设（标准 30fps / 高清 60fps）
- [x] 自动更新（Tauri updater）

### 🚧 v0.5.0-beta — Linux 支持（当前）
- [x] 平台抽象层（macOS + Linux 同一代码库）
- [x] Linux 录屏（XDG Portal + GStreamer + PipeWire）
- [x] Linux 截屏（gnome-screenshot / grim）
- [x] Linux 系统集成（zenity 对话框、xdg-open、开机自启）
- [x] CI/CD：macOS + Ubuntu 双平台构建 & 发布
- [ ] Ubuntu 24.04 实机验证

### 🔮 v0.6.0-beta — Linux 性能优化
- [ ] 纯 Rust D-Bus（zbus crate，移除 Python 依赖）
- [ ] 进程内 GStreamer 管线（gstreamer-rs crate）
- [ ] 硬件编码：VA-API (Intel/AMD) / NVENC (NVIDIA)
- [ ] Linux 端 HEVC (H.265) 输出
- [ ] 无缝暂停/恢复（GstPipeline 状态切换，无段拼接）

### v0.7 — 修剪与导出
- [ ] 录制结束后弹出预览窗口
- [ ] 拖拽修剪：首尾 Range Slider
- [ ] Stream copy 极速导出（不重编码，秒级完成）

### v1.0 — 演示模式 🎯
- [ ] **自动缩放**：摄像机自动跟随光标并放大聚焦区域
- [ ] **点击涟漪**：鼠标点击时产生视觉涟漪效果
- [ ] **按键显示**：在屏幕上实时展示键盘按键
- [ ] **聚光灯模式**：光标周围可配置半径保持明亮，其余区域自动暗化
- [ ] **平滑跟随**：电影级摄像机移动，可配置缓动曲线

> **愿景**：Zureshot 致力于成为开发者和内容创作者录制教程、产品演示和技术讲解的首选工具——将像素级精准的录制品质与智能演示功能相结合，让每一段录屏都呈现专业制作级的效果。

### 远期展望
- [ ] 多显示器选择
- [ ] 屏幕标注（箭头、矩形、文字）
- [ ] 摄像头画中画
- [ ] 自动上传云端（S3、R2、自定义端点）
- [ ] 插件系统，支持自定义后处理

---

## 💬 联系作者

<a id="wechat"></a>

欢迎加微信交流、反馈 bug、提需求：

<p align="center">
  <img src="docs/images/wechat.jpg" width="300" alt="微信二维码">
</p>

---

## 📄 开源协议

MIT © [Zureshot](https://github.com/anxiong2025/zureshot)
