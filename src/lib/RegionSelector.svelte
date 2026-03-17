<script>
  import { invoke } from '@tauri-apps/api/core';
  import { convertFileSrc } from '@tauri-apps/api/core';
  import rough from 'roughjs';


  // ─── Constants ───
  const MIN_SIZE = 50;           // Minimum selection dimension (px)
  const HANDLE_SIZE = 8;         // Resize handle visual size
  const HANDLE_HIT = 14;         // Resize handle hit area
  const SNAP_THRESHOLD = 0;      // reserved for future snap-to-grid

  // ─── State ───
  let phase = $state('idle');    // 'idle' | 'drawing' | 'adjusting'
  let startX = $state(0);
  let startY = $state(0);
  let currentX = $state(0);
  let currentY = $state(0);

  // Finalized selection rect (set after drawing phase completes)
  let selX = $state(0);
  let selY = $state(0);
  let selW = $state(0);
  let selH = $state(0);

  // Drag-to-move / resize state
  let dragType = $state(null);   // 'move' | 'nw' | 'n' | 'ne' | 'e' | 'se' | 's' | 'sw' | 'w'
  let dragStartMouseX = $state(0);
  let dragStartMouseY = $state(0);
  let dragStartRect = $state({ x: 0, y: 0, w: 0, h: 0 });

  // Crosshair position (follows mouse when idle)
  let mouseX = $state(0);
  let mouseY = $state(0);

  // ─── Settings toolbar state ───
  let aspectLocked = $state(false);
  let aspectRatio = $state(1);   // width / height at the time lock is enabled
  let micEnabled = $state(false);
  let systemAudioEnabled = $state(true);
  let selectedQuality = $state('standard'); // 'standard' | 'high'
  let cameraEnabled = $state(false);       // camera bubble toggle (default off)
  let cameraMenuOpen = $state(false);      // camera options dropdown
  let cameraDevices = $state([]);          // available video input devices
  let selectedCameraId = $state('');       // selected camera device id
  let cameraShape = $state('circle');      // 'circle' | 'square' | 'rectangle' | 'vertical'
  let cameraSizePreset = $state('medium'); // 'small' | 'medium' | 'large' | 'huge'
  let cameraSyncTimer = null;

  // ─── Mode detection: 'record' or 'screenshot' ───
  const urlParams = new URLSearchParams(window.location.search);
  const mode = urlParams.get('mode') || 'record';  // default to record
  const isScreenshot = mode === 'screenshot' || mode === 'scroll-screenshot';
  const isScrollScreenshot = mode === 'scroll-screenshot';

  // ─── Frozen screen for screenshot mode ───
  const previewPath = urlParams.get('preview') || '';
  let frozenImageSrc = $state('');
  let frozenCanvas = null;
  let frozenCtx = null;

  if (isScreenshot && !isScrollScreenshot && previewPath) {
    const previewAssetUrl = convertFileSrc(decodeURIComponent(previewPath));
    frozenImageSrc = previewAssetUrl;
    const img = new Image();
    img.onload = () => {
      frozenCanvas = document.createElement('canvas');
      frozenCanvas.width = img.naturalWidth;
      frozenCanvas.height = img.naturalHeight;
      frozenCtx = frozenCanvas.getContext('2d');
      frozenCtx.drawImage(img, 0, 0);
    };
    img.crossOrigin = 'anonymous';
    img.src = previewAssetUrl;
  }

  // ─── Annotation state (screenshot editing) ───
  let activeTool = $state(null);  // 'rect' | 'ellipse' | 'arrow' | 'draw' | 'text' | 'mosaic' | null
  let activeColor = $state('#E8D5B5');
  let annotations = $state([]);
  let currentAnnotation = $state(null);
  let isAnnotating = $state(false);
  let annotStartX = $state(0);
  let annotStartY = $state(0);
  let canvasEl = $state(null);
  let sketchMode = $state(true); // hand-drawn (Excalidraw) style

  // Text annotation state
  let textEditing = $state(null);  // { x, y, color } — active text input position
  let textInputEl = $state(null);  // bound textarea element

  // Selection & drag state
  let selectedIdx = $state(-1);        // index of selected annotation (-1 = none)
  let isDraggingAnnot = $state(false);  // dragging a selected annotation
  let dragAnnotStartX = $state(0);     // mouse start for drag
  let dragAnnotStartY = $state(0);
  let hoveredIdx = $state(-1);         // annotation under mouse cursor
  const TOOL_COLORS = ['#E8D5B5', '#D4956B', '#ff3b30', '#34c759', '#007aff', '#af52de'];

  // Enumerate camera devices
  // NOTE: On macOS WKWebView, enumerateDevices() returns nothing until
  // camera permission has been granted. We do a temporary getUserMedia()
  // first to trigger the system permission prompt, then enumerate.
  // Falls back to native AVFoundation listing for Continuity Camera support.
  async function loadCameraDevices() {
    try {
      // Try WebRTC enumeration first
      let webDevices = [];
      try {
        const tempStream = await navigator.mediaDevices.getUserMedia({ video: true, audio: false });
        tempStream.getTracks().forEach(t => t.stop());
        const devices = await navigator.mediaDevices.enumerateDevices();
        webDevices = devices.filter(d => d.kind === 'videoinput');
      } catch (permErr) {
        console.warn('[region-selector] WebRTC camera enumeration failed:', permErr.message);
      }

      if (webDevices.length > 0) {
        cameraDevices = webDevices;
        console.log('[region-selector] Found cameras via WebRTC:', cameraDevices.length);
      } else {
        // Fallback: native AVFoundation listing (supports Continuity Camera)
        console.log('[region-selector] Falling back to native camera listing...');
        try {
          const nativeDevices = await invoke('list_native_camera_devices');
          cameraDevices = nativeDevices.map(d => ({
            deviceId: d.device_id,
            label: d.label,
            kind: 'videoinput',
            _native: true, // marker for native-only device
          }));
          console.log('[region-selector] Found cameras via AVFoundation:', cameraDevices.length);
        } catch (err) {
          console.error('[region-selector] Native camera listing failed:', err);
          cameraDevices = [];
        }
      }

      // Auto-select first device if available
      if (cameraDevices.length > 0 && !selectedCameraId) {
        selectedCameraId = cameraDevices[0].deviceId;
      }
    } catch (e) {
      console.error('[region-selector] Failed to enumerate cameras:', e);
    }
  }

  // Load devices when entering record mode
  if (!isScreenshot) {
    loadCameraDevices();
  }

  // ─── Size presets ───
  let presetOpen = $state(false);
  const PRESETS = [
    { label: 'Landscape 16:9',   w: 1920, h: 1080, icon: '▬' },
    { label: 'Portrait 9:16',    w: 1080, h: 1920, icon: '▮' },
    { label: 'Social 4:3',       w: 1080, h: 1440, icon: '▭' },
    { label: 'Vertical 4:5',     w: 1080, h: 1350, icon: '▯' },
    { label: 'Square 1:1',       w: 1080, h: 1080, icon: '□' },
    { label: 'Cinematic 2.35:1', w: 1920, h: 817,  icon: '▬' },
  ];

  function applyPreset(preset) {
    const maxW = window.innerWidth;
    const maxH = window.innerHeight;
    const dpr = window.devicePixelRatio || 1;
    // Preset w/h are target OUTPUT pixels. The selection region uses CSS
    // (logical) pixels, and the backend multiplies by the Retina scale
    // factor to get the actual recording resolution. So we divide by DPR
    // to get the correct CSS region size.
    let pw = Math.round(preset.w / dpr);
    let ph = Math.round(preset.h / dpr);
    // If still too large for the screen, scale down to fit
    if (pw > maxW || ph > maxH) {
      const scale = Math.min(maxW / pw, maxH / ph) * 0.95;
      pw = Math.round(pw * scale);
      ph = Math.round(ph * scale);
    }
    // Ensure even dimensions (HEVC requires it after DPR multiplication)
    pw = pw % 2 !== 0 ? pw + 1 : pw;
    ph = ph % 2 !== 0 ? ph + 1 : ph;
    // Center on screen
    selW = pw;
    selH = ph;
    selX = Math.round((maxW - pw) / 2);
    selY = Math.round((maxH - ph) / 2);
    // Lock aspect ratio automatically
    aspectLocked = true;
    aspectRatio = preset.w / preset.h;
    presetOpen = false;

    scheduleCameraSync();
  }

  function getCameraBounds() {
    const boundW = selW > 0 ? selW : window.innerWidth;
    const boundH = selH > 0 ? selH : window.innerHeight;
    const boundX = selW > 0 ? selX : 0;
    const boundY = selH > 0 ? selY : 0;
    return { boundX, boundY, boundW, boundH };
  }

  function scheduleCameraSync() {
    if (!cameraEnabled || !selectedCameraId) return;
    if (cameraSyncTimer) clearTimeout(cameraSyncTimer);
    cameraSyncTimer = setTimeout(() => {
      // Sync prev bounds so delta tracking starts fresh after full sync
      const { boundX, boundY } = getCameraBounds();
      prevBoundX = boundX;
      prevBoundY = boundY;
      openCameraPreview(selectedCameraId, cameraShape, cameraSizePreset);
      cameraSyncTimer = null;
    }, 40);
  }

  // Track previous bounds for computing deltas during drag
  let prevBoundX = $state(0);
  let prevBoundY = $state(0);

  // Move camera overlay in real-time via Rust command during region drag
  function emitCameraBoundsUpdate() {
    if (!cameraEnabled) return;
    const { boundX, boundY, boundW, boundH } = getCameraBounds();
    const dx = boundX - prevBoundX;
    const dy = boundY - prevBoundY;
    prevBoundX = boundX;
    prevBoundY = boundY;
    if (Math.abs(dx) > 0.1 || Math.abs(dy) > 0.1) {
      invoke('move_camera_overlay', { dx, dy, boundX, boundY, boundW, boundH });
    }
  }

  // Editable dimension input values (strings for input binding)
  let inputW = $state('');
  let inputH = $state('');
  let inputWFocused = $state(false);
  let inputHFocused = $state(false);

  // Sync inputs from selection rect when not focused.
  // Display OUTPUT pixels (physical) = CSS pixels × DPR.
  // This is what the user cares about — the actual recording resolution.
  $effect(() => {
    if (!inputWFocused) {
      const dpr = window.devicePixelRatio || 1;
      inputW = String(Math.round(selW * dpr));
    }
  });
  $effect(() => {
    if (!inputHFocused) {
      const dpr = window.devicePixelRatio || 1;
      inputH = String(Math.round(selH * dpr));
    }
  });

  // Live drawing rect (during phase === 'drawing')
  let drawRect = $derived({
    x: Math.min(startX, currentX),
    y: Math.min(startY, currentY),
    width: Math.abs(currentX - startX),
    height: Math.abs(currentY - startY),
  });

  // The rect to render
  let rect = $derived(
    phase === 'drawing'
      ? drawRect
      : { x: selX, y: selY, width: selW, height: selH }
  );

  let showSelection = $derived(
    phase === 'drawing'
      ? (drawRect.width > 3 || drawRect.height > 3)
      : phase === 'adjusting' || phase === 'screenshot-editing'
  );

  let isTooSmall = $derived(false); // Click without drag now selects fullscreen, so never show "too small"

  // ─── Resize handles definition ───
  let handles = $derived((phase === 'adjusting' || phase === 'screenshot-editing') ? [
    { id: 'nw', x: rect.x, y: rect.y, cursor: 'nwse-resize' },
    { id: 'n',  x: rect.x + rect.width / 2, y: rect.y, cursor: 'ns-resize' },
    { id: 'ne', x: rect.x + rect.width, y: rect.y, cursor: 'nesw-resize' },
    { id: 'e',  x: rect.x + rect.width, y: rect.y + rect.height / 2, cursor: 'ew-resize' },
    { id: 'se', x: rect.x + rect.width, y: rect.y + rect.height, cursor: 'nwse-resize' },
    { id: 's',  x: rect.x + rect.width / 2, y: rect.y + rect.height, cursor: 'ns-resize' },
    { id: 'sw', x: rect.x, y: rect.y + rect.height, cursor: 'nesw-resize' },
    { id: 'w',  x: rect.x, y: rect.y + rect.height / 2, cursor: 'ew-resize' },
  ] : []);

  // ─── Toolbar position (centered in selection, like CleanShot X) ───
  let toolbarPos = $derived({
    x: rect.x + rect.width / 2,
    y: rect.y + rect.height / 2,
  });

  // ─── Event Handlers ───

  function onMouseDown(e) {
    const target = e.target;

    // Close preset menu on any click outside it
    if (presetOpen && !target.closest('.preset-wrapper')) {
      presetOpen = false;
    }

    // Close camera menu on any click outside it
    if (cameraMenuOpen && !target.closest('.camera-menu') && !target.closest('.cam-toggle')) {
      cameraMenuOpen = false;
    }

    if (phase === 'idle') {
      // Start drawing a new selection
      startX = e.clientX;
      startY = e.clientY;
      currentX = e.clientX;
      currentY = e.clientY;
      phase = 'drawing';
    } else if (phase === 'adjusting' || phase === 'screenshot-editing') {
      // Check if clicking on a resize handle
      const handle = hitTestHandle(e.clientX, e.clientY);
      if (handle) {
        startDrag(handle, e);
        return;
      }
      // Check if clicking inside selection → move (or annotate if tool active)
      if (isInsideSelection(e.clientX, e.clientY)) {
        if (phase === 'screenshot-editing') {
          return; // Let annotation canvas handle all clicks in screenshot-editing
        }
        startDrag('move', e);
        return;
      }
      // Clicking outside selection
      if (phase === 'screenshot-editing') {
        // Don't allow redrawing in screenshot-editing mode
        return;
      }
      // Clicking outside → redraw new selection
      startX = e.clientX;
      startY = e.clientY;
      currentX = e.clientX;
      currentY = e.clientY;
      phase = 'drawing';
    }
  }

  function onMouseMove(e) {
    mouseX = e.clientX;
    mouseY = e.clientY;

    if (phase === 'drawing') {
      currentX = e.clientX;
      currentY = e.clientY;
    } else if (dragType) {
      applyDrag(e.clientX, e.clientY);
    }
  }

  function onMouseUp(e) {
    if (phase === 'drawing') {
      currentX = e.clientX;
      currentY = e.clientY;

      const finalRect = {
        x: Math.min(startX, currentX),
        y: Math.min(startY, currentY),
        width: Math.abs(currentX - startX),
        height: Math.abs(currentY - startY),
      };

      // Too small (click without drag) → select entire screen (like CleanShot X)
      if (finalRect.width < MIN_SIZE || finalRect.height < MIN_SIZE) {
        selX = 0;
        selY = 0;
        selW = window.innerWidth;
        selH = window.innerHeight;
      } else {
        selX = finalRect.x;
        selY = finalRect.y;
        selW = finalRect.width;
        selH = finalRect.height;
      }

      // Scroll-screenshot mode: start scroll capture immediately
      if (isScrollScreenshot) {
        confirmSelection('screenshot');
        return;
      }

      // Screenshot mode: enter editing phase (toolbar + annotation)
      if (isScreenshot) {
        phase = 'screenshot-editing';
        return;
      }

      // Record mode: enter adjusting phase for toolbar
      phase = 'adjusting';
      scheduleCameraSync();
    } else if (dragType) {
      dragType = null;
      scheduleCameraSync();
    }
  }

  // ─── Drag / Resize Logic ───

  function startDrag(type, e) {
    dragType = type;
    dragStartMouseX = e.clientX;
    dragStartMouseY = e.clientY;
    dragStartRect = { x: selX, y: selY, w: selW, h: selH };
  }

  function applyDrag(mx, my) {
    const dx = mx - dragStartMouseX;
    const dy = my - dragStartMouseY;
    const r = dragStartRect;
    const maxW = window.innerWidth;
    const maxH = window.innerHeight;

    if (dragType === 'move') {
      selX = Math.max(0, Math.min(maxW - r.w, r.x + dx));
      selY = Math.max(0, Math.min(maxH - r.h, r.y + dy));
      emitCameraBoundsUpdate();
      return;
    }

    // Resize — compute new rect based on which handle is dragged
    let nx = r.x, ny = r.y, nw = r.w, nh = r.h;

    if (dragType.includes('w')) {
      nw = Math.max(MIN_SIZE, r.w - dx);
      nx = r.x + r.w - nw;
    }
    if (dragType.includes('e')) {
      nw = Math.max(MIN_SIZE, r.w + dx);
    }
    if (dragType.includes('n')) {
      nh = Math.max(MIN_SIZE, r.h - dy);
      ny = r.y + r.h - nh;
    }
    if (dragType.includes('s')) {
      nh = Math.max(MIN_SIZE, r.h + dy);
    }

    // Apply aspect ratio constraint if locked
    if (aspectLocked && aspectRatio > 0) {
      if (dragType.includes('e') || dragType.includes('w')) {
        nh = Math.round(nw / aspectRatio);
      } else {
        nw = Math.round(nh * aspectRatio);
      }
    }

    // Clamp to viewport
    nx = Math.max(0, nx);
    ny = Math.max(0, ny);
    if (nx + nw > maxW) nw = maxW - nx;
    if (ny + nh > maxH) nh = maxH - ny;

    selX = nx; selY = ny; selW = nw; selH = nh;
    emitCameraBoundsUpdate();
  }

  function hitTestHandle(mx, my) {
    for (const h of handles) {
      if (Math.abs(mx - h.x) <= HANDLE_HIT && Math.abs(my - h.y) <= HANDLE_HIT) {
        return h.id;
      }
    }
    return null;
  }

  function isInsideSelection(mx, my) {
    return mx >= selX && mx <= selX + selW && my >= selY && my <= selY + selH;
  }

  // ─── Dynamic cursor ───
  function getOverlayCursor(mx, my) {
    if (phase === 'drawing') return 'crosshair';
    if ((phase === 'adjusting' || phase === 'screenshot-editing') && !dragType) {
      const h = hitTestHandle(mx, my);
      if (h) {
        const handle = handles.find(hh => hh.id === h);
        return handle ? handle.cursor : 'crosshair';
      }
      if (isInsideSelection(mx, my)) {
        if (phase === 'screenshot-editing') {
          if (isDraggingAnnot) return 'grabbing';
          if (activeTool) return activeTool === 'text' ? 'text' : 'crosshair';
          // Check if hovering over an annotation
          const ax = mx - selX, ay = my - selY;
          if (hitTestAnnotation(ax, ay) >= 0) return 'grab';
          return 'default';
        }
        return 'move';
      }
    }
    return 'crosshair';
  }

  let overlayCursor = $derived(getOverlayCursor(mouseX, mouseY));

  // ─── Dimension input handlers ───

  function onWidthInput(e) {
    inputW = e.target.value;
    const v = parseInt(inputW, 10);
    const dpr = window.devicePixelRatio || 1;
    if (!isNaN(v) && v >= MIN_SIZE) {
      // User types output pixels → convert to CSS pixels
      selW = Math.min(Math.round(v / dpr), window.innerWidth - selX);
      if (aspectLocked && aspectRatio > 0) {
        selH = Math.round(selW / aspectRatio);
        selH = Math.min(selH, window.innerHeight - selY);
      }
      scheduleCameraSync();
    }
  }

  function onHeightInput(e) {
    inputH = e.target.value;
    const v = parseInt(inputH, 10);
    const dpr = window.devicePixelRatio || 1;
    if (!isNaN(v) && v >= MIN_SIZE) {
      // User types output pixels → convert to CSS pixels
      selH = Math.min(Math.round(v / dpr), window.innerHeight - selY);
      if (aspectLocked && aspectRatio > 0) {
        selW = Math.round(selH * aspectRatio);
        selW = Math.min(selW, window.innerWidth - selX);
      }
      scheduleCameraSync();
    }
  }

  function onWidthKeyDown(e) {
    if (e.key === 'Enter') e.target.blur();
  }
  function onHeightKeyDown(e) {
    if (e.key === 'Enter') e.target.blur();
  }

  function toggleAspectLock() {
    aspectLocked = !aspectLocked;
    if (aspectLocked && selH > 0) {
      aspectRatio = selW / selH;
    }
  }

  // ─── Actions ───

  async function confirmSelection(format = 'video') {
    try {
      if (isScrollScreenshot) {
        await invoke('start_scroll_capture', {
          x: selX,
          y: selY,
          width: selW,
          height: selH,
        });
      } else if (isScreenshot) {
        await invoke('take_screenshot', {
          x: selX,
          y: selY,
          width: selW,
          height: selH,
        });
      } else {
        await invoke('confirm_region_selection', {
          x: selX,
          y: selY,
          width: selW,
          height: selH,
          quality: selectedQuality,
          systemAudio: systemAudioEnabled,
          microphone: micEnabled,
          format: format,
          camera: cameraEnabled,
          cameraDeviceId: selectedCameraId || null,
          cameraShape: cameraShape,
          cameraSize: cameraSizePreset,
        });
      }
    } catch (e) {
      console.error('Failed to confirm selection:', e);
    }
  }

  // Open camera preview bubble immediately
  async function openCameraPreview(deviceId, shape, size) {
    console.log('[region-selector] openCameraPreview called:', { deviceId, shape, size });
    const { boundX, boundY, boundW, boundH } = getCameraBounds();
    try {
      await invoke('open_camera_overlay_with_options', {
        shape: shape || 'circle',
        size: size || 'medium',
        deviceId: deviceId || null,
        boundX,
        boundY,
        boundW,
        boundH,
      });
      console.log('[region-selector] Camera overlay opened successfully');
    } catch (e) {
      console.error('[region-selector] Failed to open camera preview:', e);
    }
  }

  async function cancel() {
    if (cameraSyncTimer) {
      clearTimeout(cameraSyncTimer);
      cameraSyncTimer = null;
    }
    // Close camera preview if open
    if (cameraEnabled) {
      try { await invoke('close_camera_overlay'); } catch {}
    }
    try {
      await invoke('cancel_region_selection');
    } catch (e) {
      console.error('Failed to cancel region selection:', e);
    }
  }

  function onKeyDown(e) {
    // Don't handle when typing in inputs or textarea
    if (e.target.tagName === 'INPUT' || e.target.tagName === 'TEXTAREA') return;

    if (e.key === 'Escape') {
      if (textEditing) {
        commitTextAnnotation();
        return;
      }
      if (selectedIdx >= 0) {
        selectedIdx = -1;
        redrawAnnotations();
        return;
      }
      if (presetOpen) {
        presetOpen = false;
        return;
      }
      if (phase === 'screenshot-editing') {
        if (activeTool) {
          activeTool = null;
        } else {
          cancel();
        }
        return;
      }
      if (phase === 'adjusting') {
        // Go back to idle
        phase = 'idle';
      } else {
        cancel();
      }
    } else if (e.key === 'Enter' && phase === 'adjusting') {
      confirmSelection('video');
    } else if (e.key === 'Enter' && phase === 'screenshot-editing') {
      if (textEditing) return; // don't confirm while editing text
      confirmScreenshotEdit();
    } else if ((e.key === 'Delete' || e.key === 'Backspace') && phase === 'screenshot-editing' && selectedIdx >= 0 && !textEditing) {
      if (e.target.tagName === 'TEXTAREA') return;
      annotations = annotations.filter((_, i) => i !== selectedIdx);
      selectedIdx = -1;
      redrawAnnotations();
    }
  }

  // ═══════════════════════════════════════════════════════════════
  //  Screenshot Annotation Tools
  // ═══════════════════════════════════════════════════════════════

  function getAnnotCoords(e) {
    return { x: e.clientX - selX, y: e.clientY - selY };
  }

  // ─── Hit-testing: find annotation at point ───
  function hitTestAnnotation(px, py) {
    // Iterate in reverse so topmost drawn annotation is picked first
    for (let i = annotations.length - 1; i >= 0; i--) {
      if (isPointInAnnotation(px, py, annotations[i])) return i;
    }
    return -1;
  }

  function isPointInAnnotation(px, py, a) {
    const margin = 6; // hit tolerance
    if (a.type === 'rect' || a.type === 'mosaic') {
      return px >= a.x - margin && px <= a.x + a.w + margin &&
             py >= a.y - margin && py <= a.y + a.h + margin;
    } else if (a.type === 'ellipse') {
      const cx = a.x + a.w / 2, cy = a.y + a.h / 2;
      const rx = a.w / 2 + margin, ry = a.h / 2 + margin;
      if (rx === 0 || ry === 0) return false;
      return ((px - cx) ** 2) / (rx ** 2) + ((py - cy) ** 2) / (ry ** 2) <= 1;
    } else if (a.type === 'arrow') {
      // Distance from point to line segment
      return distToSegment(px, py, a.x1, a.y1, a.x2, a.y2) <= margin + 4;
    } else if (a.type === 'draw') {
      for (let j = 1; j < a.points.length; j++) {
        if (distToSegment(px, py, a.points[j - 1].x, a.points[j - 1].y, a.points[j].x, a.points[j].y) <= margin + 4) return true;
      }
      return false;
    } else if (a.type === 'text') {
      const lines = a.text.split('\n');
      const th = lines.length * a.fontSize * 1.3;
      const tw = Math.max(...lines.map(l => l.length)) * a.fontSize * 0.65;
      return px >= a.x - margin && px <= a.x + tw + margin &&
             py >= a.y - margin && py <= a.y + th + margin;
    }
    return false;
  }

  function distToSegment(px, py, x1, y1, x2, y2) {
    const dx = x2 - x1, dy = y2 - y1;
    const lenSq = dx * dx + dy * dy;
    if (lenSq === 0) return Math.hypot(px - x1, py - y1);
    let t = ((px - x1) * dx + (py - y1) * dy) / lenSq;
    t = Math.max(0, Math.min(1, t));
    return Math.hypot(px - (x1 + t * dx), py - (y1 + t * dy));
  }

  // ─── Move annotation by delta ───
  function moveAnnotation(idx, dx, dy) {
    const a = { ...annotations[idx] };
    if (a.type === 'rect' || a.type === 'ellipse' || a.type === 'mosaic') {
      a.x += dx; a.y += dy;
    } else if (a.type === 'arrow') {
      a.x1 += dx; a.y1 += dy; a.x2 += dx; a.y2 += dy;
    } else if (a.type === 'draw') {
      a.points = a.points.map(p => ({ x: p.x + dx, y: p.y + dy }));
    } else if (a.type === 'text') {
      a.x += dx; a.y += dy;
    }
    annotations = annotations.map((ann, i) => i === idx ? a : ann);
  }

  function onAnnotMouseDown(e) {
    // Don't intercept clicks on the text input
    if (textEditing) return;

    e.stopPropagation();
    e.preventDefault();
    const { x, y } = getAnnotCoords(e);

    // If no tool is active, try to select/drag existing annotation
    if (!activeTool) {
      const hit = hitTestAnnotation(x, y);
      if (hit >= 0) {
        // Click on text annotation → enter edit mode directly
        if (annotations[hit].type === 'text') {
          const a = annotations[hit];
          textEditing = { x: a.x, y: a.y, color: a.color, editIdx: hit };
          selectedIdx = -1;
          redrawAnnotations();
          requestAnimationFrame(() => {
            if (textInputEl) { textInputEl.value = a.text; textInputEl.focus(); }
          });
          return;
        }
        selectedIdx = hit;
        isDraggingAnnot = true;
        dragAnnotStartX = x;
        dragAnnotStartY = y;
        // Use window-level events so drag continues even outside canvas
        const onDragMove = (ev) => {
          const coords = { x: ev.clientX - selX, y: ev.clientY - selY };
          const dx = coords.x - dragAnnotStartX;
          const dy = coords.y - dragAnnotStartY;
          moveAnnotation(selectedIdx, dx, dy);
          dragAnnotStartX = coords.x;
          dragAnnotStartY = coords.y;
          redrawAnnotations();
        };
        const onDragUp = () => {
          isDraggingAnnot = false;
          window.removeEventListener('mousemove', onDragMove);
          window.removeEventListener('mouseup', onDragUp);
        };
        window.addEventListener('mousemove', onDragMove);
        window.addEventListener('mouseup', onDragUp);
        redrawAnnotations();
      } else {
        selectedIdx = -1;
        redrawAnnotations();
      }
      return;
    }

    // Deselect when drawing
    selectedIdx = -1;

    // Text tool: commit previous text if any, then open new input
    if (activeTool === 'text') {
      if (textEditing) commitTextAnnotation();
      textEditing = { x, y, color: activeColor };
      requestAnimationFrame(() => { if (textInputEl) textInputEl.focus(); });
      return;
    }

    annotStartX = x;
    annotStartY = y;
    isAnnotating = true;

    // Generate a random seed for sketch mode consistency per annotation
    const seed = Math.floor(Math.random() * 2147483647);
    if (activeTool === 'draw') {
      currentAnnotation = { type: 'draw', points: [{ x, y }], color: activeColor, sw: 3, seed };
    } else if (activeTool === 'mosaic') {
      currentAnnotation = { type: 'mosaic', x, y, w: 0, h: 0 };
    } else if (activeTool === 'rect') {
      currentAnnotation = { type: 'rect', x, y, w: 0, h: 0, color: activeColor, sw: 2, seed };
    } else if (activeTool === 'ellipse') {
      currentAnnotation = { type: 'ellipse', x, y, w: 0, h: 0, color: activeColor, sw: 2, seed };
    } else if (activeTool === 'arrow') {
      currentAnnotation = { type: 'arrow', x1: x, y1: y, x2: x, y2: y, color: activeColor, sw: 2, seed };
    }
  }

  function onAnnotMouseMove(e) {
    // Update hover cursor when no tool active
    if (!activeTool && !isDraggingAnnot && !isAnnotating) {
      const { x, y } = getAnnotCoords(e);
      hoveredIdx = hitTestAnnotation(x, y);
    }
    if (!isAnnotating || !currentAnnotation) return;
    const { x, y } = getAnnotCoords(e);

    if (activeTool === 'draw') {
      currentAnnotation = { ...currentAnnotation, points: [...currentAnnotation.points, { x, y }] };
    } else if (activeTool === 'mosaic' || activeTool === 'rect' || activeTool === 'ellipse') {
      currentAnnotation = {
        ...currentAnnotation,
        x: Math.min(annotStartX, x),
        y: Math.min(annotStartY, y),
        w: Math.abs(x - annotStartX),
        h: Math.abs(y - annotStartY),
      };
    } else if (activeTool === 'arrow') {
      currentAnnotation = { ...currentAnnotation, x2: x, y2: y };
    }
    redrawAnnotations();
  }

  function onAnnotMouseUp(e) {
    if (!isAnnotating) return;
    isAnnotating = false;
    if (currentAnnotation) {
      const a = currentAnnotation;
      const valid =
        a.type === 'draw' ? a.points.length > 1 :
        a.type === 'arrow' ? (Math.abs(a.x2 - a.x1) > 3 || Math.abs(a.y2 - a.y1) > 3) :
        (a.w > 3 && a.h > 3);
      if (valid) {
        annotations = [...annotations, { ...a }];
      }
      currentAnnotation = null;
    }
    redrawAnnotations();
  }

  function undoAnnotation() {
    if (annotations.length > 0) {
      annotations = annotations.slice(0, -1);
      redrawAnnotations();
    }
  }

  function redrawAnnotations() {
    if (!canvasEl) return;
    const ctx = canvasEl.getContext('2d');
    const dpr = window.devicePixelRatio || 1;
    ctx.clearRect(0, 0, canvasEl.width, canvasEl.height);
    for (const anno of annotations) {
      drawAnnotation(ctx, anno, dpr);
    }
    // Draw selection indicator
    if (selectedIdx >= 0 && selectedIdx < annotations.length) {
      drawSelectionIndicator(ctx, annotations[selectedIdx], dpr);
    }
    if (currentAnnotation) {
      drawAnnotation(ctx, currentAnnotation, dpr);
    }
  }

  function drawAnnotation(ctx, a, dpr) {
    ctx.save();
    // Use sketch (hand-drawn) style if enabled and applicable
    const useSketch = sketchMode && a.type !== 'mosaic' && a.type !== 'text';
    if (a.type === 'text') {
      drawTextAnnotation(ctx, a, dpr);
    } else if (useSketch) {
      drawAnnotationSketch(ctx, a, dpr);
    } else {
      drawAnnotationPrecise(ctx, a, dpr);
    }
    ctx.restore();
  }

  function drawAnnotationSketch(ctx, a, dpr) {
    const rc = rough.canvas({ getContext: () => ctx });
    const sw = a.sw * dpr;
    const opts = {
      stroke: a.color,
      strokeWidth: sw,
      roughness: 1.5,
      bowing: 1.2,
      seed: a.seed || 1,
    };
    if (a.type === 'rect') {
      rc.rectangle(a.x * dpr, a.y * dpr, a.w * dpr, a.h * dpr, opts);
    } else if (a.type === 'ellipse') {
      const cx = (a.x + a.w / 2) * dpr;
      const cy = (a.y + a.h / 2) * dpr;
      rc.ellipse(cx, cy, a.w * dpr, a.h * dpr, opts);
    } else if (a.type === 'arrow') {
      const x1 = a.x1 * dpr, y1 = a.y1 * dpr, x2 = a.x2 * dpr, y2 = a.y2 * dpr;
      // Sketch line
      rc.line(x1, y1, x2, y2, opts);
      // Sketch arrowhead
      const headLen = Math.max(sw * 4, 12);
      const angle = Math.atan2(y2 - y1, x2 - x1);
      const ax1 = x2 - headLen * Math.cos(angle - Math.PI / 6);
      const ay1 = y2 - headLen * Math.sin(angle - Math.PI / 6);
      const ax2 = x2 - headLen * Math.cos(angle + Math.PI / 6);
      const ay2 = y2 - headLen * Math.sin(angle + Math.PI / 6);
      rc.polygon([[x2, y2], [ax1, ay1], [ax2, ay2]], {
        ...opts,
        fill: a.color,
        fillStyle: 'solid',
        roughness: 0.8,
      });
    } else if (a.type === 'draw') {
      const pts = a.points.map(p => [p.x * dpr, p.y * dpr]);
      if (pts.length > 1) {
        rc.linearPath(pts, { ...opts, roughness: 0.4 });
      }
    }
  }

  function drawAnnotationPrecise(ctx, a, dpr) {
    if (a.type === 'rect') {
      ctx.strokeStyle = a.color;
      ctx.lineWidth = a.sw * dpr;
      ctx.lineJoin = 'round';
      ctx.strokeRect(a.x * dpr, a.y * dpr, a.w * dpr, a.h * dpr);
    } else if (a.type === 'ellipse') {
      ctx.strokeStyle = a.color;
      ctx.lineWidth = a.sw * dpr;
      const cx = (a.x + a.w / 2) * dpr;
      const cy = (a.y + a.h / 2) * dpr;
      const rx = (a.w / 2) * dpr;
      const ry = (a.h / 2) * dpr;
      ctx.beginPath();
      ctx.ellipse(cx, cy, rx, ry, 0, 0, Math.PI * 2);
      ctx.stroke();
    } else if (a.type === 'arrow') {
      const x1 = a.x1 * dpr, y1 = a.y1 * dpr, x2 = a.x2 * dpr, y2 = a.y2 * dpr;
      const headLen = Math.max(a.sw * dpr * 4, 12);
      const angle = Math.atan2(y2 - y1, x2 - x1);
      ctx.strokeStyle = a.color;
      ctx.fillStyle = a.color;
      ctx.lineWidth = a.sw * dpr;
      ctx.lineCap = 'round';
      ctx.beginPath();
      ctx.moveTo(x1, y1);
      ctx.lineTo(x2, y2);
      ctx.stroke();
      ctx.beginPath();
      ctx.moveTo(x2, y2);
      ctx.lineTo(x2 - headLen * Math.cos(angle - Math.PI / 6), y2 - headLen * Math.sin(angle - Math.PI / 6));
      ctx.lineTo(x2 - headLen * Math.cos(angle + Math.PI / 6), y2 - headLen * Math.sin(angle + Math.PI / 6));
      ctx.closePath();
      ctx.fill();
    } else if (a.type === 'draw') {
      ctx.strokeStyle = a.color;
      ctx.lineWidth = a.sw * dpr;
      ctx.lineCap = 'round';
      ctx.lineJoin = 'round';
      ctx.beginPath();
      for (let i = 0; i < a.points.length; i++) {
        const p = a.points[i];
        if (i === 0) ctx.moveTo(p.x * dpr, p.y * dpr);
        else ctx.lineTo(p.x * dpr, p.y * dpr);
      }
      ctx.stroke();
    } else if (a.type === 'mosaic') {
      drawMosaicBlock(ctx, a, dpr);
    }
  }

  function commitTextAnnotation() {
    if (!textEditing) return;
    const text = textInputEl ? textInputEl.value.trim() : '';
    // If we're editing an existing text annotation
    if (textEditing.editIdx != null) {
      if (text) {
        annotations = annotations.map((a, i) => i === textEditing.editIdx ? { ...a, text, color: textEditing.color } : a);
      } else {
        // Empty text = delete
        annotations = annotations.filter((_, i) => i !== textEditing.editIdx);
        selectedIdx = -1;
      }
    } else if (text) {
      annotations = [...annotations, { type: 'text', x: textEditing.x, y: textEditing.y, text, color: textEditing.color, fontSize: 16 }];
    }
    textEditing = null;
    redrawAnnotations();
  }

  function drawTextAnnotation(ctx, a, dpr) {
    const fontSize = a.fontSize * dpr;
    ctx.fillStyle = a.color;
    if (sketchMode) {
      ctx.font = `${fontSize}px 'Segoe Print', 'Comic Sans MS', 'Bradley Hand', cursive`;
    } else {
      ctx.font = `${fontSize}px -apple-system, BlinkMacSystemFont, 'Helvetica Neue', sans-serif`;
    }
    ctx.textBaseline = 'top';
    const lines = a.text.split('\n');
    for (let i = 0; i < lines.length; i++) {
      ctx.fillText(lines[i], a.x * dpr, (a.y + i * a.fontSize * 1.3) * dpr);
    }
  }

  // ─── Selection indicator (dashed border around selected annotation) ───
  function drawSelectionIndicator(ctx, a, dpr) {
    ctx.save();
    ctx.strokeStyle = 'rgba(96, 165, 250, 0.8)';
    ctx.lineWidth = 1.5 * dpr;
    ctx.setLineDash([4 * dpr, 3 * dpr]);
    const pad = 4;
    let bx, by, bw, bh;
    if (a.type === 'rect' || a.type === 'ellipse' || a.type === 'mosaic') {
      bx = a.x - pad; by = a.y - pad; bw = a.w + pad * 2; bh = a.h + pad * 2;
    } else if (a.type === 'arrow') {
      bx = Math.min(a.x1, a.x2) - pad; by = Math.min(a.y1, a.y2) - pad;
      bw = Math.abs(a.x2 - a.x1) + pad * 2; bh = Math.abs(a.y2 - a.y1) + pad * 2;
    } else if (a.type === 'draw') {
      const xs = a.points.map(p => p.x), ys = a.points.map(p => p.y);
      bx = Math.min(...xs) - pad; by = Math.min(...ys) - pad;
      bw = Math.max(...xs) - Math.min(...xs) + pad * 2;
      bh = Math.max(...ys) - Math.min(...ys) + pad * 2;
    } else if (a.type === 'text') {
      const lines = a.text.split('\n');
      const th = lines.length * a.fontSize * 1.3;
      const tw = Math.max(...lines.map(l => l.length)) * a.fontSize * 0.65;
      bx = a.x - pad; by = a.y - pad; bw = tw + pad * 2; bh = th + pad * 2;
    }
    if (bw && bh) {
      ctx.strokeRect(bx * dpr, by * dpr, bw * dpr, bh * dpr);
    }
    ctx.restore();
  }

  function drawMosaicBlock(ctx, a, dpr) {
    if (!frozenCtx || !frozenCanvas) return;
    const blockSize = 10;
    for (let by = 0; by < a.h; by += blockSize) {
      for (let bx = 0; bx < a.w; bx += blockSize) {
        const imgX = Math.min(Math.max(0, Math.round((selX + a.x + bx + blockSize / 2) * dpr)), frozenCanvas.width - 1);
        const imgY = Math.min(Math.max(0, Math.round((selY + a.y + by + blockSize / 2) * dpr)), frozenCanvas.height - 1);
        const pixel = frozenCtx.getImageData(imgX, imgY, 1, 1).data;
        ctx.fillStyle = `rgb(${pixel[0]},${pixel[1]},${pixel[2]})`;
        ctx.fillRect(
          Math.round((a.x + bx) * dpr),
          Math.round((a.y + by) * dpr),
          Math.ceil(blockSize * dpr),
          Math.ceil(blockSize * dpr)
        );
      }
    }
  }

  // ─── Screenshot confirm/cancel ───
  async function confirmScreenshotEdit() {
    if (annotations.length === 0) {
      // No annotations — capture live screen and copy to clipboard
      try {
        await invoke('screenshot_to_clipboard', {
          x: selX, y: selY, width: selW, height: selH,
        });
      } catch (e) {
        console.error('Screenshot failed:', e);
      }
      cancel();
      return;
    }

    // Has annotations — composite from frozen image + canvas
    const dpr = window.devicePixelRatio || 1;
    const finalW = Math.round(selW * dpr);
    const finalH = Math.round(selH * dpr);
    const finalCanvas = document.createElement('canvas');
    finalCanvas.width = finalW;
    finalCanvas.height = finalH;
    const fCtx = finalCanvas.getContext('2d');

    // Draw frozen background cropped to selection
    if (frozenCanvas) {
      fCtx.drawImage(
        frozenCanvas,
        Math.round(selX * dpr), Math.round(selY * dpr), finalW, finalH,
        0, 0, finalW, finalH
      );
    }

    // Draw annotations
    for (const anno of annotations) {
      drawAnnotation(fCtx, anno, dpr);
    }

    // Export as base64 PNG and copy via Rust
    const dataUrl = finalCanvas.toDataURL('image/png');
    const base64 = dataUrl.split(',')[1];
    try {
      await invoke('copy_image_data_to_clipboard', { data: base64 });
    } catch (e) {
      console.error('Failed to copy annotated screenshot:', e);
    }
    cancel();
  }

  function selectTool(tool) {
    if (textEditing) commitTextAnnotation();
    activeTool = activeTool === tool ? null : tool;
  }

  // Pin screenshot to desktop (always on top, movable, resizable)
  async function pinScreenshotEdit() {
    const dpr = window.devicePixelRatio || 1;
    const finalW = Math.round(selW * dpr);
    const finalH = Math.round(selH * dpr);
    const finalCanvas = document.createElement('canvas');
    finalCanvas.width = finalW;
    finalCanvas.height = finalH;
    const fCtx = finalCanvas.getContext('2d');

    // Draw frozen background cropped to selection
    if (frozenCanvas) {
      fCtx.drawImage(
        frozenCanvas,
        Math.round(selX * dpr), Math.round(selY * dpr), finalW, finalH,
        0, 0, finalW, finalH
      );
    }

    // Draw annotations on top
    for (const anno of annotations) {
      drawAnnotation(fCtx, anno, dpr);
    }

    // Save to temp file via take_screenshot, then pin it
    const dataUrl = finalCanvas.toDataURL('image/png');
    const base64 = dataUrl.split(',')[1];
    try {
      // Write annotated image to temp, then pin at original position
      // Rust side destroys region-selector, so no cancel() needed
      await invoke('save_annotated_and_pin', {
        data: base64,
        x: selX, y: selY, width: selW, height: selH,
      });
    } catch (e) {
      console.error('Failed to pin screenshot:', e);
    }
  }
