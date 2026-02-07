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
  let systemAudioEnabled = $state(false);
  let selectedQuality = $state('standard'); // 'standard' | 'high'

  // Editable dimension input values (strings for input binding)
  let inputW = $state('');
  let inputH = $state('');
  let inputWFocused = $state(false);
  let inputHFocused = $state(false);

  // Sync inputs from selection rect when not focused
  $effect(() => {
    if (!inputWFocused) inputW = String(Math.round(selW));
  });
  $effect(() => {
    if (!inputHFocused) inputH = String(Math.round(selH));
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
    if (!isNaN(v) && v >= MIN_SIZE) {
      const oldW = selW;
      selW = Math.min(v, window.innerWidth - selX);
      if (aspectLocked && aspectRatio > 0) {
        selH = Math.round(selW / aspectRatio);
        selH = Math.min(selH, window.innerHeight - selY);
      }
    }
  }

  function onHeightInput(e) {
    inputH = e.target.value;
    const v = parseInt(inputH, 10);
    if (!isNaN(v) && v >= MIN_SIZE) {
      selH = Math.min(v, window.innerHeight - selY);
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
    } catch (e) {
      console.error('Failed to confirm region selection:', e);
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
      if (phase === 'adjusting') {
        // Go back to idle
        phase = 'idle';
      } else {
        cancel();
      }
    } else if (e.key === 'Enter' && phase === 'adjusting') {
      confirmSelection('video');
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
          {Math.round(rect.width)} &times; {Math.round(rect.height)}
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
          <!-- Settings icon -->
          <div class="toolbar-icon" title="Region settings">
            <svg width="16" height="16" viewBox="0 0 16 16" fill="none">
              <rect x="1" y="1" width="14" height="14" rx="2" stroke="currentColor" stroke-width="1.5" fill="none"/>
              <line x1="5" y1="1" x2="5" y2="15" stroke="currentColor" stroke-width="1" opacity="0.5"/>
              <line x1="11" y1="1" x2="11" y2="15" stroke="currentColor" stroke-width="1" opacity="0.5"/>
              <line x1="1" y1="5" x2="15" y2="5" stroke="currentColor" stroke-width="1" opacity="0.5"/>
              <line x1="1" y1="11" x2="15" y2="11" stroke="currentColor" stroke-width="1" opacity="0.5"/>
            </svg>
          </div>

          <div class="toolbar-sep"></div>

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

          <!-- Aspect ratio lock -->
          <button
            class="toolbar-btn aspect-btn"
            class:active={aspectLocked}
            title={aspectLocked ? 'Unlock aspect ratio' : 'Lock aspect ratio'}
            onclick={toggleAspectLock}
          >
            {#if aspectLocked}
              <svg width="14" height="14" viewBox="0 0 14 14" fill="none">
                <rect x="2" y="6" width="10" height="7" rx="1.5" stroke="currentColor" stroke-width="1.5" fill="none"/>
                <path d="M4.5 6V4.5a2.5 2.5 0 015 0V6" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" fill="none"/>
              </svg>
            {:else}
              <svg width="14" height="14" viewBox="0 0 14 14" fill="none">
                <rect x="2" y="6" width="10" height="7" rx="1.5" stroke="currentColor" stroke-width="1.5" fill="none"/>
                <path d="M4.5 6V4.5a2.5 2.5 0 015 0" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" fill="none"/>
              </svg>
            {/if}
          </button>

          <div class="toolbar-sep"></div>

          <!-- Cancel button -->
          <button class="toolbar-btn cancel-btn" onclick={cancel} title="Cancel (Esc)">
            <svg width="12" height="12" viewBox="0 0 12 12" fill="none">
              <path d="M2 2l8 8M10 2l-8 8" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/>
            </svg>
          </button>
        </div>

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
            <span class="gif-badge">GIF</span>
            Record GIF
          </button>

          <!-- Record Video button -->
          <button class="btn-record-video" onclick={() => confirmSelection('video')}>
            <span class="rec-dot"></span>
            Record Video
          </button>
        </div>
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
      Click and drag to select region · Click to record fullscreen<br />
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
    gap: 6px;
    padding: 6px 14px;
    border: none;
    border-radius: 8px;
    background: rgba(139, 92, 246, 0.9);
    color: white;
    font-family: -apple-system, BlinkMacSystemFont, 'SF Pro Text', sans-serif;
    font-size: 13px;
    font-weight: 600;
    cursor: pointer;
    transition: all 0.15s ease;
    box-shadow: 0 2px 8px rgba(139, 92, 246, 0.3);
    white-space: nowrap;
  }
  .btn-record-gif:hover {
    background: rgba(124, 58, 237, 1);
    transform: scale(1.02);
    box-shadow: 0 3px 12px rgba(139, 92, 246, 0.4);
  }
  .btn-record-gif:active {
    transform: scale(0.98);
  }

  .gif-badge {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 1px 4px;
    border-radius: 3px;
    background: rgba(255, 255, 255, 0.25);
    font-size: 9px;
    font-weight: 800;
    letter-spacing: 0.5px;
    line-height: 1.2;
    flex-shrink: 0;
  }

  /* Record Video button */
  .btn-record-video {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 16px;
    border: none;
    border-radius: 8px;
    background: rgba(239, 68, 68, 0.9);
    color: white;
    font-family: -apple-system, BlinkMacSystemFont, 'SF Pro Text', sans-serif;
    font-size: 13px;
    font-weight: 600;
    cursor: pointer;
    transition: all 0.15s ease;
    box-shadow: 0 2px 8px rgba(239, 68, 68, 0.3);
    white-space: nowrap;
  }
  .btn-record-video:hover {
    background: rgba(220, 38, 38, 1);
    transform: scale(1.02);
    box-shadow: 0 3px 12px rgba(239, 68, 68, 0.4);
  }
  .btn-record-video:active {
    transform: scale(0.98);
  }

  .rec-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: white;
    animation: pulse-dot 1.5s ease-in-out infinite;
    flex-shrink: 0;
  }
  @keyframes pulse-dot {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.5; }
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
</style>
