<script>
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import { PhysicalPosition } from '@tauri-apps/api/dpi';

  let screenshot = $state(null);
  let visible = $state(false);
  let autoHideTimer = null;
  let imgSrc = $state('');

  onMount(async () => {
    await listen('screenshot-taken', async (event) => {
      screenshot = event.payload;
      visible = true;

      // Use base64 data URL for reliable image loading
      if (screenshot.image_base64) {
        imgSrc = `data:image/png;base64,${screenshot.image_base64}`;
      }

      const win = getCurrentWindow();
      await win.show();
      await win.setFocus();

      // Position in bottom-left corner
      try {
        const monitors = await win.availableMonitors();
        if (monitors.length > 0) {
          const monitor = monitors[0];
          const x = 20;
          const y = monitor.size.height - 200;
          await win.setPosition(new PhysicalPosition(x, y));
        }
      } catch {}

      // Auto-hide after 6 seconds
      startAutoHide();
    });
  });

  function startAutoHide() {
    cancelAutoHide();
    autoHideTimer = setTimeout(dismiss, 6000);
  }

  function cancelAutoHide() {
    if (autoHideTimer) {
      clearTimeout(autoHideTimer);
      autoHideTimer = null;
    }
  }

  async function dismiss() {
    cancelAutoHide();
    visible = false;
    if (screenshot?.path) {
      try { await invoke('dismiss_screenshot', { path: screenshot.path }); } catch {}
    }
    screenshot = null;
    imgSrc = '';
    const win = getCurrentWindow();
    await win.hide();
  }

  async function saveScreenshot() {
    cancelAutoHide();
    if (screenshot?.path) {
      try {
        const saved = await invoke('save_screenshot', { path: screenshot.path });
        await invoke('reveal_in_finder', { path: saved });
      } catch (e) {
        console.error('Failed to save screenshot:', e);
      }
    }
    visible = false;
    screenshot = null;
    imgSrc = '';
    const win = getCurrentWindow();
    await win.hide();
  }

  async function copyScreenshot() {
    cancelAutoHide();
    if (screenshot?.path) {
      try {
        await invoke('copy_screenshot', { path: screenshot.path });
      } catch (e) {
        console.error('Failed to copy screenshot:', e);
      }
    }
    visible = false;
    screenshot = null;
    imgSrc = '';
    const win = getCurrentWindow();
    await win.hide();
  }

  async function pinScreenshot() {
    cancelAutoHide();
    if (screenshot?.path) {
      try {
        await invoke('pin_screenshot', { path: screenshot.path });
      } catch (e) {
        console.error('Failed to pin screenshot:', e);
      }
    }
    visible = false;
    screenshot = null;
    imgSrc = '';
    const win = getCurrentWindow();
    await win.hide();
  }

  let ocrText = $state('');
  let ocrLoading = $state(false);

  async function ocrScreenshot() {
    cancelAutoHide();
    if (!screenshot?.path) return;
    ocrLoading = true;
    try {
      const result = await invoke('ocr_screenshot', { path: screenshot.path });
      if (result.full_text) {
        // Copy recognized text to clipboard via a textarea trick
        const ta = document.createElement('textarea');
        ta.value = result.full_text;
        document.body.appendChild(ta);
        ta.select();
        document.execCommand('copy');
        document.body.removeChild(ta);
        ocrText = `Copied ${result.block_count} text blocks`;
        setTimeout(() => { ocrText = ''; }, 2000);
      } else {
        ocrText = 'No text found';
        setTimeout(() => { ocrText = ''; }, 2000);
      }
    } catch (e) {
      console.error('OCR failed:', e);
      ocrText = 'OCR failed';
      setTimeout(() => { ocrText = ''; }, 2000);
    }
    ocrLoading = false;
  }

  function formatSize(bytes) {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  }
</script>

