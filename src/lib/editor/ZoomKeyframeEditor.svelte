<script>
  import { invoke } from '@tauri-apps/api/core';

  /** @type {{
   *   keyframes: Array<{ time: number, zoom: number, center_x: number, center_y: number, easing: string }>,
   *   duration: number,
   *   currentTime: number,
   *   trimStart: number,
   *   trimEnd: number,
   *   enabled: boolean,
   *   videoWidth: number,
   *   videoHeight: number,
   *   onKeyframesChange: (kf: any[]) => void,
   *   onEnabledChange: (v: boolean) => void,
   *   onSeek: (t: number) => void,
   *   videoPath: string,
   * }}
   */
  let {
    keyframes = [],
    duration = 0,
    currentTime = 0,
    trimStart = 0,
    trimEnd = 0,
    enabled = true,
    videoWidth = 1920,
    videoHeight = 1080,
    onKeyframesChange = () => {},
    onEnabledChange = () => {},
    onSeek = () => {},
    videoPath = '',
  } = $props();

  let trackEl = $state(null);
  let draggingIdx = $state(-1);
  let selectedIdx = $state(-1);
  let autoZoomLoading = $state(false);

  // Current zoom level at playhead (interpolated)
  let currentZoomInfo = $derived.by(() => {
    return interpolateZoom(currentTime, keyframes);
  });

  // ─── Interpolation (FocuSee-style breathe model) ───
  // Must match VideoEditor.svelte's interpolation exactly.

  const TRANSITION_IN = 0.55;
  const TRANSITION_OUT = 0.60;
  const DEFAULT_HOLD = 0.6;
  const MERGE_GAP = 1.0;

  function springEaseIn(t) {
    const c = 1.2;
    const p = t - 1;
    return p * p * ((c + 1) * p + c) + 1;
  }

  function easeOutCubic(t) {
    return 1 - Math.pow(1 - t, 3);
  }

  function smoothStep(t) {
    return t * t * (3 - 2 * t);
  }

  function interpolateZoom(time, kfs) {
    if (!kfs || kfs.length === 0) {
      return { zoom: 1.0, centerX: 0.5, centerY: 0.5 };
    }

    const sorted = [...kfs].sort((a, b) => a.time - b.time);
    const segments = [];
    let seg = { kfs: [sorted[0]] };
    for (let i = 1; i < sorted.length; i++) {
      const prevKf = seg.kfs[seg.kfs.length - 1];
      const prevEnd = prevKf.time + (prevKf.hold || DEFAULT_HOLD);
      if (sorted[i].time - prevEnd < MERGE_GAP) {
        seg.kfs.push(sorted[i]);
      } else {
        segments.push(seg);
        seg = { kfs: [sorted[i]] };
      }
    }
    segments.push(seg);

    for (const s of segments) {
      const first = s.kfs[0];
      const last = s.kfs[s.kfs.length - 1];
      s.enterStart = first.time - TRANSITION_IN;
      s.peakStart = first.time;
      s.peakEnd = last.time + (last.hold || DEFAULT_HOLD);
      s.exitEnd = s.peakEnd + TRANSITION_OUT;
    }

    const REST = { zoom: 1.0, centerX: 0.5, centerY: 0.5 };

    for (const s of segments) {
      if (time < s.enterStart || time > s.exitEnd) continue;

      const firstKf = s.kfs[0];
      const lastKf = s.kfs[s.kfs.length - 1];

      if (time < s.peakStart) {
        const t = Math.max(0, Math.min(1, (time - s.enterStart) / TRANSITION_IN));
        const e = springEaseIn(t);
        return {
          zoom: 1.0 + (firstKf.zoom - 1.0) * e,
          centerX: 0.5 + (firstKf.center_x - 0.5) * e,
          centerY: 0.5 + (firstKf.center_y - 0.5) * e,
        };
      }

      if (time > s.peakEnd) {
        const t = Math.max(0, Math.min(1, (time - s.peakEnd) / TRANSITION_OUT));
        const e = 1 - easeOutCubic(t);
        return {
          zoom: 1.0 + (lastKf.zoom - 1.0) * e,
          centerX: 0.5 + (lastKf.center_x - 0.5) * e,
          centerY: 0.5 + (lastKf.center_y - 0.5) * e,
        };
      }

      if (s.kfs.length === 1) {
        return { zoom: firstKf.zoom, centerX: firstKf.center_x, centerY: firstKf.center_y };
      }

      for (let i = 0; i < s.kfs.length; i++) {
        const kf = s.kfs[i];
        const kfEnd = kf.time + (kf.hold || DEFAULT_HOLD);
        if (time >= kf.time && time <= kfEnd) {
          return { zoom: kf.zoom, centerX: kf.center_x, centerY: kf.center_y };
        }
        if (i < s.kfs.length - 1) {
          const next = s.kfs[i + 1];
          if (time > kfEnd && time < next.time) {
            const span = next.time - kfEnd;
            const t = span > 0 ? (time - kfEnd) / span : 0;
            const e = smoothStep(t);
            return {
              zoom: kf.zoom + (next.zoom - kf.zoom) * e,
              centerX: kf.center_x + (next.center_x - kf.center_x) * e,
              centerY: kf.center_y + (next.center_y - kf.center_y) * e,
            };
          }
        }
      }

      return { zoom: lastKf.zoom, centerX: lastKf.center_x, centerY: lastKf.center_y };
    }

    return REST;
  }

  // ─── Auto-Zoom from Mouse Track ───

  async function autoZoom() {
    autoZoomLoading = true;
    try {
      const suggested = await invoke('suggest_zoom_keyframes', { videoPath });

      if (suggested.length === 0) {
        // No suggestions — maybe show a hint
        console.log('[zoom] No zoom suggestions from mouse track');
        autoZoomLoading = false;
        return;
      }

      // Coordinates are already normalized 0-1 from Rust
      const normalized = suggested.map(kf => ({
        ...kf,
        center_x: Math.max(0, Math.min(1, kf.center_x)),
        center_y: Math.max(0, Math.min(1, kf.center_y)),
        zoom: Math.min(3.0, Math.max(1.2, kf.zoom)),
      }));

      onKeyframesChange(normalized);
    } catch (e) {
      console.warn('[zoom] Auto-zoom failed (no mouse track?):', e);
    }
    autoZoomLoading = false;
  }

  // ─── Keyframe Manipulation ───

  function addKeyframe() {
    const newKf = {
      time: currentTime,
      zoom: 2.0,
      center_x: 0.5,
      center_y: 0.5,
      easing: 'spring',
      hold: 0.5,
    };
    const updated = [...keyframes, newKf].sort((a, b) => a.time - b.time);
    onKeyframesChange(updated);
    selectedIdx = updated.findIndex(k => k.time === currentTime);
  }

  function removeKeyframe(idx) {
    const updated = keyframes.filter((_, i) => i !== idx);
    onKeyframesChange(updated);
    if (selectedIdx === idx) selectedIdx = -1;
    if (selectedIdx > idx) selectedIdx--;
  }

  function clearAllKeyframes() {
    onKeyframesChange([]);
    selectedIdx = -1;
  }

  function updateKeyframe(idx, field, value) {
    const updated = [...keyframes];
    updated[idx] = { ...updated[idx], [field]: value };
    onKeyframesChange(updated);
  }

  // ─── Track Interaction ───

  function timeFromX(clientX) {
    if (!trackEl || duration <= 0) return 0;
    const rect = trackEl.getBoundingClientRect();
    return Math.max(0, Math.min(duration, ((clientX - rect.left) / rect.width) * duration));
  }

  function handleTrackClick(e) {
    if (draggingIdx >= 0) return;
    // Check if clicking near a keyframe
    const clickTime = timeFromX(e.clientX);
    const threshold = duration * 0.015; // 1.5% of timeline
    const nearIdx = keyframes.findIndex(kf => Math.abs(kf.time - clickTime) < threshold);
    if (nearIdx >= 0) {
      selectedIdx = nearIdx;
      onSeek(keyframes[nearIdx].time);
    } else {
      selectedIdx = -1;
    }
  }

  function startDragKeyframe(idx, e) {
    e.stopPropagation();
    e.preventDefault();
    draggingIdx = idx;
    selectedIdx = idx;

    const handleMove = (e) => {
      const time = timeFromX(e.clientX);
      updateKeyframe(draggingIdx, 'time', time);
    };

    const handleUp = () => {
      draggingIdx = -1;
      window.removeEventListener('mousemove', handleMove);
      window.removeEventListener('mouseup', handleUp);
    };

    window.addEventListener('mousemove', handleMove);
    window.addEventListener('mouseup', handleUp);
  }

  function formatT(s) {
    const m = Math.floor(s / 60);
    const sec = Math.floor(s % 60);
    return `${m}:${sec.toString().padStart(2, '0')}`;
  }

  // Generate SVG path for the zoom curve visualization
  function generateZoomCurvePoints() {
    const steps = 200;
    let pathData = `M 0 40`; // start at bottom-left

    for (let i = 0; i <= steps; i++) {
      const t = (i / steps) * duration;
      const z = interpolateZoom(t, keyframes);
      const x = (i / steps) * 1000;
      // Map zoom 1.0-4.0 to height 40-0 (inverted Y for SVG)
      const y = 40 - Math.min(40, ((z.zoom - 1.0) / 3.0) * 36);
      pathData += ` L ${x.toFixed(1)} ${y.toFixed(1)}`;
    }

    pathData += ` L 1000 40 Z`; // close path to bottom-right
    return pathData;
  }
