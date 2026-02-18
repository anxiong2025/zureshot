# Zureshot Linux 支持开发计划

> 目标平台：**Ubuntu 24.04 LTS (Noble) x86_64**
> 发行格式：**.deb** + **.AppImage**
> 显示协议：**优先 Wayland（XDG Portal）**，兼容 X11

---

## 一、代码复用分析

### 总览

| 模块 | 代码行数 | macOS 专有 | 可复用 | Linux 工作量 |
|------|---------|-----------|--------|-------------|
| `capture.rs` | 917 行 | **100%** | 0% | 🔴 全部重写 |
| `writer.rs` | 485 行 | **100%** | 0% | 🔴 全部重写 |
| `commands.rs` | 1187 行 | ~60% | ~40% | 🟡 大幅重构 |
| `tray.rs` | 484 行 | ~15% | ~85% | 🟢 少量修改 |
| `lib.rs` | 67 行 | ~5% | ~95% | 🟢 几乎不变 |
| 前端 (Svelte/HTML/JS) | ~2000+ 行 | 0% | **100%** | ✅ 零修改 |
| `Cargo.toml` | 45 行 | ~50% | ~50% | 🟡 加 Linux 依赖 |
| `tauri.conf.json` | - | ~10% | ~90% | 🟢 加 Linux bundle 配置 |
| CI/CD | 0 行 | - | - | 🔴 新建 |

**结论：前端 100% 复用，Rust 后端约 40% 可复用，需新写约 1500-2000 行 Rust 代码。**

---

### 各模块详细分析

#### 🔴 `capture.rs` — 需全部重写

当前使用的 macOS 专有 API：
- **ScreenCaptureKit**: `SCStream`, `SCContentFilter`, `SCShareableContent`, `SCStreamConfiguration`
- **CoreGraphics**: `CGWindowListCreateImage` (截屏), `CGDisplayCopyDisplayMode` (分辨率)
- **CoreMedia**: `CMSampleBuffer`, `CMTime`
- **ImageIO**: `CGImageDestination` (PNG 写入)
- **Objective-C Runtime**: `objc2`, `define_class!`, `msg_send!`, `block2`, `dispatch2`

Linux 替代方案：
- 录屏：**PipeWire** + **XDG Desktop Portal** `Screencast` 接口
- 截屏：**XDG Desktop Portal** `Screenshot` 接口，或 `grim`(Wayland) / `xdotool`+`import`(X11)
- 窗口列表：Portal API 或 `xdotool`
- 显示信息：`wlr-randr` / `xrandr`

#### 🔴 `writer.rs` — 需全部重写

当前使用的 macOS 专有 API：
- **AVAssetWriter** + **AVAssetWriterInput**: 硬件 HEVC 编码
- **VideoToolbox**: 通过 AVFoundation 间接使用
- **AAC 音频编码**: 通过 AVFoundation

Linux 替代方案：
- 视频编码：**GStreamer**（pipewiresrc → x264enc/vaapih264enc → mp4mux）或 **FFmpeg** 库
- 音频编码：GStreamer AAC 编码器或 FFmpeg
- 硬件加速：VA-API (Intel/AMD) 或 NVENC (NVIDIA)，但 MVP 阶段可先用软件编码

#### 🟡 `commands.rs` — 需大幅重构

**可复用部分（~40%）：**
- `RecordingStatus`, `RecordingResult`, `RecordingStartedPayload` 等序列化结构体
- 录制计时逻辑（`start_time`, `pause_accumulated`, `pause_start`）
- 所有 `#[tauri::command]` 函数签名和前端交互逻辑
- 窗口创建逻辑（`do_open_recording_bar`, `do_open_recording_overlay` 等使用 Tauri API）
- 暂停/恢复逻辑（`pause_recording`, `resume_recording`）
- 文件路径生成、目录创建

