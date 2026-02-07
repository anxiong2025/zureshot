# Zureshot 开发计划

> 核心指标: Retina 物理像素 60fps 录制 | 内存 < 50MB (引擎) | 零丢帧 | M 系列极致优化
> 目标平台: Apple Silicon (M1-M4 全系列) | macOS 13+

---

## 当前进度总览

```
Phase 0: 技术栈 ████████████████████ 100%  ✅ 已超越原方案
Phase 1: 录制核心 ████████████████████ 100%  ✅ 零拷贝 HEVC 管线
Phase 2: GUI      ████████████████████ 100%  ✅ 区域录制 + 控制条
Phase 3: 粗剪     ░░░░░░░░░░░░░░░░░░░░   0%  ⬜ 待开发
Phase 4: LUT 滤镜 ░░░░░░░░░░░░░░░░░░░░   0%  ⬜ 待开发
```

---

## ✅ Phase 0: 技术栈 (已完成)

### Task 5: 技术栈

- [x] objc2 全家桶: screen-capture-kit 0.3, core-media 0.3, av-foundation 0.3
- [x] 音频采集: SCK 原生 audio output (无需 cpal)
- [x] 线程通信: std::sync::mpsc + AtomicBool (无需 crossbeam)
- [x] 编码器: **HEVC (H.265)** 默认 (H.264 兼容性问题已不存在——所有主流平台均支持)
- [x] objc2 0.6 + block2 0.6 + dispatch2 0.3 FFI 层

实际依赖 (远比原方案精简):

```toml
objc2 = { version = "0.6", features = ["exception"] }
objc2-screen-capture-kit = "0.3"
objc2-core-media = "0.3"
objc2-av-foundation = { version = "0.3", features = ["AVVideoSettings", "AVAssetWriter", "AVAssetWriterInput"] }
objc2-core-graphics = { version = "0.3", features = ["CGColorSpace"] }
block2 = "0.6"
dispatch2 = "0.3"
```

### Task 6: ObjC 桥接安全

- [x] objc2 类型安全桥接 (非手写 FFI，编译期检查)
- [x] CMSampleBuffer 生命周期由 SCK 回调管理，直接 appendSampleBuffer 零拷贝
- [x] define_class! 宏定义 SCStreamOutput 委托，避免手写 ObjC 类
- [x] catch_objc() 异常捕获防止 ObjC 异常导致进程终止
- [x] 32 秒以上录制稳定测试通过，零丢帧

> **原风险「高」→ 实际「已解决」**：objc2 的 Retained<T> 自动管理 retain/release，
> 加上 SCK 的 CMSampleBuffer 回调模型天然避免了手动内存管理。

---

## ✅ Phase 1: 录制核心 (已完成)

### Task 1: 全屏/区域 HEVC 录制

- [x] ScreenCaptureKit 捕获屏幕 (SCStreamOutput 委托)
- [x] VideoToolbox **HEVC (H.265) Main AutoLevel** 硬件编码
- [x] AVAssetWriter 写入 MP4 容器 (moov atom 正确写入)
- [x] NV12 (`420v`) 像素格式——编码器原生格式，零色彩转换
- [x] **SCCaptureResolutionType::Best** 强制物理像素采集
- [x] **sRGB 色彩空间 + BT.709 三件套** (Primaries/Transfer/Matrix)
- [x] Queue depth 3 帧，内存增量 ~30-50 MB
- [x] PTS 单调性交叉乘法检查
- [x] 系统音频 + 麦克风双轨 AAC 48kHz 128kbps
- [x] 暂停/恢复 (AtomicBool 零开销丢帧)
- [x] 验证: CPU < 3%, 零丢帧, 60fps 稳定

**实际性能 (M4, 3200×2132 Retina, 60fps HEVC):**

| 指标 | 原方案目标 | 实际达成 |
|------|----------|----------|
| CPU 占用 | < 10% | **< 3%** |
| 内存增量 | < 250 MB | **~30-50 MB** |
| 丢帧率 | 允许少量 | **0 丢帧** |
| 编码 | H.264 | **HEVC H.265** (更小文件) |
| 码率 | 6-25 Mbps | **8-36 Mbps** (自适应) |
| 色彩 | 无要求 | **BT.709 全链路** |
| 分辨率 | 逻辑像素 | **物理像素** (真 Retina) |

---

## ✅ Phase 2: Tauri GUI + 区域选择 (已完成)

### Task 2: Tauri 壳与区域录制

- [x] Tauri v2 项目结构 (Svelte 5 + Vite)
- [x] Menu bar tray 图标 + 下拉菜单
  - 📹 全屏录制 (Standard / High Quality)
  - 🔲 区域录制
  - 🔊 系统音频开关
  - 🎤 麦克风开关
  - ⏹ 停止录制
  - ⏸ 暂停/恢复
  - 📂 打开录制目录
  - Finder 中显示
- [x] 全屏透明 overlay 窗口 (RegionSelector.svelte)
- [x] 可拖拽选区矩形框，输出 CSS 坐标 (x, y, w, h)
- [x] Retina 屏幕坐标缩放 (CGDisplayModeGetPixelWidth 物理像素)
- [x] 选区坐标传递给 SCK sourceRect 实现区域录制
- [x] 录制中控制条 (RecordingBar.svelte) + 暗化遮罩 (RecordingOverlay.svelte)
- [x] 自动排除 App 自身窗口 (PID 匹配 + 动态 filter 更新)
- [x] 快捷键: `⌘⇧R` 录制, `⌘⇧A` 区域选择
- [x] macOS 录屏权限处理 + 友好错误提示

