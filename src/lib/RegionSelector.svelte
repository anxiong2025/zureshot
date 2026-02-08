<script>
  import { invoke } from '@tauri-apps/api/core';

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

  // ─── Mode detection: 'record' or 'screenshot' ───
  const urlParams = new URLSearchParams(window.location.search);
  const mode = urlParams.get('mode') || 'record';  // default to record
  const isScreenshot = mode === 'screenshot';

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
      : phase === 'adjusting'
  );

  let isTooSmall = $derived(false); // Click without drag now selects fullscreen, so never show "too small"

  // ─── Resize handles definition ───
  let handles = $derived(phase === 'adjusting' ? [
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
    // Close preset menu on any click outside it
    if (presetOpen) {
      presetOpen = false;
    }

    if (phase === 'idle') {
      // Start drawing a new selection
      startX = e.clientX;
      startY = e.clientY;
      currentX = e.clientX;
      currentY = e.clientY;
      phase = 'drawing';
    } else if (phase === 'adjusting') {
      // Check if clicking on a resize handle
      const handle = hitTestHandle(e.clientX, e.clientY);
      if (handle) {
        startDrag(handle, e);
        return;
      }
      // Check if clicking inside selection → move
      if (isInsideSelection(e.clientX, e.clientY)) {
        startDrag('move', e);
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
        phase = 'adjusting';
        return;
      }

      // Commit to adjusting phase
      selX = finalRect.x;
      selY = finalRect.y;
      selW = finalRect.width;
      selH = finalRect.height;
      phase = 'adjusting';
    } else if (dragType) {
      dragType = null;
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
    if (phase === 'adjusting' && !dragType) {
      const h = hitTestHandle(mx, my);
      if (h) {
        const handle = handles.find(hh => hh.id === h);
        return handle ? handle.cursor : 'crosshair';
      }
      if (isInsideSelection(mx, my)) return 'move';
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
      if (isScreenshot) {
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
        });
      }
    } catch (e) {
      console.error('Failed to confirm selection:', e);
    }
  }

  async function cancel() {
    try {
      await invoke('cancel_region_selection');
    } catch (e) {
      console.error('Failed to cancel region selection:', e);
    }
  }

  function onKeyDown(e) {
    // Don't handle when typing in dimension inputs
    if (e.target.tagName === 'INPUT') return;

    if (e.key === 'Escape') {
      if (presetOpen) {
        presetOpen = false;
        return;
      }
      if (phase === 'adjusting') {
        // Go back to idle
        phase = 'idle';
      } else {
        cancel();
      }
    } else if (e.key === 'Enter' && phase === 'adjusting') {
      confirmSelection(isScreenshot ? 'screenshot' : 'video');
    }
  }
</script>

<svelte:window onkeydown={onKeyDown} />

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

        <!-- Row 2: Audio toggles + Record button (record mode) or Screenshot button -->
        {#if isScreenshot}
        <div class="toolbar-row toolbar-bottom">
          <!-- Screenshot capture button -->
          <button class="btn-screenshot" onclick={() => confirmSelection('screenshot')}>
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M23 19a2 2 0 01-2 2H3a2 2 0 01-2-2V8a2 2 0 012-2h4l2-3h6l2 3h4a2 2 0 012 2z"/>
              <circle cx="12" cy="13" r="4"/>
            </svg>
            Capture
          </button>
        </div>
        {:else}
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
      {#if isScreenshot}
        Click and drag to select region · Click for fullscreen screenshot<br />
      {:else}
        Click and drag to select region · Click to record fullscreen<br />
      {/if}
      <span class="hint">Press ESC to cancel</span>
    </div>
  {/if}

  {#if phase === 'adjusting'}
    <div class="instructions-bottom">
      {#if isScreenshot}
        Drag to move · Handles to resize · <b>Enter</b> to capture · <b>Esc</b> to reset
      {:else}
        Drag to move · Handles to resize · <b>Enter</b> to record video · <b>Esc</b> to reset
      {/if}
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
</style>