</script>

<svelte:window onkeydown={onKeyDown} />

<!-- Frozen screen background for screenshot mode -->
{#if frozenImageSrc}
  <img src={frozenImageSrc} class="frozen-bg" alt="" draggable="false" />
{/if}

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="overlay"
  style="cursor: {overlayCursor};"
  onmousedown={onMouseDown}
  onmousemove={onMouseMove}
  onmouseup={onMouseUp}
>
  {#if showSelection}
    <!-- Dim mask: 4 rectangles around the selection -->
    <div class="dim" style="top:0;left:0;width:{rect.x}px;bottom:0;"></div>
    <div class="dim" style="top:0;left:{rect.x}px;width:{rect.width}px;height:{rect.y}px;"></div>
    <div class="dim" style="top:{rect.y + rect.height}px;left:{rect.x}px;width:{rect.width}px;bottom:0;"></div>
    <div class="dim" style="top:0;left:{rect.x + rect.width}px;right:0;bottom:0;"></div>

    <!-- Selection rectangle -->
    <div
      class="selection"
      class:too-small={isTooSmall}
      class:confirmed={phase === 'adjusting'}
      style="left:{rect.x}px;top:{rect.y}px;width:{rect.width}px;height:{rect.height}px;"
    ></div>

    <!-- Dimensions label (drawing phase only) -->
    {#if phase === 'drawing'}
      <div
        class="dimensions"
        style="left:{rect.x + rect.width / 2}px;top:{rect.y + rect.height + 10}px;"
      >
        {#if drawRect.width < MIN_SIZE || drawRect.height < MIN_SIZE}
          Fullscreen
        {:else}
          {Math.round(rect.width * (window.devicePixelRatio || 1))} &times; {Math.round(rect.height * (window.devicePixelRatio || 1))}
        {/if}
      </div>
    {/if}

    <!-- Resize handles (adjusting phase only) -->
    {#if phase === 'adjusting'}
      {#each handles as h}
        <div
          class="handle"
          style="left:{h.x}px;top:{h.y}px;cursor:{h.cursor};"
        ></div>
      {/each}

      <!-- ═══ Settings Toolbar ═══ -->
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div
        class="toolbar"
        style="left:{toolbarPos.x}px;top:{toolbarPos.y}px;"
        onmousedown={(e) => e.stopPropagation()}
      >
        <!-- Row 1: Dimensions + Settings -->
        <div class="toolbar-row toolbar-top">
          <!-- Width input -->
          <input
            type="text"
            class="dim-input"
            value={inputW}
            oninput={onWidthInput}
            onfocus={() => inputWFocused = true}
            onblur={() => inputWFocused = false}
            onkeydown={onWidthKeyDown}
          />

          <!-- Dimension separator × -->
          <span class="dim-times">×</span>

          <!-- Height input -->
          <input
            type="text"
            class="dim-input"
            value={inputH}
            oninput={onHeightInput}
            onfocus={() => inputHFocused = true}
            onblur={() => inputHFocused = false}
            onkeydown={onHeightKeyDown}
          />

          <!-- Preset dropdown (opens upward) -->
          <div class="preset-wrapper">
            <button
              class="toolbar-btn preset-btn"
              class:active={presetOpen}
              title="Size presets"
              onclick={(e) => { e.stopPropagation(); presetOpen = !presetOpen; }}
            >
              <svg class="chevron-up" class:open={presetOpen} width="10" height="6" viewBox="0 0 10 6" fill="none">
                <path d="M1 5l4-4 4 4" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"/>
              </svg>
            </button>
            {#if presetOpen}
              <!-- svelte-ignore a11y_no_static_element_interactions -->
              <div class="preset-menu" onmousedown={(e) => e.stopPropagation()}>
                {#each PRESETS as preset}
                  <button class="preset-item" onclick={() => applyPreset(preset)}>
                    <span class="preset-icon">{preset.icon}</span>
                    <span class="preset-label">{preset.label}</span>
                    <span class="preset-size">{preset.w}×{preset.h}</span>
                  </button>
                {/each}
              </div>
            {/if}
          </div>

          <div class="toolbar-sep"></div>

          <!-- Cancel button -->
          <button class="toolbar-btn cancel-btn" onclick={cancel} title="Cancel (Esc)">
            <svg width="12" height="12" viewBox="0 0 12 12" fill="none">
              <path d="M2 2l8 8M10 2l-8 8" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/>
            </svg>
          </button>
        </div>

        <!-- Row 2: Audio toggles + Record button (record mode only) -->
        {#if !isScreenshot}
        <!-- Row 2: Audio toggles + Record button -->
        <div class="toolbar-row toolbar-bottom">
          <!-- Microphone toggle -->
          <button
            class="toolbar-btn audio-btn"
            class:active={micEnabled}
            title={micEnabled ? 'Disable microphone' : 'Enable microphone'}
            onclick={() => micEnabled = !micEnabled}
          >
            {#if micEnabled}
              <svg width="16" height="16" viewBox="0 0 16 16" fill="none">
                <rect x="5.5" y="1" width="5" height="9" rx="2.5" stroke="currentColor" stroke-width="1.5" fill="none"/>
                <path d="M3 7.5a5 5 0 0010 0" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" fill="none"/>
                <line x1="8" y1="12.5" x2="8" y2="15" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/>
              </svg>
            {:else}
              <svg width="16" height="16" viewBox="0 0 16 16" fill="none">
                <rect x="5.5" y="1" width="5" height="9" rx="2.5" stroke="currentColor" stroke-width="1.5" fill="none" opacity="0.4"/>
                <path d="M3 7.5a5 5 0 0010 0" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" fill="none" opacity="0.4"/>
                <line x1="8" y1="12.5" x2="8" y2="15" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" opacity="0.4"/>
                <line x1="2" y1="2" x2="14" y2="14" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" opacity="0.7"/>
              </svg>
            {/if}
            <span class="audio-label">Mic</span>
          </button>

          <!-- System audio toggle -->
          <button
            class="toolbar-btn audio-btn"
            class:active={systemAudioEnabled}
            title={systemAudioEnabled ? 'Disable system audio' : 'Enable system audio'}
            onclick={() => systemAudioEnabled = !systemAudioEnabled}
          >
            {#if systemAudioEnabled}
              <svg width="16" height="16" viewBox="0 0 16 16" fill="none">
                <path d="M2 5.5h2l4-3.5v12l-4-3.5H2a1 1 0 01-1-1v-3a1 1 0 011-1z" stroke="currentColor" stroke-width="1.3" fill="none"/>
                <path d="M11 5.5a3 3 0 010 5" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" fill="none"/>
                <path d="M12.5 3.5a5.5 5.5 0 010 9" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" fill="none"/>
              </svg>
            {:else}
              <svg width="16" height="16" viewBox="0 0 16 16" fill="none">
                <path d="M2 5.5h2l4-3.5v12l-4-3.5H2a1 1 0 01-1-1v-3a1 1 0 011-1z" stroke="currentColor" stroke-width="1.3" fill="none" opacity="0.4"/>
                <line x1="10" y1="5.5" x2="15" y2="10.5" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" opacity="0.7"/>
                <line x1="15" y1="5.5" x2="10" y2="10.5" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" opacity="0.7"/>
              </svg>
            {/if}
            <span class="audio-label">System</span>
          </button>

          <div class="toolbar-sep"></div>

          <!-- Quality selector -->
          <div class="quality-toggle">
            <button
              class="quality-opt"
              class:active={selectedQuality === 'standard'}
              onclick={() => selectedQuality = 'standard'}
            >SD</button>
            <button
              class="quality-opt"
              class:active={selectedQuality === 'high'}
              onclick={() => selectedQuality = 'high'}
            >HD</button>
          </div>

          <div class="toolbar-sep"></div>

          <!-- Camera toggle with dropdown -->
          <div class="preset-wrapper">
            <button
              class="toolbar-btn audio-btn cam-toggle"
              class:active={cameraEnabled}
              title={cameraEnabled ? 'Camera settings' : 'Enable camera bubble'}
              onmousedown={(e) => e.stopPropagation()}
              onclick={(e) => {
                e.stopPropagation();
                if (!cameraEnabled) {
                  cameraEnabled = true;
                  cameraMenuOpen = true;
                  // If a default device is already selected, open preview immediately
                  if (selectedCameraId) {
                    openCameraPreview(selectedCameraId, cameraShape, cameraSizePreset);
                  }
                } else {
                  cameraMenuOpen = !cameraMenuOpen;
                }
              }}
            >
              <svg width="16" height="16" viewBox="0 0 16 16" fill="none">
                {#if cameraEnabled}
                  <rect x="1" y="3.5" width="10" height="9" rx="2" stroke="currentColor" stroke-width="1.3" fill="none"/>
                  <path d="M11 7l4-2.5v7L11 9" stroke="currentColor" stroke-width="1.3" stroke-linejoin="round" fill="none"/>
                {:else}
                  <rect x="1" y="3.5" width="10" height="9" rx="2" stroke="currentColor" stroke-width="1.3" fill="none" opacity="0.4"/>
                  <path d="M11 7l4-2.5v7L11 9" stroke="currentColor" stroke-width="1.3" stroke-linejoin="round" fill="none" opacity="0.4"/>
                  <line x1="1" y1="2" x2="15" y2="14" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" opacity="0.7"/>
                {/if}
              </svg>
              <span class="audio-label">Cam</span>
            </button>

            {#if cameraMenuOpen}
              <!-- svelte-ignore a11y_no_static_element_interactions -->
              <div class="camera-menu" onmousedown={(e) => e.stopPropagation()}>
                <!-- Camera source -->
                <div class="camera-menu-section">Camera</div>
                <button class="camera-menu-item" onclick={() => { cameraEnabled = false; cameraMenuOpen = false; invoke('close_camera_overlay'); }}>
                  <span class="camera-menu-check">{!cameraEnabled ? '✓' : ''}</span>
                  None
                </button>
                {#each cameraDevices as device}
                  <button class="camera-menu-item" onclick={async (e) => {
                    e.stopPropagation();
                    selectedCameraId = device.deviceId;
                    cameraEnabled = true;
                    try {
                      await openCameraPreview(device.deviceId, cameraShape, cameraSizePreset);
                    } catch (err) {
                      // Show error visibly on screen for debugging
                      document.title = 'CAM ERR: ' + String(err);
                    }
                  }}>
                    <span class="camera-menu-check">{cameraEnabled && selectedCameraId === device.deviceId ? '✓' : ''}</span>
                    {device.label || `Camera ${cameraDevices.indexOf(device) + 1}`}
                  </button>
                {/each}
                {#if cameraDevices.length === 0}
                  <div class="camera-menu-hint">No cameras found</div>
                {/if}

                <div class="camera-menu-divider"></div>

                <!-- Size -->
                <div class="camera-menu-section">Size</div>
                {#each [['small', 'Small'], ['medium', 'Medium'], ['large', 'Large'], ['huge', 'Huge']] as [val, label]}
                  <button class="camera-menu-item" onclick={() => { cameraSizePreset = val; if (cameraEnabled) openCameraPreview(selectedCameraId, cameraShape, val); }}>
                    <span class="camera-menu-check">{cameraSizePreset === val ? '✓' : ''}</span>
                    {label}
                  </button>
                {/each}

                <div class="camera-menu-divider"></div>

                <!-- Shape -->
                <div class="camera-menu-section">Shape</div>
                {#each [['circle', 'Circle'], ['square', 'Square'], ['rectangle', 'Rectangle'], ['vertical', 'Vertical']] as [val, label]}
                  <button class="camera-menu-item" onclick={() => { cameraShape = val; if (cameraEnabled) openCameraPreview(selectedCameraId, val, cameraSizePreset); }}>
                    <span class="camera-menu-check">{cameraShape === val ? '✓' : ''}</span>
                    {label}
                  </button>
                {/each}
              </div>
            {/if}
          </div>

          <div class="toolbar-sep"></div>

          <!-- Record GIF button -->
          <button class="btn-record-gif" onclick={() => confirmSelection('gif')}>
            <svg width="16" height="16" viewBox="0 0 16 16" fill="none">
              <rect x="1" y="3" width="14" height="10" rx="2" stroke="currentColor" stroke-width="1.4" fill="none"/>
              <circle cx="8" cy="8" r="2" stroke="currentColor" stroke-width="1.2" fill="none"/>
              <path d="M4 6h1M11 6h1" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"/>
            </svg>
            GIF
          </button>

          <!-- Record Video button -->
          <button class="btn-record-video" onclick={() => confirmSelection('video')}>
            <svg width="16" height="16" viewBox="0 0 16 16" fill="none">
              <rect x="1" y="3.5" width="10" height="9" rx="2" stroke="currentColor" stroke-width="1.4" fill="none"/>
              <path d="M11 7l4-2.5v7L11 9" stroke="currentColor" stroke-width="1.4" stroke-linejoin="round" fill="none"/>
            </svg>
            Video
          </button>
        </div>
        {/if}
      </div>
    {/if}

    <!-- ═══ Screenshot Editing Phase ═══ -->
    {#if phase === 'screenshot-editing'}
      {#each handles as h}
        <div
          class="handle"
          style="left:{h.x}px;top:{h.y}px;cursor:{h.cursor};"
        ></div>
      {/each}

      <!-- Annotation canvas -->
      <canvas
        bind:this={canvasEl}
        class="annotation-canvas"
        width={Math.round(selW * (window.devicePixelRatio || 1))}
        height={Math.round(selH * (window.devicePixelRatio || 1))}
        style="left:{selX}px;top:{selY}px;width:{selW}px;height:{selH}px;pointer-events:auto;cursor:inherit;"
        onmousedown={(e) => { e.stopPropagation(); onAnnotMouseDown(e); }}
        onmousemove={onAnnotMouseMove}
        onmouseup={onAnnotMouseUp}
      ></canvas>

      <!-- Text input overlay -->
      {#if textEditing}
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <textarea
          bind:this={textInputEl}
          class="text-annot-input"
          style="left:{selX + textEditing.x}px;top:{selY + textEditing.y}px;color:{textEditing.color};"
          onmousedown={(e) => e.stopPropagation()}
          onkeydown={(e) => {
            e.stopPropagation();
            if (e.key === 'Escape') { commitTextAnnotation(); }
            else if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); commitTextAnnotation(); }
          }}
          onblur={() => commitTextAnnotation()}
          placeholder="输入文字..."
        ></textarea>
      {/if}

      <!-- Screenshot toolbar (below selection, right-aligned) -->
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div
        class="ss-toolbar"
        style="left:{Math.max(8, Math.min(selX + selW, window.innerWidth - 8))}px;top:{selY + selH + 10 + 44 < window.innerHeight ? selY + selH + 10 : selY - 50}px;"
        onmousedown={(e) => e.stopPropagation()}
      >
        <!-- Annotation tools -->
        <button class="ss-tool" class:active={activeTool === 'rect'} onclick={() => selectTool('rect')} title="Rectangle (outline)">
          <svg width="16" height="16" viewBox="0 0 16 16" fill="none">
            <rect x="2" y="3" width="12" height="10" rx="1.5" stroke="currentColor" stroke-width="1.5" fill="none"/>
          </svg>
        </button>
        <button class="ss-tool" class:active={activeTool === 'ellipse'} onclick={() => selectTool('ellipse')} title="Ellipse (circle)">
          <svg width="16" height="16" viewBox="0 0 16 16" fill="none">
            <ellipse cx="8" cy="8" rx="6" ry="5" stroke="currentColor" stroke-width="1.5" fill="none"/>
          </svg>
        </button>
        <button class="ss-tool" class:active={activeTool === 'arrow'} onclick={() => selectTool('arrow')} title="Arrow">
          <svg width="16" height="16" viewBox="0 0 16 16" fill="none">
            <path d="M3 13L13 3" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/>
            <path d="M13 3L8 3M13 3L13 8" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/>
          </svg>
        </button>
        <button class="ss-tool" class:active={activeTool === 'draw'} onclick={() => selectTool('draw')} title="Freehand draw">
          <svg width="16" height="16" viewBox="0 0 16 16" fill="none">
            <path d="M3 11.5Q5 6 8 8T13 4.5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" fill="none"/>
          </svg>
        </button>
        <button class="ss-tool" class:active={activeTool === 'text'} onclick={() => selectTool('text')} title="Text">
          <svg width="16" height="16" viewBox="0 0 16 16" fill="none">
            <text x="3" y="13" font-size="13" font-weight="600" font-family="serif" fill="currentColor">T</text>
          </svg>
        </button>
        <button class="ss-tool" class:active={activeTool === 'mosaic'} onclick={() => selectTool('mosaic')} title="Mosaic / Pixelate">
          <svg width="16" height="16" viewBox="0 0 16 16" fill="none">
            <rect x="2" y="2" width="4" height="4" fill="currentColor" opacity="0.7"/>
            <rect x="6" y="6" width="4" height="4" fill="currentColor" opacity="0.5"/>
            <rect x="10" y="2" width="4" height="4" fill="currentColor" opacity="0.3"/>
            <rect x="2" y="10" width="4" height="4" fill="currentColor" opacity="0.3"/>
            <rect x="10" y="10" width="4" height="4" fill="currentColor" opacity="0.7"/>
            <rect x="6" y="2" width="4" height="4" fill="currentColor" opacity="0.4"/>
            <rect x="2" y="6" width="4" height="4" fill="currentColor" opacity="0.4"/>
            <rect x="10" y="6" width="4" height="4" fill="currentColor" opacity="0.4"/>
            <rect x="6" y="10" width="4" height="4" fill="currentColor" opacity="0.5"/>
          </svg>
        </button>

        <!-- Sketch / Precise style toggle -->
        <button class="ss-tool" class:active={sketchMode} onclick={() => { sketchMode = !sketchMode; redrawAnnotations(); }} title={sketchMode ? 'Hand-drawn style (click for precise)' : 'Precise style (click for hand-drawn)'}>
          <svg width="16" height="16" viewBox="0 0 16 16" fill="none">
            {#if sketchMode}
              <!-- Pencil icon for sketch mode -->
              <path d="M2.5 13.5l1-4L11 2l2.5 2.5L6 12l-3.5 1.5z" stroke="currentColor" stroke-width="1.3" stroke-linejoin="round" fill="none"/>
              <path d="M9.5 3.5l2.5 2.5" stroke="currentColor" stroke-width="1.3" stroke-linecap="round"/>
            {:else}
              <!-- Ruler icon for precise mode -->
              <rect x="1" y="6" width="14" height="4" rx="1" stroke="currentColor" stroke-width="1.3" fill="none"/>
              <line x1="4" y1="6" x2="4" y2="8" stroke="currentColor" stroke-width="1.2"/>
              <line x1="8" y1="6" x2="8" y2="8.5" stroke="currentColor" stroke-width="1.2"/>
              <line x1="12" y1="6" x2="12" y2="8" stroke="currentColor" stroke-width="1.2"/>
            {/if}
          </svg>
        </button>

        <div class="ss-sep"></div>

        <!-- Color picker -->
        {#each TOOL_COLORS as c}
          <button
            class="ss-color"
            class:active={activeColor === c}
            style="background:{c};"
            onclick={() => activeColor = c}
            title={c}
          ></button>
        {/each}

        <div class="ss-sep"></div>

        <!-- Pin to desktop -->
        <button class="ss-tool" onclick={pinScreenshotEdit} title="Pin to desktop (always on top)">
          <svg width="16" height="16" viewBox="0 0 16 16" fill="none">
            <path d="M10.5 2.5l3 3-1.5 1.5-1-0.2-2.8 2.8-.2 2.4-1.2 1.2-1.5-1.5L3 14l2.3-2.8-1.5-1.5 1.2-1.2 2.4-.2 2.8-2.8-.2-1L10.5 2.5z" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round" fill="none"/>
          </svg>
        </button>

        <!-- Undo -->
        <button class="ss-tool" onclick={undoAnnotation} title="Undo" disabled={annotations.length === 0}>
          <svg width="16" height="16" viewBox="0 0 16 16" fill="none">
            <path d="M4 6l-3 3 3 3" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/>
            <path d="M1 9h9a4 4 0 010 0 4 4 0 000-8H7" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" fill="none"/>
          </svg>
        </button>

        <div class="ss-sep"></div>

        <!-- Cancel -->
        <button class="ss-action ss-cancel" onclick={cancel} title="Cancel (Esc)">
          <svg width="14" height="14" viewBox="0 0 14 14" fill="none">
            <path d="M3 3l8 8M11 3l-8 8" stroke="currentColor" stroke-width="2" stroke-linecap="round"/>
          </svg>
        </button>

        <!-- Done / Copy to clipboard -->
        <button class="ss-action ss-done" onclick={confirmScreenshotEdit} title="Copy to clipboard (Enter)">
          <svg width="14" height="14" viewBox="0 0 14 14" fill="none">
            <path d="M2.5 7.5l3 3 6.5-7" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
          </svg>
        </button>
      </div>
    {/if}
  {:else}
    <!-- Full screen dim when no selection yet -->
    <div class="dim" style="top:0;left:0;right:0;bottom:0;"></div>
  {/if}

  <!-- Crosshair lines (idle phase, follows mouse) -->
  {#if phase === 'idle'}
    <div class="crosshair-h" style="top:{mouseY}px;"></div>
    <div class="crosshair-v" style="left:{mouseX}px;"></div>
    <div class="crosshair-coord" style="left:{mouseX + 14}px;top:{mouseY + 14}px;">
      {Math.round(mouseX)}, {Math.round(mouseY)}
    </div>
  {/if}

  <!-- Instructions -->
  {#if phase === 'idle'}
    <div class="instructions">
      {#if isScrollScreenshot}
        Click and drag to select the visible area for scrolling capture<br />
      {:else if isScreenshot}
        Click and drag to capture region · Click for fullscreen screenshot<br />
      {:else}
        Click and drag to select region · Click to record fullscreen<br />
      {/if}
      <span class="hint">Press ESC to cancel</span>
    </div>
  {/if}

  {#if phase === 'adjusting'}
    <div class="instructions-bottom">
      Drag to move · Handles to resize · <b>Enter</b> to record video · <b>Esc</b> to reset
    </div>
  {/if}
</div>

<style>
  .overlay {
    position: fixed;
    top: 0;
    left: 0;
    width: 100vw;
    height: 100vh;
    z-index: 99999;
    user-select: none;
    -webkit-user-select: none;
    background: transparent;
  }

  .dim {
    position: absolute;
    background: rgba(0, 0, 0, 0.25);
    pointer-events: none;
    transition: background 0.15s ease;
  }

  /* ─── Selection box ─── */
  .selection {
    position: absolute;
    border: 2px solid rgba(59, 130, 246, 0.9);
    box-shadow:
      0 0 0 1px rgba(0, 0, 0, 0.2),
      inset 0 0 0 1px rgba(255, 255, 255, 0.1),
      0 0 20px rgba(59, 130, 246, 0.15);
    pointer-events: none;
    transition: border-color 0.15s ease, box-shadow 0.15s ease;
  }
  .selection.confirmed {
    border-color: rgba(59, 130, 246, 1);
    box-shadow:
      0 0 0 1px rgba(0, 0, 0, 0.3),
      inset 0 0 0 1px rgba(255, 255, 255, 0.1),
      0 0 30px rgba(59, 130, 246, 0.25);
  }
  .selection.too-small {
    border-color: rgba(255, 80, 60, 0.8);
    box-shadow:
      0 0 0 1px rgba(0, 0, 0, 0.2),
      0 0 15px rgba(255, 80, 60, 0.2);
  }

  /* ─── Dimensions label ─── */
  .dimensions {
    position: absolute;
    transform: translateX(-50%);
    background: rgba(0, 0, 0, 0.85);
    color: #e2e8f0;
    padding: 4px 12px;
    border-radius: 6px;
    font-family: ui-monospace, 'SF Mono', 'Menlo', monospace;
    font-size: 12px;
    white-space: nowrap;
    pointer-events: none;
    backdrop-filter: blur(8px);
    -webkit-backdrop-filter: blur(8px);
    border: 1px solid rgba(255, 255, 255, 0.1);
  }
  .too-small-label {
    color: #fca5a5;
  }
  .min-hint {
    color: #f87171;
    margin-left: 6px;
    font-size: 10px;
  }

  /* ─── Resize handles ─── */
  .handle {
    position: absolute;
    width: 8px;
    height: 8px;
    background: white;
    border: 1.5px solid rgba(59, 130, 246, 0.9);
    border-radius: 2px;
    transform: translate(-50%, -50%);
    pointer-events: auto;
    z-index: 10;
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.3);
    transition: transform 0.1s ease;
  }
  .handle:hover {
    transform: translate(-50%, -50%) scale(1.3);
  }

  /* ═══ Settings Toolbar (centered in selection) ═══ */
  .toolbar {
    position: absolute;
    transform: translate(-50%, -50%);
    display: flex;
    flex-direction: column;
    gap: 4px;
    pointer-events: auto;
    z-index: 20;
    animation: toolbar-in 0.2s ease-out;
  }
  @keyframes toolbar-in {
    from { opacity: 0; transform: translate(-50%, -50%) scale(0.95); }
    to   { opacity: 1; transform: translate(-50%, -50%) scale(1); }
  }

  .toolbar-row {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 5px 8px;
    background: rgba(30, 30, 30, 0.92);
    backdrop-filter: blur(20px);
    -webkit-backdrop-filter: blur(20px);
    border: 1px solid rgba(255, 255, 255, 0.12);
    border-radius: 10px;
    box-shadow:
      0 4px 24px rgba(0, 0, 0, 0.5),
      0 1px 4px rgba(0, 0, 0, 0.3);
  }

  .toolbar-sep {
    width: 1px;
    height: 20px;
    background: rgba(255, 255, 255, 0.12);
    flex-shrink: 0;
  }

  .toolbar-icon {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    color: rgba(255, 255, 255, 0.6);
    flex-shrink: 0;
  }

  /* Dimension inputs */
  .dim-input {
    width: 52px;
    height: 26px;
    background: rgba(255, 255, 255, 0.08);
    border: 1px solid rgba(255, 255, 255, 0.15);
    border-radius: 5px;
    color: #e2e8f0;
    font-family: ui-monospace, 'SF Mono', 'Menlo', monospace;
    font-size: 12px;
    text-align: center;
    padding: 0 4px;
    outline: none;
    transition: border-color 0.15s, background 0.15s;
  }
  .dim-input:focus {
    border-color: rgba(59, 130, 246, 0.7);
    background: rgba(255, 255, 255, 0.12);
  }
  .dim-input:hover:not(:focus) {
    background: rgba(255, 255, 255, 0.1);
  }

  .dim-times {
    color: rgba(255, 255, 255, 0.4);
    font-size: 13px;
    font-weight: 300;
    user-select: none;
  }

  /* Toolbar buttons */
  .toolbar-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    background: transparent;
    border: none;
    color: rgba(255, 255, 255, 0.6);
    cursor: pointer;
    padding: 4px;
    border-radius: 5px;
    transition: all 0.12s ease;
  }
  .toolbar-btn:hover {
    background: rgba(255, 255, 255, 0.1);
    color: rgba(255, 255, 255, 0.9);
  }

  .aspect-btn.active {
    color: rgba(59, 130, 246, 0.9);
  }
  .aspect-btn.active:hover {
    color: rgba(59, 130, 246, 1);
    background: rgba(59, 130, 246, 0.15);
  }

  .cancel-btn {
    color: rgba(255, 255, 255, 0.45);
  }
  .cancel-btn:hover {
    color: rgba(255, 100, 100, 0.9);
    background: rgba(255, 100, 100, 0.1);
  }

  /* Audio buttons */
  .audio-btn {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 4px 8px;
    border-radius: 6px;
    font-family: -apple-system, BlinkMacSystemFont, 'SF Pro Text', sans-serif;
    font-size: 11px;
    border: none;
    background: transparent;
    color: rgba(255, 255, 255, 0.45);
    cursor: pointer;
    transition: all 0.12s ease;
  }
  .audio-btn:hover {
    background: rgba(255, 255, 255, 0.08);
    color: rgba(255, 255, 255, 0.7);
  }
  .audio-btn.active {
    color: rgba(52, 211, 153, 0.95);
  }
  .audio-btn.active:hover {
    background: rgba(52, 211, 153, 0.12);
    color: rgba(52, 211, 153, 1);
  }

  .audio-label {
    font-size: 11px;
    font-weight: 500;
    line-height: 1;
  }

  /* Quality toggle */
  .quality-toggle {
    display: flex;
    background: rgba(255, 255, 255, 0.06);
    border-radius: 5px;
    overflow: hidden;
    border: 1px solid rgba(255, 255, 255, 0.1);
  }
  .quality-opt {
    padding: 3px 8px;
    font-family: -apple-system, BlinkMacSystemFont, 'SF Pro Text', sans-serif;
    font-size: 10px;
    font-weight: 600;
    letter-spacing: 0.5px;
    border: none;
    background: transparent;
    color: rgba(255, 255, 255, 0.4);
    cursor: pointer;
    transition: all 0.12s ease;
  }
  .quality-opt:hover:not(.active) {
    color: rgba(255, 255, 255, 0.65);
    background: rgba(255, 255, 255, 0.06);
  }
  .quality-opt.active {
    background: rgba(59, 130, 246, 0.25);
    color: rgba(59, 130, 246, 1);
  }

  /* Record GIF button */
  .btn-record-gif {
    display: flex;
    align-items: center;
    gap: 5px;
    padding: 5px 12px;
    border: 1px solid rgba(167, 139, 250, 0.35);
    border-radius: 8px;
    background: rgba(139, 92, 246, 0.15);
    color: rgba(196, 181, 253, 0.95);
    font-family: -apple-system, BlinkMacSystemFont, 'SF Pro Text', sans-serif;
    font-size: 12px;
    font-weight: 600;
    letter-spacing: 0.2px;
    cursor: pointer;
    transition: all 0.15s ease;
    white-space: nowrap;
  }
  .btn-record-gif:hover {
    background: rgba(139, 92, 246, 0.28);
    border-color: rgba(167, 139, 250, 0.55);
    color: #e0d4ff;
  }
  .btn-record-gif:active {
    transform: scale(0.97);
    background: rgba(139, 92, 246, 0.35);
  }

  /* Record Video button */
  .btn-record-video {
    display: flex;
    align-items: center;
    gap: 5px;
    padding: 5px 14px;
    border: 1px solid rgba(96, 165, 250, 0.35);
    border-radius: 8px;
    background: rgba(59, 130, 246, 0.18);
    color: rgba(147, 197, 253, 0.95);
    font-family: -apple-system, BlinkMacSystemFont, 'SF Pro Text', sans-serif;
    font-size: 12px;
    font-weight: 600;
    letter-spacing: 0.2px;
    cursor: pointer;
    transition: all 0.15s ease;
    white-space: nowrap;
  }
  .btn-record-video:hover {
    background: rgba(59, 130, 246, 0.3);
    border-color: rgba(96, 165, 250, 0.55);
    color: #dbeafe;
  }
  .btn-record-video:active {
    transform: scale(0.97);
    background: rgba(59, 130, 246, 0.38);
  }

  /* Screenshot button */
  .btn-screenshot {
    display: flex;
    align-items: center;
    gap: 5px;
    padding: 5px 14px;
    border: 1px solid rgba(52, 211, 153, 0.35);
    border-radius: 8px;
    background: rgba(16, 185, 129, 0.18);
    color: rgba(167, 243, 208, 0.95);
    font-family: -apple-system, BlinkMacSystemFont, 'SF Pro Text', sans-serif;
    font-size: 12px;
    font-weight: 600;
    letter-spacing: 0.2px;
    cursor: pointer;
    transition: all 0.15s ease;
    white-space: nowrap;
  }
  .btn-screenshot:hover {
    background: rgba(16, 185, 129, 0.3);
    border-color: rgba(52, 211, 153, 0.55);
    color: #d1fae5;
  }
  .btn-screenshot:active {
    transform: scale(0.97);
    background: rgba(16, 185, 129, 0.38);
  }

  /* ─── Crosshair (idle) ─── */
  .crosshair-h {
    position: absolute;
    left: 0;
    right: 0;
    height: 1px;
    background: rgba(255, 255, 255, 0.4);
    pointer-events: none;
    z-index: 5;
  }
  .crosshair-v {
    position: absolute;
    top: 0;
    bottom: 0;
    width: 1px;
    background: rgba(255, 255, 255, 0.4);
    pointer-events: none;
    z-index: 5;
  }
  .crosshair-coord {
    position: absolute;
    color: rgba(255, 255, 255, 0.7);
    font-family: ui-monospace, 'SF Mono', 'Menlo', monospace;
    font-size: 11px;
    pointer-events: none;
    z-index: 5;
    white-space: nowrap;
  }

  /* ─── Instructions ─── */
  .instructions {
    position: absolute;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    color: white;
    font-family: -apple-system, BlinkMacSystemFont, 'SF Pro Text', sans-serif;
    font-size: 18px;
    text-align: center;
    pointer-events: none;
    text-shadow: 0 1px 6px rgba(0, 0, 0, 0.6);
  }
  .hint {
    font-size: 13px;
    opacity: 0.6;
    margin-top: 6px;
    display: inline-block;
  }

  .instructions-bottom {
    position: absolute;
    bottom: 24px;
    left: 50%;
    transform: translateX(-50%);
    color: rgba(255, 255, 255, 0.7);
    font-family: -apple-system, BlinkMacSystemFont, 'SF Pro Text', sans-serif;
    font-size: 13px;
    text-align: center;
    pointer-events: none;
    text-shadow: 0 1px 4px rgba(0, 0, 0, 0.5);
    background: rgba(0, 0, 0, 0.5);
    padding: 6px 16px;
    border-radius: 8px;
    backdrop-filter: blur(8px);
    -webkit-backdrop-filter: blur(8px);
  }

  /* ─── Preset dropdown ─── */
  .preset-wrapper {
    position: relative;
  }

  .preset-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 24px;
    padding: 0;
  }
  .preset-btn.active {
    color: rgba(59, 130, 246, 0.9);
    background: rgba(59, 130, 246, 0.12);
  }

  .chevron-up {
    transition: transform 0.15s ease;
  }
  .chevron-up.open {
    transform: rotate(180deg);
  }

  .preset-menu {
    position: absolute;
    bottom: calc(100% + 8px);
    left: 50%;
    transform: translateX(-50%);
    min-width: 230px;
    background: rgba(30, 30, 30, 0.95);
    backdrop-filter: blur(20px);
    -webkit-backdrop-filter: blur(20px);
    border: 1px solid rgba(255, 255, 255, 0.12);
    border-radius: 10px;
    box-shadow:
      0 -4px 32px rgba(0, 0, 0, 0.5),
      0 -1px 8px rgba(0, 0, 0, 0.3);
    padding: 4px;
    z-index: 100;
    animation: preset-in 0.15s ease-out;
  }
  @keyframes preset-in {
    from { opacity: 0; transform: translateX(-50%) translateY(4px); }
    to   { opacity: 1; transform: translateX(-50%) translateY(0); }
  }

  .preset-item {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 7px 10px;
    border: none;
    border-radius: 7px;
    background: transparent;
    color: rgba(255, 255, 255, 0.85);
    font-family: -apple-system, BlinkMacSystemFont, 'SF Pro Text', sans-serif;
    font-size: 12px;
    cursor: pointer;
    transition: background 0.1s ease;
    text-align: left;
  }
  .preset-item:hover {
    background: rgba(255, 255, 255, 0.1);
  }
  .preset-item:active {
    background: rgba(59, 130, 246, 0.2);
  }

  .preset-icon {
    width: 18px;
    text-align: center;
    font-size: 14px;
    color: rgba(255, 255, 255, 0.5);
    flex-shrink: 0;
  }

  .preset-label {
    flex: 1;
    font-weight: 500;
  }

  .preset-size {
    font-family: ui-monospace, 'SF Mono', 'Menlo', monospace;
    font-size: 11px;
    color: rgba(255, 255, 255, 0.4);
    flex-shrink: 0;
  }

  /* ─── Camera dropdown menu ─── */
  .camera-menu {
    position: absolute;
    bottom: calc(100% + 8px);
    left: 50%;
    transform: translateX(-50%);
    min-width: 200px;
    max-height: 400px;
    overflow-y: auto;
    background: rgba(30, 30, 30, 0.95);
    backdrop-filter: blur(20px);
    -webkit-backdrop-filter: blur(20px);
    border: 1px solid rgba(255, 255, 255, 0.12);
    border-radius: 10px;
    box-shadow:
      0 -4px 32px rgba(0, 0, 0, 0.5),
      0 -1px 8px rgba(0, 0, 0, 0.3);
    padding: 4px;
    z-index: 100;
    animation: preset-in 0.15s ease-out;
  }

  .camera-menu-section {
    padding: 6px 10px 2px;
    font-size: 10px;
    font-weight: 600;
    color: rgba(255, 255, 255, 0.35);
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }

  .camera-menu-item {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    padding: 5px 10px;
    border: none;
    border-radius: 6px;
    background: transparent;
    color: rgba(255, 255, 255, 0.85);
    font-family: -apple-system, BlinkMacSystemFont, 'SF Pro Text', sans-serif;
    font-size: 12px;
    cursor: pointer;
    transition: background 0.1s ease;
    text-align: left;
  }

  .camera-menu-item:hover {
    background: rgba(255, 255, 255, 0.1);
  }

  .camera-menu-check {
    width: 14px;
    font-size: 11px;
    flex-shrink: 0;
    text-align: center;
    color: #0a84ff;
  }

  .camera-menu-hint {
    padding: 5px 10px 5px 30px;
    font-size: 11px;
    color: rgba(255, 255, 255, 0.3);
    font-style: italic;
  }

  .camera-menu-divider {
    height: 1px;
    background: rgba(255, 255, 255, 0.1);
    margin: 4px 8px;
  }

  /* ═══════════════════════════════════════════════════════════════
     Frozen Screen Background + Screenshot Toolbar + Annotations
     ═══════════════════════════════════════════════════════════════ */

  :global(.frozen-bg) {
    position: fixed;
    top: 0;
    left: 0;
    width: 100vw;
    height: 100vh;
    z-index: 99998;
    object-fit: cover;
    pointer-events: none;
    user-select: none;
    -webkit-user-select: none;
  }

  .annotation-canvas {
    position: absolute;
    z-index: 12;
  }

  .text-annot-input {
    position: absolute;
    z-index: 13;
    min-width: 60px;
    min-height: 24px;
    max-width: 400px;
    background: rgba(0, 0, 0, 0.45);
    border: 1.5px dashed rgba(255, 255, 255, 0.35);
    border-radius: 4px;
    padding: 4px 6px;
    font-size: 16px;
    font-family: 'Segoe Print', 'Comic Sans MS', 'Bradley Hand', cursive;
    line-height: 1.3;
    outline: none;
    resize: none;
    overflow: hidden;
    field-sizing: content;
  }
  .text-annot-input::placeholder {
    color: rgba(255, 255, 255, 0.3);
  }

  /* ─── Screenshot floating toolbar ─── */
  .ss-toolbar {
    position: absolute;
    transform: translateX(-100%);
    display: flex;
    align-items: center;
    gap: 3px;
    padding: 4px 6px;
    background: rgba(24, 24, 28, 0.92);
    backdrop-filter: blur(24px);
    -webkit-backdrop-filter: blur(24px);
    border: 1px solid rgba(255, 255, 255, 0.1);
    border-radius: 10px;
    box-shadow:
      0 4px 20px rgba(0, 0, 0, 0.5),
      0 1px 4px rgba(0, 0, 0, 0.3);
    pointer-events: auto;
    z-index: 30;
    animation: ss-toolbar-in 0.18s ease-out;
  }
  @keyframes ss-toolbar-in {
    from { opacity: 0; transform: translateX(-100%) translateY(4px) scale(0.97); }
    to   { opacity: 1; transform: translateX(-100%) translateY(0) scale(1); }
  }

  .ss-tool {
    width: 28px;
    height: 28px;
    border-radius: 6px;
    border: none;
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    padding: 0;
    transition: all 0.12s ease;
    background: transparent;
    color: rgba(255, 255, 255, 0.5);
    flex-shrink: 0;
  }
  .ss-tool:hover:not(:disabled) {
    background: rgba(255, 255, 255, 0.1);
    color: rgba(255, 255, 255, 0.85);
  }
  .ss-tool.active {
    background: rgba(59, 130, 246, 0.2);
    color: rgba(96, 165, 250, 1);
  }
  .ss-tool:disabled {
    opacity: 0.3;
    cursor: default;
  }

  .ss-sep {
    width: 1px;
    height: 20px;
    background: rgba(255, 255, 255, 0.1);
    flex-shrink: 0;
    margin: 0 2px;
  }

  .ss-color {
    width: 16px;
    height: 16px;
    border-radius: 50%;
    border: 2px solid transparent;
    cursor: pointer;
    transition: all 0.12s ease;
    padding: 0;
    flex-shrink: 0;
  }
  .ss-color:hover {
    transform: scale(1.15);
  }
  .ss-color.active {
    border-color: rgba(255, 255, 255, 0.8);
    box-shadow: 0 0 0 1px rgba(0, 0, 0, 0.3);
  }

  .ss-action {
    width: 30px;
    height: 30px;
    border-radius: 8px;
    border: none;
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    padding: 0;
    transition: all 0.12s ease;
    flex-shrink: 0;
  }

  .ss-cancel {
    background: rgba(255, 69, 58, 0.15);
    color: rgba(255, 69, 58, 0.8);
  }
  .ss-cancel:hover {
    background: rgba(255, 69, 58, 0.3);
    color: rgba(255, 69, 58, 1);
  }

  .ss-done {
    background: rgba(48, 209, 88, 0.18);
    color: rgba(48, 209, 88, 0.85);
  }
  .ss-done:hover {
    background: rgba(48, 209, 88, 0.35);
    color: rgba(48, 209, 88, 1);
  }
</style>