---

## ⬜ Phase 3: 粗剪 + 导出 (待开发)

### Task 3: 首尾修剪与导出

- [ ] 前端: 录制结束后弹出预览窗口
- [ ] 前端: Range Slider 拖动选择 Start / End 时间点
- [ ] 后端: FFmpeg sidecar 或原生 AVAssetExportSession 实现修剪
- [ ] Stream copy 极速导出 (只剪首尾不重编码)
- [ ] 可选: 4K → 1080p 硬件加速转码

---

## ⬜ Phase 4: LUT 滤镜 (待开发)

### Task 4: 实时 LUT 渲染

- [ ] 解析 .cube 文件
- [ ] Core Image `CIColorCubeWithColorSpace` 滤镜 (最简方案)
- [ ] 或 Metal Compute Shader (高性能方案)
- [ ] 滤镜后的纹理送入编码器

---

## ⬜ Phase 5: 体验打磨 + 导出格式

- [ ] 多显示器选择
- [ ] 录制停止时缩略图预览
- [ ] 全局设置面板 (输出路径、格式、画质)
- [ ] 导出后自动在 Finder 中打开
- [ ] 录制倒计时 (3-2-1)
- [ ] GIF 导出 (调色板优化，最高 30fps)
- [ ] WebM / VP9 导出
- [ ] 截图模式 (全屏 / 区域)
- [ ] 复制到剪贴板

---

## ⬜ Phase 6: 标注与覆盖层

- [ ] 屏幕标注: 箭头、矩形、文字
- [ ] 高亮 / 聚光灯效果 (光标区域外自动暗化)
- [ ] 摄像头画中画 (圆形浮窗)
- [ ] 自定义水印

---

## 🔭 Phase 7: 演示模式 (远景目标)

> **核心理念**: 让 Zureshot 从「录屏工具」进化为「演示录制神器」
> 对标产品: Screen Studio / Loom 的缩放效果，但完全开源、完全本地

### 自动缩放 (Auto Zoom)

- [ ] 实时追踪光标位置
- [ ] 光标移动时摄像机平滑跟随，自动放大聚焦区域
- [ ] 可配置缩放倍率 (1.5x / 2x / 3x)
- [ ] 可配置缓动曲线 (ease-in-out / spring / linear)
- [ ] 智能检测: 光标静止时不缩放，快速移动时延迟跟随

技术路线:
```
SCK 原始帧 (全分辨率)
       │
       ▼
Metal Compute Shader / Core Image
  ┌─ 根据光标坐标计算 viewport
  ├─ 对 IOSurface 做 GPU crop + scale
  └─ 输出新的纹理 → VideoToolbox 编码
```

### 点击涟漪 (Click Ripple)

- [ ] 鼠标点击时在点击位置绘制涟漪动画
- [ ] Metal 渲染或 Core Animation overlay
- [ ] 左键 / 右键 / 双击不同样式
- [ ] 可配置颜色和持续时间

### 按键显示 (Keystroke Overlay)

- [ ] 实时捕获键盘输入 (Accessibility API)
- [ ] 在屏幕底部/角落显示按键组合
- [ ] 特别适合快捷键演示场景
- [ ] 自动消失动画

### 聚光灯模式 (Spotlight)

- [ ] 光标周围保持明亮 (可配置半径)
- [ ] 其余区域实时高斯模糊 + 暗化
- [ ] GPU 实时渲染，零性能影响

### 平滑跟随 (Smooth Pan)

- [ ] 电影级镜头移动效果
- [ ] 弹簧物理模型 (spring dynamics)
- [ ] 防抖: 小幅移动过滤，大幅移动跟随
- [ ] 可调灵敏度和阻尼系数

### 场景预设 (Scene Presets)

- [ ] 保存多个缩放级别 / 聚焦区域
- [ ] 快捷键快速切换场景
- [ ] 适合分步骤演示

---

## 🌐 Phase 8: 远期展望

- [ ] 实时 LUT / 色彩滤镜 (Core Image 或 Metal Compute)
- [ ] 自动上传云端 (S3、R2、自定义端点)
- [ ] 插件系统，支持自定义后处理
- [ ] Apple 快捷指令集成
- [ ] 菜单栏录制指示器 + 实时音频波形
- [ ] 分享链接生成 (类似 Loom)
- [ ] AI 智能剪辑建议

---

## 原方案修正记录

| # | 原方案 | 实际实现 | 原因 |
|---|--------|---------|------|
| 1 | 内存 < 250MB | **< 50MB** | 零拷贝管线 + IOSurface 驻留 GPU |
| 2 | 默认 H.264 | **默认 HEVC** | 2024+ 所有主流平台已支持 H.265 |
| 3 | wgpu / Metal | **不需要** | SCK→VideoToolbox 直通，无需 GPU 合成 |
| 4 | FFmpeg sidecar | **AVAssetWriter** | 原生 Apple API 更轻量，零依赖 |
| 5 | videotoolbox-rs | **objc2-av-foundation** | 通过 AVAssetWriter 间接使用 VideoToolbox |
| 6 | fMP4 fragment | **标准 MP4 + finalize** | AVAssetWriter 自动处理 moov atom |
| 7 | cpal 音频 | **SCK 原生音频** | ScreenCaptureKit 直接支持系统音频+麦克风 |
| 8 | crossbeam + parking_lot | **std::sync** | 标准库足够，无需额外依赖 |
| 9 | Ring Buffer 丢帧 | **零丢帧** | SCK + VTB 管线足够快，不需要丢帧策略 |
