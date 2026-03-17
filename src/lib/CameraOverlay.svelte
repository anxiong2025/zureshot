<script>
  import { invoke } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import { LogicalPosition, LogicalSize } from '@tauri-apps/api/dpi';

  // ─── Constants ───
  const MIN_SIZE = 80;
  const MAX_SIZE = 500;
  const RESIZE_EDGE = 12;
  const PADDING = 16;

  // Size presets
  const SIZE_MAP = { small: 100, medium: 180, large: 260, huge: 360 };

  // ─── Read initial settings from URL params ───
  const params = new URLSearchParams(window.location.search);
  const initShape = params.get('shape') || 'circle';
  const initSize = params.get('size') || 'medium';
  const initDeviceId = params.get('deviceId') || '';
  const boundXParam = Number.parseFloat(params.get('boundX') || '');
  const boundYParam = Number.parseFloat(params.get('boundY') || '');
  const boundWParam = Number.parseFloat(params.get('boundW') || '');
  const boundHParam = Number.parseFloat(params.get('boundH') || '');

  let boundX = Number.isFinite(boundXParam) ? boundXParam : 0;
  let boundY = Number.isFinite(boundYParam) ? boundYParam : 0;
  let boundW = Number.isFinite(boundWParam) && boundWParam > 0 ? boundWParam : window.screen.width;
  let boundH = Number.isFinite(boundHParam) && boundHParam > 0 ? boundHParam : window.screen.height;

  // ─── State ───
  let videoEl = $state(null);
  let stream = $state(null);
  let cameraReady = $state(false);
  let cameraError = $state('');
  let shape = $state(initShape);           // 'circle' | 'square' | 'rectangle' | 'vertical'
  let contentW = $state(0);
  let contentH = $state(0);
  let isHovered = $state(false);
  let isMirrored = $state(true);
  let deviceId = $state(initDeviceId);

  // Native camera fallback state
  let useNativeCamera = $state(false);
  let nativeFrameSrc = $state('');
  let nativeFrameUnlisten = null;

  // Resize state
  let isResizing = $state(false);
  let resizeStartW = 0;
  let resizeStartH = 0;
  let resizeStartX = 0;
  let resizeStartY = 0;

  // Drag state
  let isDragging = $state(false);

  // Derive border-radius from shape
  let borderRadius = $derived(
    shape === 'circle' ? '50%' :
    shape === 'square' ? '16px' :
    '16px'
  );

  // Initialize size from preset
  function initDimensions() {
    const base = SIZE_MAP[initSize] || 180;
    if (shape === 'circle' || shape === 'square') {
      contentW = base;
      contentH = base;
    } else if (shape === 'rectangle') {
      contentW = Math.round(base * 1.5);
      contentH = base;
    } else if (shape === 'vertical') {
      contentW = base;
      contentH = Math.round(base * 1.5);
    }
  }
  initDimensions();

  // ─── Camera setup ───
  async function startCamera() {
    try {
      const constraints = {
        video: {
          width: { ideal: 640 },
          height: { ideal: 640 },
          facingMode: 'user',
          frameRate: { ideal: 30, max: 30 },
        },
        audio: false,
      };
      // Use specific device if provided
      if (deviceId) {
        constraints.video.deviceId = { exact: deviceId };
        delete constraints.video.facingMode;
      }

      stream = await navigator.mediaDevices.getUserMedia(constraints);

      if (videoEl) {
        videoEl.srcObject = stream;
        await videoEl.play();
        cameraReady = true;
        console.log('[camera-overlay] Camera started');
      }
    } catch (e) {
      console.error('[camera-overlay] getUserMedia failed, trying native camera:', e);
      // Fallback to native camera capture (for Continuity Camera etc.)
      await startNativeCamera();
    }
  }

  // ─── Native camera fallback ───
  async function startNativeCamera() {
    try {
      console.log('[camera-overlay] Starting native camera, deviceId:', deviceId);
      await invoke('start_native_camera', { deviceId: deviceId || '' });
      useNativeCamera = true;
      cameraError = '';

      // Listen for native frames
      nativeFrameUnlisten = await listen('camera-native-frame', (event) => {
        nativeFrameSrc = event.payload;
        if (!cameraReady) {
          cameraReady = true;
          console.log('[camera-overlay] Native camera started (first frame received)');
        }
      });
    } catch (e) {
      console.error('[camera-overlay] Native camera error:', e);
      cameraError = e.message || 'Camera not available';
    }
  }

  async function stopNativeCamera() {
    if (nativeFrameUnlisten) {
      nativeFrameUnlisten();
      nativeFrameUnlisten = null;
    }
    if (useNativeCamera) {
      try {
        await invoke('stop_native_camera');
      } catch {}
      useNativeCamera = false;
      nativeFrameSrc = '';
    }
  }

  function stopCamera() {
    if (stream) {
      stream.getTracks().forEach(t => t.stop());
      stream = null;
    }
    stopNativeCamera();
    cameraReady = false;
  }

  // ─── Window resize helper ───
  function clampPosition(x, y) {
    const minX = boundX + 4;
    const minY = boundY + 4;
    const maxX = Math.max(minX, boundX + boundW - (contentW + PADDING) - 4);
    const maxY = Math.max(minY, boundY + boundH - (contentH + PADDING) - 4);
    return {
      x: Math.min(Math.max(x, minX), maxX),
      y: Math.min(Math.max(y, minY), maxY),
    };
  }

  async function clampWindowToBounds() {
    try {
      const win = getCurrentWindow();
      const scale = await win.scaleFactor();
      const pos = await win.outerPosition();
      const logicalX = pos.x / scale;
      const logicalY = pos.y / scale;
      const clamped = clampPosition(logicalX, logicalY);
      if (Math.abs(clamped.x - logicalX) > 0.5 || Math.abs(clamped.y - logicalY) > 0.5) {
        await win.setPosition(new LogicalPosition(clamped.x, clamped.y));
      }
    } catch {}
  }

  async function updateWindowSize() {
    const win = getCurrentWindow();
    await win.setSize(new LogicalSize(contentW + PADDING, contentH + PADDING));
    await clampWindowToBounds();
  }

  // ─── Resize by scroll wheel ───
  function onWheel(e) {
    e.preventDefault();
    const scale = -e.deltaY * 0.5;
    const aspectRatio = contentW / contentH;
    let newW = Math.round(Math.max(MIN_SIZE, Math.min(MAX_SIZE, contentW + scale)));
    let newH = Math.round(newW / aspectRatio);
    // Clamp height too
    if (newH > MAX_SIZE) { newH = MAX_SIZE; newW = Math.round(newH * aspectRatio); }
    if (newH < MIN_SIZE) { newH = MIN_SIZE; newW = Math.round(newH * aspectRatio); }
    if (newW !== contentW || newH !== contentH) {
      contentW = newW;
      contentH = newH;
      updateWindowSize();
    }
  }

  // ─── Resize by edge drag ───
  function isOnEdge(e) {
    const r = e.currentTarget.getBoundingClientRect();
    const insetL = e.clientX - r.left;
    const insetR = r.right - e.clientX;
    const insetT = e.clientY - r.top;
    const insetB = r.bottom - e.clientY;
    return Math.min(insetL, insetR, insetT, insetB) < RESIZE_EDGE;
  }

  function onMouseDown(e) {
    if (e.button !== 0) return;
    if (isOnEdge(e)) {
      e.preventDefault();
      e.stopPropagation();
      isResizing = true;
      resizeStartW = contentW;
      resizeStartH = contentH;
      resizeStartX = e.screenX;
      resizeStartY = e.screenY;
      window.addEventListener('mousemove', onResizeMove);
      window.addEventListener('mouseup', onResizeUp);
      return;
    }

    if (e.target.closest('.controls')) return;

    startDrag(e);
  }

  const SNAP_DISTANCE = 15; // px threshold for magnetic snap

  async function snapToEdge() {
    try {
      const win = getCurrentWindow();
      const scale = await win.scaleFactor();
      const pos = await win.outerPosition();
      const size = await win.outerSize();
      let x = pos.x / scale;
      let y = pos.y / scale;
      const w = size.width / scale;
      const h = size.height / scale;
      const screenW = boundX + boundW;
      const screenH = boundY + boundH;

      let snapped = false;
      // Snap left
      if (Math.abs(x - boundX) < SNAP_DISTANCE) { x = boundX + 4; snapped = true; }
      // Snap right
      if (Math.abs((x + w) - screenW) < SNAP_DISTANCE) { x = screenW - w - 4; snapped = true; }
      // Snap top
      if (Math.abs(y - boundY) < SNAP_DISTANCE) { y = boundY + 4; snapped = true; }
      // Snap bottom
      if (Math.abs((y + h) - screenH) < SNAP_DISTANCE) { y = screenH - h - 4; snapped = true; }

      if (snapped) {
        await win.setPosition(new LogicalPosition(x, y));
      }

      // Persist position
      savePosition(x, y);
    } catch {}
  }

  function savePosition(x, y) {
    try {
      localStorage.setItem('camera-pos', JSON.stringify({ x, y, w: contentW, h: contentH, shape }));
    } catch {}
  }

  function loadPosition() {
    try {
      const saved = localStorage.getItem('camera-pos');
      if (saved) {
        const p = JSON.parse(saved);
        return p;
      }
    } catch {}
    return null;
  }

  async function startDrag(e) {
    if (isResizing) return;
    e.preventDefault();
    e.stopPropagation();

    isDragging = true;
    try {
      await getCurrentWindow().startDragging();
    } catch (err) {
      console.error('[camera-overlay] startDragging failed:', err);
    } finally {
      isDragging = false;
      // After drag ends, snap to edges and save position
      await snapToEdge();
    }
  }

  function onResizeMove(e) {
    if (!isResizing) return;
    const dx = e.screenX - resizeStartX;
    const dy = e.screenY - resizeStartY;
    const delta = Math.abs(dx) > Math.abs(dy) ? dx : dy;
    const aspectRatio = resizeStartW / resizeStartH;
    let newW = Math.round(Math.max(MIN_SIZE, Math.min(MAX_SIZE, resizeStartW + delta)));
    let newH = Math.round(newW / aspectRatio);
    if (newH > MAX_SIZE) { newH = MAX_SIZE; newW = Math.round(newH * aspectRatio); }
    if (newH < MIN_SIZE) { newH = MIN_SIZE; newW = Math.round(newH * aspectRatio); }
    if (newW !== contentW || newH !== contentH) {
      contentW = newW;
      contentH = newH;
      updateWindowSize();
    }
  }

  function onResizeUp() {
    isResizing = false;
    window.removeEventListener('mousemove', onResizeMove);
    window.removeEventListener('mouseup', onResizeUp);
  }

  // ─── Close ───
  async function closeBubble() {
    stopCamera();
    try {
      await invoke('close_camera_overlay');
    } catch {
      const win = getCurrentWindow();
      win.destroy();
    }
  }

  // ─── Lifecycle ───
  $effect(() => {
    startCamera();
    getCurrentWindow().setIgnoreCursorEvents(false).catch(() => {});

    // Restore saved position
    const saved = loadPosition();
    if (saved && Number.isFinite(saved.x) && Number.isFinite(saved.y)) {
      getCurrentWindow().setPosition(new LogicalPosition(saved.x, saved.y)).catch(() => {});
    } else {
      clampWindowToBounds();
    }

    return () => {
      stopCamera();
      onResizeUp();
    };
  });

  // Listen for close event from Rust side
  listen('camera-overlay-close', () => {
    stopCamera();
    getCurrentWindow().destroy();
  });

  // Listen for settings update (when user changes options while camera is open)
  listen('camera-overlay-settings', async (event) => {
    const s = event.payload;
    if (s.shape) shape = s.shape;

    if (
      Number.isFinite(s.bound_x) &&
      Number.isFinite(s.bound_y) &&
      Number.isFinite(s.bound_w) &&
      Number.isFinite(s.bound_h) &&
      s.bound_w > 0 &&
      s.bound_h > 0
    ) {
      const dx = s.bound_x - boundX;
      const dy = s.bound_y - boundY;
      boundX = s.bound_x;
      boundY = s.bound_y;
      boundW = s.bound_w;
      boundH = s.bound_h;

      if (!isDragging && !isResizing) {
        try {
          const win = getCurrentWindow();
          const scale = await win.scaleFactor();
          const pos = await win.outerPosition();
          const logicalX = pos.x / scale + dx;
          const logicalY = pos.y / scale + dy;
          const clamped = clampPosition(logicalX, logicalY);
          await win.setPosition(new LogicalPosition(clamped.x, clamped.y));
        } catch {}
      }
    }

    if (s.device_id && s.device_id !== deviceId) {
      deviceId = s.device_id;
      stopCamera();
      useNativeCamera = false;
      startCamera();
    }
    if (s.size) {
      const base = SIZE_MAP[s.size] || 180;
      if (shape === 'circle' || shape === 'square') {
        contentW = base; contentH = base;
      } else if (shape === 'rectangle') {
        contentW = Math.round(base * 1.5); contentH = base;
      } else if (shape === 'vertical') {
        contentW = base; contentH = Math.round(base * 1.5);
      }
      updateWindowSize();
    }
  });
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="camera-bubble"
  class:resizing={isResizing}
  style="width:{contentW + PADDING}px; height:{contentH + PADDING}px;"
  onmouseenter={() => isHovered = true}
  onmouseleave={() => isHovered = false}
  onwheel={onWheel}
  onmousedown={onMouseDown}