**需要 Linux 适配的部分（~60%）：**
- `RecordingState` 结构体：去掉 `Retained<SCStream>`, `Retained<AVAssetWriter>` 等 ObjC 类型，改用 Linux 录制句柄
- `do_start_recording()`: 替换 `capture::create_and_start()` 和 `writer::create_writer()` 调用
- `do_stop_recording()`: 替换 `capture::stop()` 和 `writer::finalize()` 调用
- `reveal_in_finder()`: `open -R` → `xdg-open`（已有 `#[cfg]` 框架）
- `copy_screenshot()`: `osascript` → `wl-copy` / `xclip`
- `take_screenshot()`: 替换 `capture::take_screenshot_region()` 调用
- `collect_app_windows_to_exclude()`: 移除 ObjC 窗口枚举
- `refresh_stream_exclusion()`: 移除 SCStream filter 更新

#### 🟢 `tray.rs` — 少量修改

**可复用部分（~85%）：**
- 系统托盘创建和菜单构建（Tauri 跨平台 API）
- 托盘图标加载和切换
- 设置持久化（`settings.json`）
- 自动更新检查逻辑
- 菜单事件处理框架

**需要 Linux 适配的部分（~15%）：**
- `show_confirm_dialog()` / `show_info_dialog()`: `osascript` → `zenity` 或 `kdialog`
- `open_folder` 菜单项：`open` → `xdg-open`
- 快捷键文本显示：`CmdOrCtrl` → `Ctrl`（Tauri 可能已自动处理）

#### 🟢 `lib.rs` — 几乎不变

- `set_activation_policy(Accessory)` 已有 `#[cfg(target_os = "macos")]`
- 其余全部是 Tauri 跨平台代码

---

## 二、技术方案

### Linux 截屏方案

```
XDG Desktop Portal (D-Bus)
  └─ org.freedesktop.portal.Screenshot
       └─ Screenshot() → 返回临时文件路径
       └─ 用户通过系统 Portal UI 确认权限
```

### Linux 录屏方案

```
XDG Desktop Portal (D-Bus)
  └─ org.freedesktop.portal.ScreenCast
       └─ CreateSession() → SelectSources() → Start()
       └─ 返回 PipeWire fd + node_id
  
PipeWire
  └─ 连接 fd, 从 node 读取视频帧

GStreamer Pipeline
  └─ pipewiresrc → videoconvert → x264enc → mp4mux → filesink
  └─ (可选) pulsesrc → audioconvert → faac → mp4mux
```

### Linux 依赖库（Rust crate）

| 用途 | crate | 说明 |
|------|-------|------|
| D-Bus 通信 | `zbus` | 与 XDG Portal 通信 |
| PipeWire 连接 | `pipewire` | 读取屏幕流 |
| 视频编码 | `gstreamer` + `gstreamer-app` | GStreamer Rust 绑定 |
| 截屏后处理 | `image` | PNG 读写（替代 ImageIO） |
| 剪贴板 | `arboard` 或调用 `wl-copy` | 跨平台剪贴板 |

### 系统依赖（Ubuntu 24.04 apt）

```bash
# 构建依赖
sudo apt-get install -y \
  libwebkit2gtk-4.1-dev \
  libappindicator3-dev \
  librsvg2-dev \
  patchelf \
  libgstreamer1.0-dev \
  libgstreamer-plugins-base1.0-dev \
  gstreamer1.0-plugins-good \
  gstreamer1.0-plugins-ugly \
  libpipewire-0.3-dev \
  libdbus-1-dev

# 运行时依赖
sudo apt-get install -y \
  xdg-desktop-portal \
  xdg-desktop-portal-gnome \
  gstreamer1.0-pipewire \
  pipewire \
  zenity
```

---

## 三、代码架构方案

使用 **条件编译** 在同一代码库中管理两个平台：