</script>

<div class="zoom-editor">
  <!-- Header with auto-zoom + add buttons -->
  <div class="zoom-header">
    <div class="zoom-title">
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="11" cy="11" r="8"/><line x1="21" y1="21" x2="16.65" y2="16.65"/><line x1="11" y1="8" x2="11" y2="14"/><line x1="8" y1="11" x2="14" y2="11"/></svg>
      <span>Auto Zoom</span>
      {#if keyframes.length > 0}
        <span class="kf-count">{keyframes.length}</span>
      {/if}
    </div>
    <div class="zoom-actions">
      <button class="action-btn" onclick={autoZoom} disabled={autoZoomLoading} title="Auto-detect zoom from mouse movement">
        {#if autoZoomLoading}
          <div class="mini-spinner"></div>
        {:else}
          <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M12 2v4m0 12v4M2 12h4m12 0h4M4.93 4.93l2.83 2.83m8.48 8.48l2.83 2.83M4.93 19.07l2.83-2.83m8.48-8.48l2.83-2.83"/></svg>
        {/if}
        Auto
      </button>
      <button class="action-btn" onclick={addKeyframe} title="Add keyframe at playhead">
        <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><line x1="12" y1="5" x2="12" y2="19"/><line x1="5" y1="12" x2="19" y2="12"/></svg>
      </button>
      {#if keyframes.length > 0}
        <button class="action-btn danger" onclick={clearAllKeyframes} title="Clear all keyframes">
          <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 01-2 2H7a2 2 0 01-2-2V6m3 0V4a2 2 0 012-2h4a2 2 0 012 2v2"/></svg>
        </button>
      {/if}
    </div>
  </div>

  <!-- Zoom Track (keyframe diamonds on timeline) -->
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="zoom-track" bind:this={trackEl} onmousedown={handleTrackClick}>
    <!-- Zoom level visualization (area graph) -->
    <svg class="zoom-vis" viewBox="0 0 1000 40" preserveAspectRatio="none">
      <defs>
        <linearGradient id="zoomGrad" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0%" stop-color="#00b894" stop-opacity="0.4"/>
          <stop offset="100%" stop-color="#00b894" stop-opacity="0.05"/>
        </linearGradient>
      </defs>
      {#if keyframes.length > 0}
        {@const points = generateZoomCurvePoints()}
        <path d={points} fill="url(#zoomGrad)" stroke="#00b894" stroke-width="1.5" stroke-opacity="0.6"/>
      {/if}
    </svg>

    <!-- Keyframe diamonds -->
    {#each keyframes as kf, i}
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div
        class="kf-marker"
        class:selected={selectedIdx === i}
        style="left: {(kf.time / duration) * 100}%"
        onmousedown={(e) => startDragKeyframe(i, e)}
        title="{formatT(kf.time)} — {kf.zoom.toFixed(1)}x"
      >
        <div class="kf-diamond"></div>
        <span class="kf-label">{kf.zoom.toFixed(1)}x</span>
      </div>
    {/each}

    <!-- Current zoom indicator -->
    {#if currentZoomInfo.zoom > 1.01}
      <div class="current-zoom-badge" style="left: {(currentTime / duration) * 100}%">
        {currentZoomInfo.zoom.toFixed(1)}x
      </div>
    {/if}
  </div>

  <!-- Selected Keyframe Editor -->
  {#if selectedIdx >= 0 && keyframes[selectedIdx]}
    {@const kf = keyframes[selectedIdx]}
    <div class="kf-detail">
      <div class="kf-detail-row">
        <span class="kf-detail-label">Time</span>
        <span class="kf-detail-val">{formatT(kf.time)}</span>
      </div>
      <div class="kf-detail-row">
        <span class="kf-detail-label">Zoom</span>
        <input
          type="range"
          min="1.1"
          max="4.0"
          step="0.1"
          value={kf.zoom}
          oninput={(e) => updateKeyframe(selectedIdx, 'zoom', parseFloat(e.target.value))}
        />
        <span class="kf-detail-val">{kf.zoom.toFixed(1)}x</span>
      </div>
      <div class="kf-detail-row">
        <span class="kf-detail-label">Focus X</span>
        <input
          type="range"
          min="0"
          max="1"
          step="0.01"
          value={kf.center_x}
          oninput={(e) => updateKeyframe(selectedIdx, 'center_x', parseFloat(e.target.value))}
        />
        <span class="kf-detail-val">{Math.round(kf.center_x * 100)}%</span>
      </div>
      <div class="kf-detail-row">
        <span class="kf-detail-label">Focus Y</span>
        <input
          type="range"
          min="0"
          max="1"
          step="0.01"
          value={kf.center_y}
          oninput={(e) => updateKeyframe(selectedIdx, 'center_y', parseFloat(e.target.value))}
        />
        <span class="kf-detail-val">{Math.round(kf.center_y * 100)}%</span>
      </div>
      <div class="kf-detail-row">
        <span class="kf-detail-label">Hold</span>
        <input
          type="range"
          min="0.1"
          max="2.0"
          step="0.1"
          value={kf.hold || 0.5}
          oninput={(e) => updateKeyframe(selectedIdx, 'hold', parseFloat(e.target.value))}
        />
        <span class="kf-detail-val">{(kf.hold || 0.5).toFixed(1)}s</span>
      </div>
      <button class="remove-kf-btn" onclick={() => removeKeyframe(selectedIdx)}>Remove Keyframe</button>
    </div>
  {/if}
</div>

<style>
  .zoom-editor {
    border-top: 1px solid rgba(255,255,255,0.04);
    padding: 8px 16px 12px;
  }

  /* ─── Header ─── */
  .zoom-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 6px;
  }

  .zoom-title {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 11px;
    font-weight: 600;
    color: #00b894;
  }

  .kf-count {
    font-size: 9px;
    font-weight: 700;
    padding: 1px 5px;
    border-radius: 8px;
    background: rgba(0, 184, 148, 0.15);
    color: #00b894;
  }

  .zoom-actions {
    display: flex;
    gap: 4px;
  }

  .action-btn {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 3px 8px;
    border: none;
    border-radius: 4px;
    background: rgba(255,255,255,0.06);
    color: #888;
    font-size: 10px;
    font-weight: 600;
    cursor: pointer;
    transition: all 0.15s;
  }

  .action-btn:hover {
    background: rgba(255,255,255,0.12);
    color: #fff;
  }

  .action-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .action-btn.danger:hover {
    background: rgba(231, 76, 60, 0.15);
    color: #e74c3c;
  }

  .mini-spinner {
    width: 10px;
    height: 10px;
    border: 1.5px solid rgba(255,255,255,0.15);
    border-top-color: #00b894;
    border-radius: 50%;
    animation: spin 0.6s linear infinite;
  }

  @keyframes spin { to { transform: rotate(360deg); } }

  /* ─── Zoom Track ─── */
  .zoom-track {
    position: relative;
    height: 40px;
    background: rgba(255,255,255,0.03);
    border-radius: 4px;
    cursor: pointer;
    overflow: visible;
  }

  .zoom-vis {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    pointer-events: none;
  }

  /* ─── Keyframe Markers ─── */
  .kf-marker {
    position: absolute;
    top: 50%;
    transform: translate(-50%, -50%);
    cursor: grab;
    z-index: 5;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 2px;
  }

  .kf-marker:active { cursor: grabbing; }

  .kf-diamond {
    width: 10px;
    height: 10px;
    background: #00b894;
    transform: rotate(45deg);
    border-radius: 2px;
    box-shadow: 0 0 6px rgba(0, 184, 148, 0.4);
    transition: all 0.15s;
  }

  .kf-marker:hover .kf-diamond {
    background: #55efc4;
    box-shadow: 0 0 10px rgba(0, 184, 148, 0.6);
    transform: rotate(45deg) scale(1.2);
  }

  .kf-marker.selected .kf-diamond {
    background: #fff;
    box-shadow: 0 0 12px rgba(255, 255, 255, 0.5);
  }

  .kf-label {
    font-size: 8px;
    font-weight: 700;
    color: #00b894;
    white-space: nowrap;
    pointer-events: none;
  }

  .current-zoom-badge {
    position: absolute;
    top: -2px;
    transform: translateX(-50%);
    font-size: 8px;
    font-weight: 700;
    color: #00b894;
    background: rgba(0, 184, 148, 0.15);
    padding: 1px 4px;
    border-radius: 3px;
    pointer-events: none;
    white-space: nowrap;
  }

  /* ─── Keyframe Detail Editor ─── */
  .kf-detail {
    margin-top: 8px;
    padding: 8px 10px;
    background: rgba(0, 184, 148, 0.04);
    border: 1px solid rgba(0, 184, 148, 0.1);
    border-radius: 6px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .kf-detail-row {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .kf-detail-label {
    font-size: 10px;
    font-weight: 600;
    color: #666;
    width: 48px;
    flex-shrink: 0;
  }

  .kf-detail-val {
    font-size: 10px;
    font-weight: 700;
    color: #aaa;
    font-variant-numeric: tabular-nums;
    width: 36px;
    text-align: right;
    flex-shrink: 0;
  }

  .kf-detail-row input[type="range"] {
    flex: 1;
    height: 3px;
    -webkit-appearance: none;
    appearance: none;
    background: rgba(255,255,255,0.08);
    border-radius: 2px;
    outline: none;
  }

  .kf-detail-row input[type="range"]::-webkit-slider-thumb {
    -webkit-appearance: none;
    appearance: none;
    width: 12px;
    height: 12px;
    border-radius: 50%;
    background: #00b894;
    cursor: pointer;
    box-shadow: 0 1px 4px rgba(0,0,0,0.4);
  }

  .remove-kf-btn {
    padding: 4px 10px;
    border: 1px solid rgba(231, 76, 60, 0.2);
    border-radius: 4px;
    background: transparent;
    color: #e74c3c;
    font-size: 10px;
    font-weight: 600;
    cursor: pointer;
    transition: all 0.15s;
    align-self: flex-start;
  }

  .remove-kf-btn:hover {
    background: rgba(231, 76, 60, 0.1);
  }
</style>