>
  <!-- Video container (shape controlled by border-radius) -->
  <div class="video-container" style="width:{contentW}px; height:{contentH}px; border-radius:{borderRadius};">
    {#if cameraError}
      <div class="error-state">
        <svg width="32" height="32" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
          <path d="M15.182 15.182a4.5 4.5 0 01-6.364 0M21 12a9 9 0 11-18 0 9 9 0 0118 0zM9.75 9.75c0 .414-.168.75-.375.75S9 10.164 9 9.75 9.168 9 9.375 9s.375.336.375.75zm-.375 0h.008v.015h-.008V9.75zm5.625 0c0 .414-.168.75-.375.75s-.375-.336-.375-.75.168-.75.375-.75.375.336.375.75zm-.375 0h.008v.015h-.008V9.75z"/>
        </svg>
        <span class="error-text">No Camera</span>
      </div>
    {:else if useNativeCamera}
      <!-- Native camera: display frames as img -->
      {#if nativeFrameSrc}
        <img
          src={nativeFrameSrc}
          alt="Camera"
          class="camera-video"
          class:mirrored={isMirrored}
          class:ready={cameraReady}
        />
      {:else}
        <div class="loading-state">
          <div class="spinner"></div>
        </div>
      {/if}
    {:else}
      <!-- svelte-ignore a11y_media_has_caption -->
      <video
        bind:this={videoEl}
        class="camera-video"
        class:mirrored={isMirrored}
        class:ready={cameraReady}
        autoplay
        playsinline
        muted
      ></video>

      {#if !cameraReady}
        <div class="loading-state">
          <div class="spinner"></div>
        </div>
      {/if}
    {/if}
  </div>

  <!-- Hover controls -->
  {#if isHovered && !isResizing}
    <div class="controls">
      <button class="ctrl-btn ctrl-close" onclick={closeBubble} title="Close camera">
        <svg width="10" height="10" viewBox="0 0 10 10" fill="none">
          <path d="M1 1L9 9M9 1L1 9" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/>
        </svg>
      </button>
      <button class="ctrl-btn ctrl-mirror" class:active={isMirrored} onclick={() => isMirrored = !isMirrored} title="Flip Camera">
        <svg width="12" height="12" viewBox="0 0 12 12" fill="none">
          <path d="M6 1v10M2 3l2.5 3L2 9M10 3L7.5 6 10 9" stroke="currentColor" stroke-width="1" stroke-linecap="round" stroke-linejoin="round"/>
        </svg>
      </button>
    </div>
    <div class="size-hint">{contentW}×{contentH}</div>
  {/if}
</div>

<style>
  /* ─── Bubble container ─── */
  .camera-bubble {
    position: relative;
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: move;
    -webkit-app-region: no-drag;
    user-select: none;
    -webkit-user-select: none;
  }

  .camera-bubble.resizing {
    cursor: nwse-resize;
    -webkit-app-region: no-drag;
  }

  /* ─── Video container ─── */
  .video-container {
    overflow: hidden;
    background: rgba(0, 0, 0, 0.85);
    border: 2.5px solid rgba(255, 255, 255, 0.2);
    box-shadow:
      0 8px 32px rgba(0, 0, 0, 0.5),
      0 2px 8px rgba(0, 0, 0, 0.3),
      inset 0 0 0 1px rgba(255, 255, 255, 0.05);
    position: relative;
    transition: border-color 0.2s ease;
  }

  .camera-bubble:hover .video-container {
    border-color: rgba(255, 255, 255, 0.35);
  }

  .camera-video {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
    opacity: 0;
    transition: opacity 0.3s ease;
  }

  .camera-video.ready {
    opacity: 1;
  }

  .camera-video.mirrored {
    transform: scaleX(-1);
  }

  /* ─── Error state ─── */
  .error-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    height: 100%;
    color: rgba(255, 255, 255, 0.5);
    gap: 6px;
  }

  .error-text {
    font-size: 10px;
    font-weight: 500;
    opacity: 0.7;
  }

  /* ─── Loading state ─── */
  .loading-state {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .spinner {
    width: 24px;
    height: 24px;
    border: 2px solid rgba(255, 255, 255, 0.15);
    border-top-color: rgba(255, 255, 255, 0.6);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  /* ─── Hover controls ─── */
  .controls {
    position: absolute;
    top: 4px;
    right: 4px;
    display: flex;
    flex-direction: column;
    gap: 4px;
    z-index: 10;
    -webkit-app-region: no-drag;
  }

  .ctrl-btn {
    width: 24px;
    height: 24px;
    border-radius: 50%;
    border: none;
    background: rgba(0, 0, 0, 0.65);
    backdrop-filter: blur(8px);
    -webkit-backdrop-filter: blur(8px);
    color: rgba(255, 255, 255, 0.85);
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    transition: all 0.15s ease;
    padding: 0;
    -webkit-app-region: no-drag;
  }

  .ctrl-btn:hover {
    background: rgba(0, 0, 0, 0.85);
    color: #ffffff;
    transform: scale(1.1);
  }

  .ctrl-btn:active {
    transform: scale(0.95);
  }

  .ctrl-close:hover {
    background: rgba(255, 59, 48, 0.8);
  }

  .ctrl-mirror.active {
    background: rgba(0, 122, 255, 0.5);
  }

  .ctrl-mirror.active:hover {
    background: rgba(0, 122, 255, 0.7);
  }

  /* ─── Size hint ─── */
  .size-hint {
    position: absolute;
    bottom: 2px;
    left: 50%;
    transform: translateX(-50%);
    font-size: 10px;
    font-weight: 600;
    color: rgba(255, 255, 255, 0.7);
    background: rgba(0, 0, 0, 0.55);
    backdrop-filter: blur(6px);
    -webkit-backdrop-filter: blur(6px);
    padding: 2px 8px;
    border-radius: 6px;
    white-space: nowrap;
    pointer-events: none;
    -webkit-app-region: no-drag;
  }
</style>
