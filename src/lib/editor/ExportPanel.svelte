<script>
  import { invoke } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import { onMount } from 'svelte';

  /** @type {{
   *   videoPath: string,
   *   trimStart: number,
   *   trimEnd: number,
   *   duration: number,
   *   background: { type: string, colors?: string[], angle?: number, color?: string },
   *   padding: number,
   *   cornerRadius: number,
   *   shadowEnabled: boolean,
   *   zoomKeyframes: Array<{time: number, zoom: number, center_x: number, center_y: number, easing: string}>,
   *   zoomEnabled: boolean,
   *   cursorEnabled: boolean,
   *   cursorStyle: string,
   *   cursorSize: number,
   *   cursorColor: string,
   *   showCursorHighlight: boolean,
   *   showClickRipple: boolean,
   *   onClose: () => void,
   * }} */
  let {
    videoPath = '',
    trimStart = 0,
    trimEnd = 0,
    duration = 0,
    background = { type: 'transparent' },
    padding = 0,
    cornerRadius = 0,
    shadowEnabled = false,
    zoomKeyframes = [],
    zoomEnabled = true,
    keptSegments = [],
    cursorEnabled = true,
    cursorStyle = 'dot',
    cursorSize = 20,
    cursorColor = '#ff5050',
    showCursorHighlight = true,
    showClickRipple = true,
    onClose = () => {},
  } = $props();

  let exporting = $state(false);
  let progress = $state(0);
  let stage = $state('');
  let outputFormat = $state('mp4');
  let error = $state('');

  // Listen for export progress events (with cleanup)
  onMount(() => {
    let unlistenFn;
    listen('editor-export-progress', (event) => {
      const p = event.payload;
      progress = p.progress;
      stage = p.stage;
      if (p.error) error = p.error;
      if (p.stage === 'done') {
        exporting = false;
        if (p.output_path) {
          invoke('reveal_in_finder', { path: p.output_path });
        }
        setTimeout(onClose, 1000);
      }
    }).then(fn => { unlistenFn = fn; });

    return () => {
      if (unlistenFn) unlistenFn();
    };
  });

  async function startExport() {
    exporting = true;
    error = '';
    progress = 0;
    stage = 'starting';

    try {
      // If we have cuts, pass kept segments; otherwise use full trim range
      const hasSegments = keptSegments.length > 1 || (keptSegments.length === 1 &&
        (Math.abs(keptSegments[0].start - trimStart) > 0.05 || Math.abs(keptSegments[0].end - trimEnd) > 0.05));
      const segmentsToExport = hasSegments
        ? keptSegments.map(s => ({ start: s.start, end: s.end }))
        : [];

      const project = {
        source_path: videoPath,
        trim_start: trimStart,
        trim_end: trimEnd,
        segments: segmentsToExport,
        background: background,
        padding: padding,
        corner_radius: cornerRadius,
        shadow_radius: shadowEnabled ? 20 : 0,
        shadow_opacity: shadowEnabled ? 0.4 : 0,
        zoom_keyframes: zoomEnabled ? zoomKeyframes : [],
        cursor: cursorEnabled ? {
          enabled: true,
          style: cursorStyle,
          size: cursorSize,
          show_highlight: showCursorHighlight,
          show_click_ripple: showClickRipple,
          color: cursorColor,
        } : null,
        output_format: outputFormat,
      };

      await invoke('start_export', { project });
    } catch (e) {
      error = String(e);
      exporting = false;
    }
  }

  function formatTime(s) {
    const m = Math.floor(s / 60);
    const sec = Math.floor(s % 60);
    return `${m}:${sec.toString().padStart(2, '0')}`;
  }
</script>

