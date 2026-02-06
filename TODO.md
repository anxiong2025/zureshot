# Zureshot 开发计划

> 核心指标: 1080p/4K 60fps 录制 | 内存 < 250MB | 零崩溃 | 极速导出
> 目标平台: Apple Silicon (M 系列) | 后期扩展 Windows

---

## 依赖关系

```
#5 修正技术栈 ─┐
               ├─► #1 录制核心 ─┬─► #2 Tauri GUI ─► #3 粗剪导出
#6 ObjC 安全方案┘               └─► #4 LUT 滤镜
```

---

## Phase 0: 前置工作

### Task 5: 修正技术栈 — 更新 Cargo.toml 依赖

- [ ] 使用 objc2 全家桶: screen-capture-kit, core-media, core-video, video-toolbox, metal
- [ ] 添加 cpal 音频捕获
- [ ] 添加 crossbeam-channel (线程通信) + parking_lot (高性能锁)
- [ ] 移除不存在的 `videotoolbox-rs`，MVP 不引入 wgpu
- [ ] 默认编码改为 H.264 (兼容性优先)，高级选项提供 H.265

参考依赖:

```toml
[dependencies]
tauri = { version = "2", features = ["macos-private-api"] }
objc2 = "0.6"
objc2-screen-capture-kit = "0.2"
objc2-core-media = "0.2"
objc2-core-video = "0.2"
objc2-video-toolbox = "0.2"
objc2-metal = "0.2"
cpal = "0.15"
crossbeam-channel = "0.5"
parking_lot = "0.12"
```

### Task 6: ObjC 桥接内存安全方案

- [ ] 封装 RAII wrapper 确保 CFRetain/CFRelease 配对
- [ ] Ring Buffer 中正确管理引用计数
- [ ] 编写压力测试验证长时间录制无泄漏无悬垂引用

> **风险等级: 高** — CMSampleBuffer 生命周期管理是零崩溃目标的最大威胁

---

## Phase 1: 录制核心 (无 GUI)

### Task 1: 纯 Rust CLI 全屏录制

- [ ] 调用 ScreenCaptureKit 捕获屏幕
- [ ] VideoToolbox H.264 硬件编码
- [ ] 写入 fMP4 容器 (每 2-4 秒一个 fragment，崩溃安全)
- [ ] 实现 Ring Buffer (3-5 帧容量，背压时丢弃旧帧)
- [ ] 验证: CPU < 10%, 内存 < 250MB, 输出文件可播放

关键技术点:

- **Zero-Copy 主通路**: ScreenCaptureKit -> CMSampleBuffer 引用 -> VideoToolbox，不在 CPU 侧复制像素数据
- **VBR 码率参数**:
  - 1080p: AverageBitRate 6-8 Mbps, 峰值 15 Mbps
  - 4K: AverageBitRate 20-25 Mbps, 峰值 50 Mbps
  - Quality: 0.7-0.8
- **关键帧间隔**: MaxKeyFrameInterval = 60 (1 秒 1 个关键帧，为粗剪精度服务)

---

## Phase 2: Tauri GUI + 区域选择

### 应用交互模型 (类 CleanShot X)

应用启动后 **无主窗口**，仅在 macOS 顶部菜单栏显示一个 tray 小图标。用户点击图标弹出下拉菜单:

```
┌──────────────────────┐
│  📹  录制视频         │  ← 全屏 / 区域录制
│  📸  截图             │  ← 全屏 / 区域截图
│  ─────────────────── │
│  📂  打开录制目录      │
│  ⚙️  设置             │
│  ─────────────────── │
│  退出 Zureshot       │
└──────────────────────┘
```

**交互流程:**

1. 用户点击「录制视频」→ 进入区域选择 overlay → 拖拽选区 → 开始录制 → 菜单栏图标变为录制状态 (红点/计时)
2. 用户点击菜单栏图标或快捷键 → 停止录制 → 弹出预览/粗剪窗口
3. 用户点击「截图」→ 进入区域选择 overlay → 拖拽选区 → 完成截图 → 复制到剪贴板 / 保存

**录制格式路线图:**