```
src-tauri/src/
├── lib.rs              # 入口，几乎不变
├── tray.rs             # 托盘，少量 #[cfg] 分支
├── commands.rs         # 重构为平台无关框架 + 调用 platform trait
├── platform/
│   ├── mod.rs          # 定义 trait ScreenCapture, VideoWriter
│   ├── macos/
│   │   ├── mod.rs
│   │   ├── capture.rs  # 现有 capture.rs 迁移
│   │   └── writer.rs   # 现有 writer.rs 迁移
│   └── linux/
│       ├── mod.rs
│       ├── capture.rs  # XDG Portal + PipeWire 截屏/录屏
│       ├── writer.rs   # GStreamer 编码
│       └── dialog.rs   # zenity 对话框
```

### 核心 Trait 定义（`platform/mod.rs`）

```rust
/// 平台无关的录制句柄
pub trait PlatformRecorder: Send {
    fn start(&mut self, config: RecordConfig) -> Result<(), String>;
    fn stop(&mut self) -> Result<(), String>;
    fn pause(&mut self);
    fn resume(&mut self);
    fn is_recording(&self) -> bool;
}

/// 平台无关的截屏接口
pub trait PlatformScreenshot {
    fn take_region(x: f64, y: f64, w: f64, h: f64, output: &str) -> Result<(usize, usize, u64), String>;
}

/// 平台无关的系统集成
pub trait PlatformIntegration {
    fn reveal_file(path: &str) -> Result<(), String>;
    fn copy_image_to_clipboard(path: &str) -> Result<(), String>;
    fn show_confirm_dialog(title: &str, message: &str, accept: &str, cancel: &str) -> bool;
    fn show_info_dialog(title: &str, message: &str);
    fn open_folder(path: &str) -> Result<(), String>;
}
```

### `Cargo.toml` 条件依赖

```toml
# macOS 专有依赖
[target.'cfg(target_os = "macos")'.dependencies]
objc2 = { version = "0.6", features = ["exception"] }
objc2-foundation = "0.3"
objc2-screen-capture-kit = "0.3"
objc2-core-media = "0.3"
objc2-core-video = "0.3"
objc2-av-foundation = { version = "0.3", features = [...] }
objc2-core-graphics = { version = "0.3", features = [...] }
objc2-core-foundation = "0.3"
block2 = "0.6"
dispatch2 = "0.3"

# Linux 专有依赖
[target.'cfg(target_os = "linux")'.dependencies]
zbus = "4"
gstreamer = "0.23"
gstreamer-app = "0.23"
gstreamer-video = "0.23"
pipewire = "0.8"
image = "0.25"
```

---

## 四、分阶段 TODO

### Phase 1：构建跑通 + 基础截屏（MVP） ✅ 已完成

> 预估工时：**3-5 天** → 实际 **2 天**（2026-02-17 ~ 02-19）
> 目标：应用能在 Ubuntu 24.04 上启动，托盘正常，能截屏

- [x] **1.1** 创建 `platform/` 模块结构，定义跨平台 trait
- [x] **1.2** 将现有 `capture.rs` 和 `writer.rs` 移入 `platform/macos/`
- [x] **1.3** 重构 `commands.rs`，`RecordingState` 使用 platform::imp::RecordingHandle
- [x] **1.4** `Cargo.toml` 添加条件编译依赖（macOS / Linux 分开）
- [x] **1.5** `tray.rs` 平台适配：`osascript` → `zenity`，`open` → `xdg-open`
- [x] **1.6** 实现 `platform/linux/capture.rs`：grim / gnome-screenshot / ImageMagick 截屏
- [x] **1.7** Linux 对话框（zenity/kdialog）整合到 `platform/linux/mod.rs`
- [x] **1.8** `tauri.conf.json` 添加 Linux bundle 配置（deb + AppImage）
- [x] **1.9** 创建 GitHub Actions CI：macOS + Ubuntu 双平台构建
- [x] **1.10** CI 验证通过：macOS .dmg + Ubuntu .deb/.rpm/.AppImage 全部构建成功

