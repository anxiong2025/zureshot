<script>
  import { invoke } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import { getCurrentWindow } from '@tauri-apps/api/window';

  // ─── State ───
  let elapsed = $state(0);        // seconds (excluding paused time)
  let isPaused = $state(false);
  let isStopping = $state(false);

  let timerInterval = null;
  let lastTick = Date.now();

  // Format seconds → MM:SS
  let timeDisplay = $derived(() => {
    const mins = Math.floor(elapsed / 60);
    const secs = Math.floor(elapsed % 60);
    return `${String(mins).padStart(2, '0')}:${String(secs).padStart(2, '0')}`;
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

  // Allow dragging the bar
  let isDragging = $state(false);

  async function onBarMouseDown(e) {
    // Only drag on the bar background, not on buttons
    if (e.target.closest('button')) return;
    const win = getCurrentWindow();
    await win.startDragging();
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="bar" onmousedown={onBarMouseDown}>
  <!-- Red recording dot -->
  <div class="rec-indicator" class:paused={isPaused}>
    <div class="rec-dot"></div>
  </div>

  <!-- Timer -->
  <div class="timer" class:paused={isPaused}>
    {timeDisplay()}
  </div>

  <!-- Divider -->
  <div class="divider"></div>

  <!-- Pause/Resume button -->
  <button
    class="btn btn-pause"
    class:is-paused={isPaused}
    onclick={togglePause}
    title={isPaused ? 'Resume' : 'Pause'}
    disabled={isStopping}
  >
    {#if isPaused}
      <!-- Play icon -->
      <svg width="12" height="14" viewBox="0 0 12 14" fill="none">
        <path d="M1 1.5V12.5L11 7L1 1.5Z" fill="currentColor"/>
      </svg>
    {:else}
      <!-- Pause icon -->
      <svg width="10" height="12" viewBox="0 0 10 12" fill="none">
        <rect x="0" y="0" width="3.5" height="12" rx="1" fill="currentColor"/>
        <rect x="6.5" y="0" width="3.5" height="12" rx="1" fill="currentColor"/>
      </svg>
    {/if}
  </button>

  <!-- Stop button -->
  <button
    class="btn btn-stop"
    onclick={stopRecording}
    title="Stop Recording"
    disabled={isStopping}
  >
    <svg width="12" height="12" viewBox="0 0 12 12" fill="none">
      <rect width="12" height="12" rx="2" fill="currentColor"/>
    </svg>
  </button>
</div>

<style>
  .bar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 12px;
    background: rgba(28, 28, 30, 0.92);
    backdrop-filter: blur(20px);
    -webkit-backdrop-filter: blur(20px);
    border-radius: 12px;
    border: 1px solid rgba(255, 255, 255, 0.12);
    box-shadow:
      0 4px 24px rgba(0, 0, 0, 0.4),
      0 0 0 0.5px rgba(0, 0, 0, 0.3);
    cursor: grab;
    user-select: none;
    -webkit-user-select: none;
    width: fit-content;
    margin: 4px auto;
  }

  .bar:active {
    cursor: grabbing;
  }

  /* ─── Recording indicator dot ─── */
  .rec-indicator {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 16px;
    height: 16px;
  }

  .rec-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: #ff3b30;
    animation: pulse 1.5s ease-in-out infinite;
  }

  .rec-indicator.paused .rec-dot {
    background: #ff9500;
    animation: none;
  }

  @keyframes pulse {
    0%, 100% { opacity: 1; transform: scale(1); }
    50% { opacity: 0.5; transform: scale(0.85); }
  }

  /* ─── Timer ─── */
  .timer {
    font-family: ui-monospace, 'SF Mono', 'Menlo', monospace;
    font-size: 13px;
    font-weight: 600;
    color: #ffffff;
    letter-spacing: 0.5px;
    min-width: 48px;
    text-align: center;
  }

  .timer.paused {
    color: #ff9500;
  }

  /* ─── Divider ─── */
  .divider {
    width: 1px;
    height: 16px;
    background: rgba(255, 255, 255, 0.15);
    margin: 0 2px;
  }

  /* ─── Buttons ─── */
  .btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    border: none;
    border-radius: 7px;
    cursor: pointer;
    transition: all 0.12s ease;
    padding: 0;
    background: transparent;
    color: rgba(255, 255, 255, 0.8);
  }

  .btn:hover {
    background: rgba(255, 255, 255, 0.12);
    color: #ffffff;
  }

  .btn:active {
    transform: scale(0.92);
  }

  .btn:disabled {
    opacity: 0.4;
    cursor: default;
  }

  .btn-stop {
    color: #ff3b30;
  }

  .btn-stop:hover {
    background: rgba(255, 59, 48, 0.2);
    color: #ff453a;
  }

  .btn-pause.is-paused {
    color: #30d158;
  }

  .btn-pause.is-paused:hover {
    background: rgba(48, 209, 88, 0.2);
    color: #32d74b;
  }
</style>
