<script>
  import { invoke } from '@tauri-apps/api/core';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import { convertFileSrc } from '@tauri-apps/api/core';

  const params = new URLSearchParams(window.location.search);
  const imagePath = decodeURIComponent(params.get('path') || '');
  const windowLabel = params.get('label') || '';

  let imgSrc = $state('');
  let zoom = $state(1.0);
  let showTopbar = $state(false);

  if (imagePath) {
    imgSrc = convertFileSrc(imagePath);
  }

  // Transparent windows on macOS ignore mouse events by default — must opt in
  getCurrentWindow().setIgnoreCursorEvents(false).catch(() => {});


  function onWheel(e) {
    e.preventDefault();
    const delta = e.deltaY > 0 ? -0.1 : 0.1;
    zoom = Math.max(0.3, Math.min(5.0, zoom + delta));
  }

  async function closePin() {
    try {
      await invoke('close_pin_window', { label: windowLabel });
    } catch (e) {
      try { window.close(); } catch {}
    }
  }

  async function copyImage() {
    try {
      await invoke('copy_screenshot', { path: imagePath });
    } catch (e) {
      console.error('Copy failed:', e);
    }
  }

  async function saveImage() {
    try {
      const saved = await invoke('save_screenshot', { path: imagePath });
      await invoke('reveal_in_finder', { path: saved });
    } catch (e) {
      console.error('Save failed:', e);
    }
  }

  async function startDrag(e) {
    if (e.button !== 0) return;
    if (e.target.closest('button') || e.target.closest('.toolbar')) return;
    e.preventDefault();
    e.stopPropagation();
    try {
      await getCurrentWindow().startDragging();
    } catch (err) {
      console.error('[pinned] startDragging failed:', err);
    }
  }
</script>

<svelte:window
  onkeydown={(e) => {
    if (e.key === 'Escape') closePin();
  }}
/>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="pin-window"
  onmouseenter={() => showTopbar = true}
  onmouseleave={() => showTopbar = false}
  onmousedown={startDrag}
>
  <!-- Image area -->
  <div class="image-area" onwheel={onWheel}>
    {#if imgSrc}
      <img
        src={imgSrc}
        alt="Pinned"
        class="pinned-image"
        style="transform:scale({zoom})"
        draggable="false"
      />
    {:else}
      <div class="empty">No image</div>
    {/if}
  </div>

  <!-- Always-visible close button (no-drag so it's clickable) -->
  <button class="close-corner" onclick={closePin} title="Close (Esc)">
    <svg width="8" height="8" viewBox="0 0 10 10" fill="none">
      <path d="M1.5 1.5L8.5 8.5M8.5 1.5L1.5 8.5" stroke="currentColor" stroke-width="2" stroke-linecap="round"/>
    </svg>
  </button>

  <!-- Resize grip -->
  <div class="resize-grip">
    <svg width="10" height="10" viewBox="0 0 10 10" fill="none">
      <path d="M9 1L1 9M9 4L4 9M9 7L7 9" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"/>
    </svg>
  </div>

  <!-- Floating toolbar on hover -->
  {#if showTopbar}
    <div class="toolbar">
      <button class="tool-btn close-btn" onclick={closePin} title="Close (Esc)">
        <svg width="8" height="8" viewBox="0 0 10 10" fill="none">
          <path d="M1.5 1.5L8.5 8.5M8.5 1.5L1.5 8.5" stroke="currentColor" stroke-width="2" stroke-linecap="round"/>
        </svg>
      </button>
      <button class="tool-btn" onclick={copyImage} title="Copy">
        <svg width="10" height="10" viewBox="0 0 12 12" fill="none">
          <rect x="4" y="4" width="7" height="7" rx="1" stroke="currentColor" stroke-width="1.3"/>
          <path d="M2 8V2a1 1 0 011-1h6" stroke="currentColor" stroke-width="1.3"/>
        </svg>
      </button>
      <button class="tool-btn" onclick={saveImage} title="Save">
        <svg width="10" height="10" viewBox="0 0 12 12" fill="none">
          <path d="M2 8v2a1 1 0 001 1h6a1 1 0 001-1V8" stroke="currentColor" stroke-width="1.3" stroke-linecap="round"/>
          <path d="M6 1v7M6 8L3.5 5.5M6 8L8.5 5.5" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round"/>
        </svg>
      </button>
    </div>
  {/if}
</div>

<style>
  .pin-window {
    position: relative;
    width: 100%;
    height: 100%;
    overflow: hidden;
    background: transparent;
    border: 1px solid rgba(255, 255, 255, 0.08);
    box-sizing: border-box;
    cursor: grab;
    user-select: none;
    -webkit-user-select: none;
  }
  .pin-window:active { cursor: grabbing; }

  /* Toolbar — explicitly no-drag */
  .toolbar {
    position: absolute;
    top: 6px;
    left: 6px;
    z-index: 10;
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 3px 4px;
    background: rgba(0, 0, 0, 0.7);
    backdrop-filter: blur(12px);
    -webkit-backdrop-filter: blur(12px);
    border-radius: 8px;
    -webkit-app-region: no-drag;
    animation: fadeIn 0.15s ease;
  }

  @keyframes fadeIn {
    from { opacity: 0; transform: translateY(-3px); }
    to { opacity: 1; transform: translateY(0); }
  }

  .tool-btn {
    width: 22px;
    height: 22px;
    border-radius: 5px;
    border: none;
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    padding: 0;
    transition: all 0.1s ease;
    background: transparent;
    color: rgba(255, 255, 255, 0.6);
    -webkit-app-region: no-drag;
  }

  .tool-btn:hover {
    background: rgba(255, 255, 255, 0.15);
    color: #fff;
  }

  .close-btn:hover {
    background: rgba(255, 59, 48, 0.8);
    color: #fff;
  }

  /* Always-visible close button — no-drag so clicks work */
  .close-corner {
    position: absolute;
    top: 6px;
    right: 6px;
    width: 20px;
    height: 20px;
    border-radius: 50%;
    border: none;
    background: rgba(0, 0, 0, 0.5);
    color: rgba(255, 255, 255, 0.6);
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    z-index: 20;
    transition: all 0.15s ease;
    padding: 0;
    -webkit-app-region: no-drag;
  }
  .close-corner:hover {
    background: rgba(255, 59, 48, 0.8);
    color: #fff;
    transform: scale(1.1);
  }

  /* Image area */
  .image-area {
    width: 100%;
    height: 100%;
    overflow: hidden;
    display: flex;
    align-items: center;
    justify-content: center;
    background: #0a0a0a;
  }

  .pinned-image {
    max-width: 100%;
    max-height: 100%;
    object-fit: contain;
    display: block;
    transform-origin: center center;
    pointer-events: none;
    user-select: none;
    -webkit-user-select: none;
  }

  .empty {
    color: rgba(255, 255, 255, 0.3);
    font-size: 12px;
    font-family: -apple-system, BlinkMacSystemFont, sans-serif;
  }

  .resize-grip {
    position: absolute;
    bottom: 3px;
    right: 3px;
    color: rgba(255, 255, 255, 0.2);
    pointer-events: none;
    z-index: 5;
    transition: color 0.15s ease;
  }
  .pin-window:hover .resize-grip {
    color: rgba(255, 255, 255, 0.4);
  }
</style>