### Phase 2：录屏功能 ✅ 代码完成

> 预估工时：**5-7 天** → 实际代码 **1 天**（2026-02-19），待实机验证
> 目标：完整录屏功能（区域录屏 + 全屏 + 音频）
> 方案：纯子进程方案（零新 Rust crate），可在 macOS 上交叉编译

- [x] **2.1** 实现 `platform/linux/writer.rs`：gst-launch-1.0 子进程（H.264 x264enc → MP4）
- [x] **2.2** 实现 PipeWire 屏幕流接收：`portal.rs` 嵌入 Python 脚本调用 XDG Portal ScreenCast D-Bus
- [x] **2.3** 区域录屏：GStreamer videocrop 元素裁剪指定区域
- [x] **2.4** 全屏录屏：pipewiresrc → videoconvert → x264enc → mp4mux → filesink
- [x] **2.5** 暂停/恢复录屏：段式录制（SIGINT→EOS 保存段文件，resume 启动新 gst-launch）
- [x] **2.6** 系统音频捕获：pulsesrc + pactl get-default-sink monitor 源（PipeWire 兼容层）
- [x] **2.7** 麦克风捕获：pulsesrc 直接捕获默认输入设备
- [x] **2.8** GIF 转换：ffmpeg（已有跨平台逻辑可复用，无需修改）
- [x] **2.9** 录制质量选项：Standard 30fps / High 60fps + 自适应码率（5-24 Mbps）
- [x] **2.10** 录制控制条（recording-bar）和 dim overlay：前端 100% 复用，无需修改
- [ ] **2.11** 验证：区域选择 → 录制 → 暂停/恢复 → 停止 → MP4 可播放（待实机测试）

### Phase 2.5：性能优化 — 榨干 Linux 硬件性能 → 发布 v0.6.0-beta

> 预估工时：**5-8 天**（需要 Linux 实机开发环境）
> 目标：从 "能用" 升级到 "极致"，对标 macOS 版 ScreenCaptureKit + VideoToolbox 的性能水平
> 前置条件：v0.5.0-beta 发布后收集反馈，拿到 Linux 实机
> 发布策略：GitHub Release 标记为 `Pre-release`，标注 "Performance optimization beta"

**当前 MVP 方案 vs 优化后对比：**

| 维度 | MVP（子进程方案） | 优化后（Rust crate 方案） | 预期提升 |
|------|------------------|--------------------------|----------|
| Portal 交互 | python3 子进程 + D-Bus | `zbus` crate 纯 Rust D-Bus | 启动快 200ms+，去掉 python 运行时依赖 |
| 视频编码 | x264enc 纯 CPU 软编码 | VA-API (Intel/AMD) / NVENC (NVIDIA) 硬件编码 | CPU 占用降 80%+，功耗大幅降低 |
| 编码格式 | H.264 | H.265 (HEVC) via vaapih265enc | 同质量下文件体积减 40% |
| GStreamer 集成 | gst-launch-1.0 子进程 | `gstreamer-rs` crate 进程内管线 | 零拷贝数据流，延迟更低 |
| 暂停/恢复 | 停进程 + ffmpeg 段拼接 | GstPipeline 状态切换 PAUSED↔PLAYING | 无段文件 IO，无拼接开销，瞬间暂停 |
| 帧捕获 | 依赖 GStreamer pipewiresrc | `pipewire` crate 直接读帧 | 更精细的帧控制和时间戳管理 |