{#if visible && screenshot}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="preview-card"
    onmouseenter={cancelAutoHide}
    onmouseleave={startAutoHide}
  >
    <!-- Screenshot thumbnail fills the card -->
    {#if imgSrc}
      <img src={imgSrc} alt="Screenshot" class="thumbnail" />
    {:else}
      <div class="placeholder"></div>
    {/if}

    <!-- Hover overlay with actions -->
    <div class="overlay">
      <!-- Close button top-right -->
      <button class="close-btn" onclick={dismiss} title="Dismiss">
        <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round">
          <path d="M18 6L6 18M6 6l12 12"/>
        </svg>
      </button>

      <!-- Center action buttons -->
      <div class="actions">
        <button class="action-btn" onclick={copyScreenshot} title="Copy to clipboard">
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <rect x="9" y="9" width="13" height="13" rx="2" ry="2"/>
            <path d="M5 15H4a2 2 0 01-2-2V4a2 2 0 012-2h9a2 2 0 012 2v1"/>
          </svg>
          Copy
        </button>
        <button class="action-btn" onclick={saveScreenshot} title="Save to Zureshot folder">
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M19 21H5a2 2 0 01-2-2V5a2 2 0 012-2h11l5 5v11a2 2 0 01-2 2z"/>
            <polyline points="17 21 17 13 7 13 7 21"/>
            <polyline points="7 3 7 8 15 8"/>
          </svg>
          Save
        </button>
        <button class="action-btn" onclick={pinScreenshot} title="Pin to desktop">
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M12 2L12 12"/>
            <path d="M18 8L18 12a6 6 0 01-12 0L6 8"/>
            <line x1="12" y1="17" x2="12" y2="22"/>
          </svg>
          Pin
        </button>
        <button class="action-btn" onclick={ocrScreenshot} title="Recognize text (OCR)" disabled={ocrLoading}>
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <polyline points="4 7 4 4 20 4 20 7"/>
            <line x1="9" y1="20" x2="15" y2="20"/>
            <line x1="12" y1="4" x2="12" y2="20"/>
          </svg>
          {ocrLoading ? '...' : 'OCR'}
        </button>
      </div>

      <!-- OCR status -->
      {#if ocrText}
        <div class="ocr-status">{ocrText}</div>
      {/if}

      <!-- Info at bottom -->
      <div class="info">
        {screenshot.width}×{screenshot.height} · {formatSize(screenshot.file_size_bytes)}
      </div>
    </div>
  </div>
{/if}

<style>
  .preview-card {
    position: relative;
    width: 300px;
    height: 180px;
    border-radius: 10px;
    overflow: hidden;
    box-shadow: 0 4px 20px rgba(0, 0, 0, 0.5);
    border: 1px solid rgba(255, 255, 255, 0.08);
    font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
    animation: slideIn 0.3s ease-out;
    cursor: default;
  }

  @keyframes slideIn {
    from {
      opacity: 0;
      transform: translateY(10px) scale(0.95);
    }
    to {
      opacity: 1;
      transform: translateY(0) scale(1);
    }
  }

  .thumbnail {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
  }

  .placeholder {
    width: 100%;
    height: 100%;
    background: #1a1a1a;
  }

  /* Hover overlay */
  .overlay {
    position: absolute;
    inset: 0;
    background: rgba(0, 0, 0, 0.55);
    backdrop-filter: blur(2px);
    -webkit-backdrop-filter: blur(2px);
    opacity: 0;
    transition: opacity 0.2s ease;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 8px;
  }

  .preview-card:hover .overlay {
    opacity: 1;
  }

  /* Close button */
  .close-btn {
    position: absolute;
    top: 8px;
    right: 8px;
    width: 22px;
    height: 22px;
    border-radius: 50%;
    border: none;
    background: rgba(255, 255, 255, 0.15);
    color: rgba(255, 255, 255, 0.8);
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    transition: all 0.15s ease;
    padding: 0;
  }
  .close-btn:hover {
    background: rgba(255, 59, 48, 0.6);
    color: #fff;
  }

  /* Action buttons */
  .actions {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
    justify-content: center;
  }

  .action-btn {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 5px 10px;
    border-radius: 16px;
    border: none;
    background: rgba(255, 255, 255, 0.18);
    color: #fff;
    font-family: -apple-system, BlinkMacSystemFont, 'SF Pro Text', sans-serif;
    font-size: 12px;
    font-weight: 600;
    cursor: pointer;
    transition: all 0.15s ease;
    white-space: nowrap;
    letter-spacing: 0.2px;
  }
  .action-btn:hover {
    background: rgba(255, 255, 255, 0.3);
    transform: scale(1.04);
  }
  .action-btn:active {
    transform: scale(0.97);
  }

  /* OCR status */
  .ocr-status {
    font-size: 10px;
    color: #30d158;
    font-weight: 600;
    text-align: center;
  }

  /* Info text */
  .info {
    font-size: 10px;
    color: rgba(255, 255, 255, 0.5);
    font-family: ui-monospace, 'SF Mono', 'Menlo', monospace;
    letter-spacing: 0.3px;
  }
</style>
