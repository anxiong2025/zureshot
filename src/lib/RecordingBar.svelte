<script>
  import { invoke } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import { getCurrentWindow } from '@tauri-apps/api/window';

  // ─── State ───
  let elapsed = $state(0);        // seconds (excluding paused time)
  let isPaused = $state(false);
  let isStopping = $state(false);
  let recordingFormat = $state('video');  // 'video' or 'gif'
  let maxDuration = $state(0);           // 0 = unlimited
  let cameraOn = $state(false);          // camera bubble state

  let isGif = $derived(recordingFormat === 'gif');
  let remaining = $derived(maxDuration > 0 ? Math.max(0, maxDuration - elapsed) : 0);
  let progress = $derived(maxDuration > 0 ? Math.min(1, elapsed / maxDuration) : 0);
  let isNearLimit = $derived(maxDuration > 0 && remaining <= 5);

  let timerInterval = null;
  let lastTick = Date.now();

  // Format seconds → MM:SS
  let timeDisplay = $derived(() => {
    if (isGif && maxDuration > 0) {
      // Show remaining time for GIF
      const secs = Math.ceil(remaining);
      const mins = Math.floor(secs / 60);
      const s = secs % 60;
      return `${String(mins).padStart(2, '0')}:${String(s).padStart(2, '0')}`;
    }
    const mins = Math.floor(elapsed / 60);
    const secs = Math.floor(elapsed % 60);
    return `${String(mins).padStart(2, '0')}:${String(secs).padStart(2, '0')}`;
  });

  // Listen for recording-started to get format info
  listen('recording-started', (event) => {
    const payload = event.payload;
    if (payload.format) recordingFormat = payload.format;
    if (payload.max_duration) maxDuration = payload.max_duration;
  });

  // Start the timer immediately
  startTimer();

  function startTimer() {
    lastTick = Date.now();
    timerInterval = setInterval(() => {
      if (!isPaused) {
        const now = Date.now();
        elapsed += (now - lastTick) / 1000;
        lastTick = now;

        // Auto-stop GIF at max duration
        if (isGif && maxDuration > 0 && elapsed >= maxDuration) {
          stopRecording();
        }
      } else {
        lastTick = Date.now();
      }
    }, 100);
  }

  async function togglePause() {
    if (isPaused) {
      // Resume
      try {
        await invoke('resume_recording');
        isPaused = false;
        lastTick = Date.now();
      } catch (e) {
        console.error('Resume failed:', e);
      }
    } else {
      // Pause
      try {
        await invoke('pause_recording');
        isPaused = true;
      } catch (e) {
        console.error('Pause failed:', e);
      }
    }
  }

  async function toggleCamera() {
    try {
      const isOpen = await invoke('toggle_camera_overlay');
      cameraOn = isOpen;
    } catch (e) {
      console.error('Toggle camera failed:', e);
    }
  }

  async function stopRecording() {
    if (isStopping) return;
    isStopping = true;
    clearInterval(timerInterval);

    try {
      await invoke('stop_recording');
    } catch (e) {
      console.error('Stop failed:', e);
    }
    // The window will be closed by the Rust side via recording-stopped event
  }

  // Listen for recording-stopped to close ourselves
  listen('recording-stopped', () => {
    clearInterval(timerInterval);
    const win = getCurrentWindow();
    win.destroy();
  });
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="bar" class:gif-mode={isGif} class:near-limit={isNearLimit} data-tauri-drag-region>
  <!-- Left: status section -->
  <div class="status-section" data-tauri-drag-region>
    {#if isGif}
      <div class="format-pill gif">GIF</div>
    {:else}
      <div class="rec-dot-wrap" class:paused={isPaused}>
        <div class="rec-dot"></div>
        <div class="rec-ring"></div>
      </div>
    {/if}

    <div class="timer" class:paused={isPaused} class:countdown={isGif} class:near-limit={isNearLimit}>
      {timeDisplay()}
    </div>

    <!-- GIF progress -->
    {#if isGif && maxDuration > 0}
      <div class="progress-track">
        <div class="progress-fill" class:near-limit={isNearLimit} style="width:{progress * 100}%"></div>
      </div>
    {/if}
  </div>

  <!-- Right: controls -->
  <div class="controls">
    <!-- Camera toggle -->
    <button
      class="ctl-btn"
      class:active={cameraOn}
      onclick={toggleCamera}
      title={cameraOn ? 'Hide Camera' : 'Show Camera'}
      disabled={isStopping}
    >
      <svg width="15" height="15" viewBox="0 0 16 16" fill="none">
        <rect x="1.5" y="3.5" width="9" height="9" rx="2" stroke="currentColor" stroke-width="1.4"/>
        <path d="M10.5 6L14 4V12L10.5 10" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"/>
      </svg>
    </button>

    <!-- Pause/Resume (video only) -->
    {#if !isGif}
      <button
        class="ctl-btn"
        class:active={isPaused}
        onclick={togglePause}
        title={isPaused ? 'Resume' : 'Pause'}
        disabled={isStopping}
      >
        {#if isPaused}
          <svg width="13" height="13" viewBox="0 0 14 14" fill="none">
            <path d="M3 1.5L12 7L3 12.5V1.5Z" fill="currentColor"/>
          </svg>
        {:else}
          <svg width="13" height="13" viewBox="0 0 12 14" fill="none">
            <rect x="0" y="0" width="4" height="14" rx="1.2" fill="currentColor"/>
            <rect x="8" y="0" width="4" height="14" rx="1.2" fill="currentColor"/>
          </svg>
        {/if}
      </button>
    {/if}

    <!-- Stop -->
    <button
      class="stop-btn"
      onclick={stopRecording}
      title="Stop Recording"
      disabled={isStopping}
    >
      <svg width="10" height="10" viewBox="0 0 10 10" fill="none">
        <rect width="10" height="10" rx="2" fill="currentColor"/>
      </svg>
    </button>
  </div>
</div>

<style>
  .bar {
    display: flex;
    align-items: center;
    gap: 0;
    padding: 0;
    background: rgba(20, 20, 22, 0.88);
    backdrop-filter: blur(40px) saturate(1.8);
    -webkit-backdrop-filter: blur(40px) saturate(1.8);
    border-radius: 14px;
    border: 1px solid rgba(255, 255, 255, 0.08);
    box-shadow:
      0 8px 32px rgba(0, 0, 0, 0.5),
      0 2px 8px rgba(0, 0, 0, 0.3),
      inset 0 1px 0 rgba(255, 255, 255, 0.04);
    cursor: grab;
    -webkit-app-region: drag;
    user-select: none;
    -webkit-user-select: none;
    width: fit-content;
    margin: 4px auto;
    height: 40px;
  }
  .bar:active { cursor: grabbing; }

  /* ─── Left: status area ─── */
  .status-section {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 14px;
    height: 100%;
  }

  /* ─── Recording dot with ring effect ─── */
  .rec-dot-wrap {
    position: relative;
    width: 10px;
    height: 10px;
    flex-shrink: 0;
  }
  .rec-dot {
    width: 10px;
    height: 10px;
    border-radius: 50%;
    background: #ff3b30;
    position: relative;
    z-index: 1;
  }
  .rec-ring {
    position: absolute;
    inset: -3px;
    border-radius: 50%;
    border: 1.5px solid rgba(255, 59, 48, 0.4);
    animation: ring-pulse 2s ease-in-out infinite;
  }
  .rec-dot-wrap:not(.paused) .rec-dot {
    animation: dot-glow 2s ease-in-out infinite;
  }
  .rec-dot-wrap.paused .rec-dot {
    background: #ff9f0a;
    animation: none;
  }
  .rec-dot-wrap.paused .rec-ring {
    border-color: rgba(255, 159, 10, 0.3);
    animation: none;
  }

  @keyframes dot-glow {
    0%, 100% { opacity: 1; box-shadow: 0 0 6px rgba(255, 59, 48, 0.4); }
    50% { opacity: 0.6; box-shadow: 0 0 2px rgba(255, 59, 48, 0.2); }
  }
  @keyframes ring-pulse {
    0%, 100% { transform: scale(1); opacity: 0.6; }
    50% { transform: scale(1.25); opacity: 0; }
  }

  /* ─── Timer ─── */
  .timer {
    font-family: ui-monospace, 'SF Mono', 'Menlo', monospace;
    font-size: 13px;
    font-weight: 500;
    color: rgba(255, 255, 255, 0.95);
    letter-spacing: 0.8px;
    min-width: 48px;
    text-align: center;
    font-variant-numeric: tabular-nums;
  }
  .timer.paused { color: #ff9f0a; }
  .timer.countdown { color: rgba(255, 255, 255, 0.85); }
  .timer.near-limit {
    color: #ff9f0a;
    animation: timer-flash 0.6s ease-in-out infinite;
  }
  @keyframes timer-flash {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.4; }
  }

  /* ─── GIF badge ─── */
  .format-pill {
    font-size: 9px;
    font-weight: 800;
    letter-spacing: 0.6px;
    padding: 2px 6px;
    border-radius: 5px;
    line-height: 1.3;
    flex-shrink: 0;
  }
  .format-pill.gif {
    background: linear-gradient(135deg, #a855f7, #7c3aed);
    color: #fff;
    box-shadow: 0 1px 4px rgba(139, 92, 246, 0.4);
  }

  /* ─── Progress track ─── */
  .progress-track {
    width: 36px;
    height: 3px;
    background: rgba(255, 255, 255, 0.08);
    border-radius: 2px;
    overflow: hidden;
    flex-shrink: 0;
  }
  .progress-fill {
    height: 100%;
    background: linear-gradient(90deg, #a855f7, #7c3aed);
    border-radius: 2px;
    transition: width 0.15s linear;
  }
  .progress-fill.near-limit { background: #ff9f0a; }

  /* ─── Right: controls ─── */
  .controls {
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 0 6px;
    height: 100%;
    border-left: 1px solid rgba(255, 255, 255, 0.06);
    -webkit-app-region: no-drag;
  }

  .ctl-btn {
    width: 30px;
    height: 30px;
    border: none;
    border-radius: 8px;
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 0;
    background: transparent;
    color: rgba(255, 255, 255, 0.55);
    transition: all 0.15s ease;
    -webkit-app-region: no-drag;
  }
  .ctl-btn:hover {
    background: rgba(255, 255, 255, 0.1);
    color: rgba(255, 255, 255, 0.9);
  }
  .ctl-btn:active { transform: scale(0.9); }
  .ctl-btn:disabled { opacity: 0.3; cursor: default; pointer-events: none; }

  .ctl-btn.active {
    color: #0a84ff;
    background: rgba(10, 132, 255, 0.12);
  }
  .ctl-btn.active:hover {
    background: rgba(10, 132, 255, 0.22);
  }

  /* ─── Stop button — the standout red pill ─── */
  .stop-btn {
    width: 30px;
    height: 30px;
    border: none;
    border-radius: 8px;
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 0;
    background: rgba(255, 59, 48, 0.15);
    color: #ff453a;
    transition: all 0.15s ease;
    -webkit-app-region: no-drag;
  }
  .stop-btn:hover {
    background: rgba(255, 59, 48, 0.28);
    color: #ff6961;
    transform: scale(1.04);
  }
  .stop-btn:active { transform: scale(0.9); }
  .stop-btn:disabled { opacity: 0.3; cursor: default; pointer-events: none; }

  /* ─── Near-limit state ─── */
  .bar.near-limit {
    border-color: rgba(255, 159, 10, 0.2);
    box-shadow:
      0 8px 32px rgba(0, 0, 0, 0.5),
      0 0 16px rgba(255, 159, 10, 0.08);
  }
</style>