<div class="export-panel">
  <div class="export-header">
    <h3>Export</h3>
    <button class="close-btn" onclick={onClose} title="Close">
      <svg width="12" height="12" viewBox="0 0 12 12"><path d="M1 1l10 10M11 1L1 11" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/></svg>
    </button>
  </div>

  <div class="export-body">
    <div class="field">
      <span class="field-label">Format</span>
      <div class="format-options">
        <button
          class="format-btn"
          class:active={outputFormat === 'mp4'}
          onclick={() => outputFormat = 'mp4'}
        >
          MP4
        </button>
        <button
          class="format-btn"
          class:active={outputFormat === 'gif'}
          onclick={() => outputFormat = 'gif'}
        >
          GIF
        </button>
      </div>
    </div>

    <div class="field">
      <span class="field-label">Range</span>
      <span class="field-value">
        {#if keptSegments.length > 1}
          {keptSegments.length} segments · {formatTime(keptSegments.reduce((s, seg) => s + seg.end - seg.start, 0))}
        {:else}
          {formatTime(trimStart)} → {formatTime(trimEnd)} ({formatTime(trimEnd - trimStart)})
        {/if}
      </span>
    </div>

    {#if background.type !== 'transparent' || (zoomEnabled && zoomKeyframes.length > 0) || cursorEnabled}
      <div class="field">
        <span class="field-label">Effects</span>
        <div class="effects-list">
          {#if background.type !== 'transparent'}
            <span class="effect-badge">
              {background.type === 'gradient' ? 'Gradient BG' : 'Solid BG'}
            </span>
          {/if}
          {#if cornerRadius > 0}
            <span class="effect-badge">Corners {cornerRadius}px</span>
          {/if}
          {#if padding > 0}
            <span class="effect-badge">Pad {padding}px</span>
          {/if}
          {#if shadowEnabled}
            <span class="effect-badge">Shadow</span>
          {/if}
          {#if zoomEnabled && zoomKeyframes.length > 0}
            <span class="effect-badge zoom-badge">Zoom ×{zoomKeyframes.length}</span>
          {/if}
          {#if cursorEnabled}
            <span class="effect-badge cursor-badge">Cursor</span>
          {/if}
        </div>
      </div>
    {/if}

    {#if exporting}
      <div class="progress-area">
        <div class="progress-bar">
          <div class="progress-fill" style="width: {progress * 100}%"></div>
        </div>
        <span class="progress-text">{stage}... {Math.round(progress * 100)}%</span>
      </div>
    {/if}

    {#if error}
      <div class="error-text">{error}</div>
    {/if}
  </div>

  <div class="export-footer">
    <button class="btn btn-ghost" onclick={onClose}>Cancel</button>
    <button class="btn btn-primary" onclick={startExport} disabled={exporting}>
      {exporting ? 'Exporting...' : 'Export'}
    </button>
  </div>
</div>

<style>
  .export-panel {
    position: fixed;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    width: 340px;
    background: #1a1a1e;
    border: 1px solid rgba(255,255,255,0.08);
    border-radius: 12px;
    box-shadow: 0 20px 60px rgba(0,0,0,0.6);
    z-index: 100;
    animation: fadeIn 0.2s ease;
  }

  @keyframes fadeIn {
    from { opacity: 0; transform: translate(-50%, -50%) scale(0.95); }
    to { opacity: 1; transform: translate(-50%, -50%) scale(1); }
  }

  .export-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 14px 16px 10px;
    border-bottom: 1px solid rgba(255,255,255,0.06);
  }

  .export-header h3 {
    margin: 0;
    font-size: 14px;
    font-weight: 600;
  }

  .close-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 24px;
    border: none;
    border-radius: 4px;
    background: transparent;
    color: #666;
    cursor: pointer;
  }

  .close-btn:hover { background: rgba(255,255,255,0.08); color: #fff; }

  .export-body {
    padding: 16px;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .field .field-label {
    font-size: 11px;
    font-weight: 500;
    color: #666;
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }

  .field-value {
    font-size: 13px;
    color: #aaa;
    font-variant-numeric: tabular-nums;
  }

  .format-options {
    display: flex;
    gap: 4px;
  }

  .format-btn {
    flex: 1;
    padding: 8px;
    border: 1px solid rgba(255,255,255,0.08);
    border-radius: 6px;
    background: transparent;
    color: #888;
    font-size: 12px;
    font-weight: 600;
    cursor: pointer;
    transition: all 0.15s;
  }

  .format-btn:hover { border-color: rgba(255,255,255,0.15); color: #fff; }
  .format-btn.active { background: rgba(108,92,231,0.15); border-color: #6c5ce7; color: #6c5ce7; }

  .effects-list {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }

  .effect-badge {
    font-size: 10px;
    font-weight: 600;
    padding: 2px 8px;
    border-radius: 4px;
    background: rgba(108,92,231,0.12);
    color: #a29bfe;
    letter-spacing: 0.2px;
  }

  .progress-area {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .progress-bar {
    height: 4px;
    background: rgba(255,255,255,0.06);
    border-radius: 2px;
    overflow: hidden;
  }

  .progress-fill {
    height: 100%;
    background: #6c5ce7;
    border-radius: 2px;
    transition: width 0.3s;
  }

  .progress-text {
    font-size: 11px;
    color: #888;
  }

  .error-text {
    font-size: 12px;
    color: #e74c3c;
  }

  .export-footer {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    padding: 10px 16px 14px;
    border-top: 1px solid rgba(255,255,255,0.06);
  }

  .btn {
    padding: 7px 16px;
    border: none;
    border-radius: 6px;
    font-size: 12px;
    font-weight: 600;
    cursor: pointer;
    transition: all 0.15s;
  }

  .btn-ghost { background: rgba(255,255,255,0.06); color: #aaa; }
  .btn-ghost:hover { background: rgba(255,255,255,0.12); color: #fff; }
  .btn-primary { background: #6c5ce7; color: #fff; }
  .btn-primary:hover { background: #7d6ff0; }
  .btn-primary:disabled { opacity: 0.5; cursor: not-allowed; }
</style>
