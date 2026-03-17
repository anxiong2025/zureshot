<script>
  import { invoke } from '@tauri-apps/api/core';

  let frameCount = $state(1);
  let totalHeight = $state(0);
  let isFinishing = $state(false);
  let tickInterval = null;
  let newContentFlash = $state(false);

  startTicking();

  function startTicking() {
    tickInterval = setInterval(async () => {
      if (isFinishing) return;
      try {
        const status = await invoke('scroll_capture_tick');
        if (status.new_content && status.frame_count > frameCount) {
          newContentFlash = true;
          setTimeout(() => { newContentFlash = false; }, 400);
        }
        frameCount = status.frame_count;
        totalHeight = status.total_height;
      } catch (e) {
        console.error('scroll_capture_tick error:', e);
      }
    }, 300);
  }

  async function finish() {
    if (isFinishing) return;
    isFinishing = true;
    clearInterval(tickInterval);
    try {
      await invoke('finish_scroll_capture');
    } catch (e) {
      console.error('finish_scroll_capture error:', e);
    }
  }

  async function cancel() {
    clearInterval(tickInterval);
    try {
      await invoke('cancel_scroll_capture');
    } catch (e) {
      console.error('cancel_scroll_capture error:', e);
    }
  }
</script>

<svelte:window
  onkeydown={(e) => {
    if (e.key === 'Enter') finish();
    if (e.key === 'Escape') cancel();
  }}
/>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="bar" class:flash={newContentFlash} data-tauri-drag-region>
  <!-- Animated scroll arrow -->
  <div class="arrow">
    <svg width="14" height="14" viewBox="0 0 14 14" fill="none">
      <path d="M7 2V12M7 12L3 8M7 12L11 8" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"/>
    </svg>
  </div>

  <div class="info">
    <span class="label">
      {#if isFinishing}
        saving...
      {:else}
        scroll down
      {/if}
    </span>
    {#if frameCount > 1}
      <span class="counter">+{frameCount - 1}</span>
    {/if}
  </div>

  <button class="btn-done" onclick={finish} disabled={isFinishing}>
    Done
  </button>

  <button class="btn-x" onclick={cancel} disabled={isFinishing} title="Esc">
    <svg width="8" height="8" viewBox="0 0 10 10" fill="none">
      <path d="M1 1L9 9M9 1L1 9" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"/>
    </svg>
  </button>
</div>

<style>
  .bar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 7px 10px 7px 12px;
    background: rgba(0, 0, 0, 0.78);
    backdrop-filter: blur(16px);
    -webkit-backdrop-filter: blur(16px);
    border-radius: 20px;
    border: 1.5px solid rgba(48, 209, 88, 0.35);
    box-shadow: 0 4px 20px rgba(0, 0, 0, 0.45);
    cursor: grab;
    -webkit-app-region: drag;
    user-select: none;
    -webkit-user-select: none;
    width: fit-content;
    margin: 4px auto;
    transition: border-color 0.3s ease;
  }

  .bar:active { cursor: grabbing; }

  .bar.flash {
    border-color: rgba(48, 209, 88, 0.8);
  }

  .arrow {
    color: #30d158;
    display: flex;
    align-items: center;
    animation: bounce 1.2s ease-in-out infinite;
    flex-shrink: 0;
  }

  @keyframes bounce {
    0%, 100% { transform: translateY(-2px); opacity: 0.6; }
    50% { transform: translateY(2px); opacity: 1; }
  }

  .info {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .label {
    font-size: 12px;
    font-weight: 500;
    color: rgba(255, 255, 255, 0.75);
    white-space: nowrap;
  }

  .counter {
    font-family: ui-monospace, 'SF Mono', 'Menlo', monospace;
    font-size: 11px;
    font-weight: 700;
    color: #30d158;
    background: rgba(48, 209, 88, 0.15);
    padding: 1px 6px;
    border-radius: 8px;
  }

  .btn-done {
    background: #30d158;
    color: #fff;
    font-size: 12px;
    font-weight: 600;
    padding: 4px 14px;
    border-radius: 14px;
    border: none;
    cursor: pointer;
    transition: all 0.12s ease;
    -webkit-app-region: no-drag;
    white-space: nowrap;
  }

  .btn-done:hover { background: #34d65c; }
  .btn-done:active { transform: scale(0.93); }
  .btn-done:disabled { opacity: 0.4; cursor: default; }

  .btn-x {
    width: 22px;
    height: 22px;
    display: flex;
    align-items: center;
    justify-content: center;
    border: none;
    border-radius: 50%;
    background: transparent;
    color: rgba(255, 255, 255, 0.35);
    cursor: pointer;
    transition: all 0.12s ease;
    padding: 0;
    -webkit-app-region: no-drag;
    flex-shrink: 0;
  }

  .btn-x:hover { background: rgba(255, 59, 48, 0.25); color: #ff453a; }
  .btn-x:disabled { opacity: 0.3; cursor: default; }
</style>