- [x] **2.5.1** 替换 Portal 交互：python3 子进程 → `ashpd` crate 纯 Rust D-Bus 通信（含 tokio runtime 保活）
- [x] **2.5.2** 替换 GStreamer 集成：gst-launch-1.0 子进程 → `gstreamer-rs` 进程内管线
- [x] **2.5.3** 硬件编码支持：检测 VA-API/NVENC → 优先硬件编码，fallback 到 x264
- [x] **2.5.4** HEVC (H.265) 编码：vaapih265enc / nvh265enc，对标 macOS VideoToolbox HEVC
- [x] **2.5.5** 管线内暂停/恢复：GstPipeline PAUSED↔PLAYING 状态切换，移除段拼接
- [ ] **2.5.6** PipeWire 直接集成：`pipewire` crate 替代 pipewiresrc（延后，pipewiresrc 已足够好）
- [ ] **2.5.7** 零拷贝优化：DMA-BUF 共享内存（延后，需实机验证）
- [x] **2.5.8** 自适应编码器选择：运行时探测硬件能力，自动选择最优编码路径
- [ ] **2.5.9** 性能基准测试：CPU 占用、内存、帧率、文件大小 vs macOS 版对比（待实机）
- [x] **2.5.10** Cargo.toml 更新：添加 Linux-only crate 条件依赖（ashpd, gstreamer）

**实际使用的 Rust crate 依赖（仅 Linux）：**
```toml
[target.'cfg(target_os = "linux")'.dependencies]
ashpd = { version = "0.10", default-features = false, features = ["tokio"] }  # XDG Portal（含 zbus）
gstreamer = "0.23"            # GStreamer 进程内管线
```

**延后的 crate（需实机验证后决定是否添加）：**
```toml
# pipewire = "0.8"            # PipeWire 直接集成（pipewiresrc 已足够好）
```

> ⚠️ 这些 crate 是 Linux-only 的，CI 的 Ubuntu job 需要安装对应 -dev 包。
> macOS job 不受影响（条件编译）。

### Phase 3：完善体验 → 发布 v0.5.0-beta ✅ 代码完成

> 预估工时：**2-3 天**
> 目标：功能完整的 Linux 版本，以 beta 形式发布
> 发布策略：GitHub Release 标记为 `Pre-release`，README 标注 "Linux support is experimental"
> ⚠️ 因无 Linux 实机测试，所有 Linux 功能均为 beta 状态，待社区反馈 / 实机验证后升级为 stable

- [x] **3.1** 全局快捷键适配（Tauri global-shortcut 插件，验证 Wayland 下工作情况）
- [x] **3.2** 自动更新支持（Tauri updater，Linux 端验证）
- [x] **3.3** 开机自启（`~/.config/autostart/` .desktop 文件）
- [x] **3.4** 桌面集成：`.desktop` 文件、应用图标
- [x] **3.5** Linux 端 UI 微调（字体渲染、窗口透明度在 Wayland/X11 下的表现）
- [x] **3.6** 权限引导：首次运行提示用户允许 Screen Cast 权限
- [x] **3.7** CI 产出 release artifacts（.deb + .AppImage + 更新 JSON）
- [ ] **3.8** CI 配置 `TAURI_SIGNING_PRIVATE_KEY` GitHub Secret ✅ 已配置
- [x] **3.9** README 添加 Linux 安装说明 + beta 提示
- [ ] **3.10** 测试矩阵：Ubuntu 24.04 GNOME (Wayland) + Ubuntu 24.04 GNOME (X11)（待实机验证）

---

## 五、已知风险与注意事项

