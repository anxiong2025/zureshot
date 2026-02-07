<script>
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import { getCurrentWindow } from '@tauri-apps/api/window';

  let result = $state(null);
  let visible = $state(false);
  let autoHideTimer = null;

  onMount(async () => {
    // Listen for recording stopped event
    await listen('recording-stopped', async (event) => {
      result = event.payload;
      visible = true;
      
      // Show the window
      const win = getCurrentWindow();
      await win.show();
      await win.setFocus();

      // Position window in bottom-right corner
      const primaryMonitor = await win.availableMonitors();
      if (primaryMonitor.length > 0) {
        const monitor = primaryMonitor[0];
        const x = monitor.size.width - 340;
        const y = monitor.size.height - 240;
        await win.setPosition({ x, y, type: 'Physical' });
      }

      // Auto-hide after 5 seconds
      if (autoHideTimer) clearTimeout(autoHideTimer);
      autoHideTimer = setTimeout(hideWindow, 5000);
    });
  });

  async function hideWindow() {
    visible = false;
    const win = getCurrentWindow();
    await win.hide();
  }

  async function openFile() {
    if (result?.path) {
      await invoke('reveal_in_finder', { path: result.path });
    }
    hideWindow();
  }

  async function copyPath() {
    if (result?.path) {
      await navigator.clipboard.writeText(result.path);
    }
  }

  function formatSize(bytes) {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  }

  function formatDuration(secs) {
    const mins = Math.floor(secs / 60);
    const s = Math.floor(secs % 60);
    return `${mins}:${s.toString().padStart(2, '0')}`;
  }

  function getFilename(path) {
    return path?.split('/').pop() || 'Recording';
  }
</script>

{#if visible && result}
  <div class="thumbnail-card" role="dialog" aria-label="Recording complete">
    <div class="preview">
      <div class="play-icon">
        <svg width="32" height="32" viewBox="0 0 24 24" fill="white">
          <path d="M8 5v14l11-7z"/>
        </svg>
      </div>
    </div>
    
    <div class="info">
      <div class="filename">{getFilename(result.path)}</div>
      <div class="meta">
        {formatDuration(result.duration_secs)} • {formatSize(result.file_size_bytes)}
      </div>
    </div>

    <div class="actions">
      <button class="action-btn" onclick={openFile} title="Show in Finder">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor">
          <path d="M20 6h-8l-2-2H4c-1.1 0-2 .9-2 2v12c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2V8c0-1.1-.9-2-2-2z"/>
        </svg>
      </button>
      <button class="action-btn" onclick={copyPath} title="Copy path">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor">
          <path d="M16 1H4c-1.1 0-2 .9-2 2v14h2V3h12V1zm3 4H8c-1.1 0-2 .9-2 2v14c0 1.1.9 2 2 2h11c1.1 0 2-.9 2-2V7c0-1.1-.9-2-2-2zm0 16H8V7h11v14z"/>
        </svg>
      </button>
      <button class="action-btn close" onclick={hideWindow} title="Close">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor">
          <path d="M19 6.41L17.59 5 12 10.59 6.41 5 5 6.41 10.59 12 5 17.59 6.41 19 12 13.41 17.59 19 19 17.59 13.41 12z"/>
        </svg>
      </button>
    </div>
  </div>
{/if}

<style>
  .thumbnail-card {
    width: 300px;
    background: rgba(30, 30, 30, 0.95);
    backdrop-filter: blur(20px);
    border-radius: 12px;
    border: 1px solid rgba(255, 255, 255, 0.1);
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.4);
    overflow: hidden;
    font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
    animation: slideIn 0.3s ease-out;
  }

  @keyframes slideIn {
    from {
      opacity: 0;
      transform: translateY(20px) scale(0.95);
    }
    to {
      opacity: 1;
      transform: translateY(0) scale(1);
    }
  }

  .preview {
    height: 120px;
    background: linear-gradient(135deg, #1a1a2e 0%, #16213e 100%);
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    transition: background 0.2s;
  }

  .preview:hover {
    background: linear-gradient(135deg, #1f1f38 0%, #1a2848 100%);
  }

  .play-icon {
    width: 56px;
    height: 56px;
    background: rgba(255, 255, 255, 0.15);
    border-radius: 50%;
    display: flex;
    align-items: center;
    justify-content: center;
    transition: background 0.2s, transform 0.2s;
  }

  .preview:hover .play-icon {
    background: rgba(255, 255, 255, 0.25);
    transform: scale(1.1);
  }

  .info {
    padding: 12px 16px;
  }

  .filename {
    font-size: 14px;
    font-weight: 500;
    color: white;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .meta {
    font-size: 12px;
    color: rgba(255, 255, 255, 0.6);
    margin-top: 4px;
  }

  .actions {
    display: flex;
    gap: 4px;
    padding: 8px 12px 12px;
    border-top: 1px solid rgba(255, 255, 255, 0.05);
  }

  .action-btn {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    height: 32px;
    background: rgba(255, 255, 255, 0.08);
    border: none;
    border-radius: 6px;
    color: rgba(255, 255, 255, 0.8);
    cursor: pointer;
    transition: background 0.2s, color 0.2s;
  }

  .action-btn:hover {
    background: rgba(255, 255, 255, 0.15);
    color: white;
  }

  .action-btn.close:hover {
    background: rgba(255, 59, 48, 0.3);
    color: #ff6b6b;
  }
</style>