| 阶段 | 格式 | 说明 |
|------|------|------|
| MVP | MP4 (H.264) | 视频录制，兼容性优先 |
| 后期 | GIF | 短循环动图录制，适合分享 |
| 后期 | MP4 (H.265) | 高级选项，更小体积 |

> **Tauri 实现要点**: 使用 `tauri::tray::TrayIconBuilder` 创建系统托盘图标，应用设置 `"macOSPrivateApi": true` 以支持透明 overlay 窗口。

### Task 2: Tauri 壳与区域录制

- [ ] 搭建 Tauri v2 项目结构
- [ ] 创建 menu bar tray 图标 + 下拉菜单 (录制视频 / 截图 / 设置 / 退出)
- [ ] 全屏透明 overlay 窗口
- [ ] 绘制可拖拽的选区矩形框，输出坐标 (x, y, w, h)
- [ ] 处理 Retina 屏幕坐标缩放 (Scale Factor)
- [ ] 将选区坐标传递给 ScreenCaptureKit 实现区域录制
- [ ] 录制状态下菜单栏图标状态切换 (空闲 / 录制中)
- [ ] macOS 录屏权限弹窗 + TCC 交互处理

---

## Phase 3: 粗剪 + 导出

### Task 3: 首尾修剪与导出

- [ ] 前端: 录制结束后弹窗预览，加载本地临时文件
- [ ] 前端: Range Slider 拖动选择 Start / End 时间点
- [ ] 后端: FFmpeg sidecar 打包 (不用 ffmpeg-next 绑定库)

导出策略:

| 场景 | 条件 | 方式 | 速度 |
|------|------|------|------|
| A 极速导出 | 只剪首尾，不改分辨率 | stream copy (`-c copy`) | 1h 视频 ~2-3s |
| B 渲染导出 | 需要转分辨率 (4K->1080p) | 硬件加速转码 | >5x 实时速度 |

> **注意**: stream copy 只能在关键帧处切割。Phase 1 中已设置 1 秒关键帧间隔来缓解精度问题。如需帧级精度，对首尾各 1-2 秒做局部重编码 (smart cut)，中间段 stream copy。

---

## Phase 4: LUT 滤镜

### Task 4: 实时 LUT 渲染

- [ ] 解析 .cube 文件，加载为 GPU 3D Texture
- [ ] 编写 Metal Compute Shader (~30 行): 输入截屏纹理 -> 查表替换颜色 -> 输出滤镜纹理
- [ ] 滤镜后的纹理送入 VideoToolbox 编码器
- [ ] 验证: GPU 占用率极低，不影响录制帧率

> **MVP 阶段直接用 Metal，不引入 wgpu。** 后期 Windows 版再考虑 wgpu 跨平台。
>
> **备选方案**: Core Image `CIColorCubeWithColorSpace` 滤镜，3 行代码搞定，零配置走 GPU。

---

## 风险矩阵

| 风险 | 等级 | 说明 |
|------|------|------|
| ObjC 桥接内存安全 | **高** | CMSampleBuffer 生命周期管理是崩溃主要来源 |
| VideoToolbox 封装 | **高** | 无成熟 crate，需大量手写 FFI |
| ScreenCaptureKit 权限 | 中 | macOS 录屏权限弹窗 + TCC 数据库交互 |
| FFmpeg 分发 | 中 | 用 Tauri sidecar 打包 ffmpeg 二进制 |
| H.265 兼容性 | 低 | 默认 H.264 规避，H.265 作为高级选项 |

---

## 原方案修正记录

1. **内存指标**: 50-150MB -> **< 250MB** (4K 下 200MB 是正常的)
2. **默认编码**: H.265 -> **H.264** (自媒体平台兼容性优先)
3. **GPU 方案**: wgpu -> **Metal 直接写** (减少抽象层，MVP 更简单)
4. **FFmpeg 集成**: ffmpeg-next 绑定 -> **sidecar 命令行调用** (分发更简单)
5. **videotoolbox-rs**: 无稳定发布 -> **objc2 手写封装**
6. **fMP4 说明**: 不需要"重新封装"，只需 finalize 写入 moov/mfra atom