| 风险 | 说明 | 应对 |
|------|------|------|
| Wayland 窗口透明度 | 部分合成器不完整支持透明窗口 | 测试 GNOME Mutter，必要时降级为半透明背景 |
| XDG Portal 权限弹窗 | 每次截屏/录屏都会弹出系统权限确认 | Portal 有 `Restore` token 机制可记住选择 |
| PipeWire 版本差异 | 不同发行版 PipeWire 版本可能不同 | 锁定 Ubuntu 24.04 版本，不追求广泛兼容 |
| 区域裁剪精度 | Wayland 下没有全局坐标系 | 使用 Portal 的 `SelectSources` 进行区域选择 |
| 硬件编码可用性 | VA-API/NVENC 不一定存在 | Phase 2 用软件编码（x264），Phase 2.5 加硬件编码 + fallback |
| 全局快捷键 | Wayland 限制后台键盘监听 | 使用 `GlobalShortcuts` Portal 或依赖托盘菜单 |
| HiDPI 缩放 | 不同缩放比例下坐标计算 | 测试 100%/125%/150%/200% 缩放 |
| 自动更新签名 | `TAURI_SIGNING_PRIVATE_KEY` 需配置到 GitHub Secrets 才能签名更新包 | Phase 3 发布前配置，当前 CI 已跳过签名步骤 |

---

## 六、工作量总结

| 阶段 | 新增代码 | 修改代码 | 预估工时 | 状态 |
|------|---------|---------|---------|------|
| Phase 1 (构建+截屏) | ~600 行 Rust + ~100 行 YAML | ~300 行重构 | 3-5 天 → 实际 2 天 | ✅ 完成 |
| Phase 2 (录屏 MVP) | ~860 行 Rust | ~30 行修改 | 5-7 天 → 实际 1 天 | ✅ 代码完成 |
| Phase 2.5 (性能优化) | ~800 行 Rust (重写) | ~400 行删除 | 5-8 天 → 实际 1 天 | ✅ 代码完成 |
| Phase 3 (完善体验) | ~200 行 Rust + 文档 | ~100 行微调 | 2-3 天 | ✅ 代码完成 |
| **合计** | **~2900-3300 行新代码** | **~1230 行重构** | **15-23 天** |

> 对比：macOS 版现有 Rust 代码约 3100 行。Linux 版最终约 3000+ 行新代码。
> Phase 2.5 是可选的性能优化阶段，不影响功能发布。
> 发布路线：Phase 2→3→**v0.5.0-beta**（Linux 首发）→实机验证→**v0.5.0**→2.5→**v0.6.0-beta**→验证→**v0.6.0**

---

## 七、验收标准

### MVP (Phase 1 完成) ✅
- [x] `pnpm tauri build` 在 Ubuntu CI 上成功（Build #1, commit 154c84b）
- [x] 产出 .deb、.rpm 和 .AppImage（额外产出了 rpm 包）
- [ ] .deb 安装后应用能启动（待实机测试）
- [ ] 系统托盘图标正常显示和交互（待实机测试）
- [ ] 截屏功能可用（区域选择 → 截图 → 保存/复制）（待实机测试）

### Linux 首发 (Phase 3 完成) → v0.5.0-beta
- [ ] 录屏功能完整（全屏/区域 + 音频 + 暂停/恢复）
- [ ] 录制文件可正常播放
- [ ] 全局快捷键可用
- [ ] 自动更新可用
- [ ] GitHub Release 标记为 Pre-release
- [ ] README 标注 "Linux support is experimental — feedback welcome!"
- [ ] 产出 .deb + .AppImage 供下载

### Linux 稳定版 → v0.5.0
- [ ] v0.5.0-beta 收到社区反馈 / 实机验证
- [ ] 修复实机测试发现的问题
- [ ] GNOME Wayland 和 X11 下均正常工作
- [ ] 移除 beta 标记，升级为 Latest release

### 极致性能版 (Phase 2.5 完成) → v0.6.0-beta
- [ ] 硬件编码可用（VA-API 或 NVENC，根据用户硬件自动选择）
- [ ] 录制时 CPU 占用 < 10%（对比 MVP 软编码 ~30-50%）
- [ ] HEVC 输出：同画质文件体积比 H.264 减少 30%+
- [ ] 暂停/恢复无缝切换（无段文件拼接）
- [ ] 无 python3 运行时依赖（纯 Rust D-Bus）
- [ ] 性能基准：与 macOS 版 ScreenCaptureKit + VideoToolbox 对比在同一量级
