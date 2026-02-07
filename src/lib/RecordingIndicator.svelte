<script>
  /** @type {{ duration: number, onStop: () => void }} */
  let { duration, onStop } = $props();

  function formatDuration(secs) {
    const mins = Math.floor(secs / 60);
    const s = Math.floor(secs % 60);
    return `${mins.toString().padStart(2, '0')}:${s.toString().padStart(2, '0')}`;
  }
</script>

<div class="indicator">
  <div class="recording-dot"></div>
  <span class="time">{formatDuration(duration)}</span>
  <button class="stop-btn" onclick={onStop}>
    <svg width="12" height="12" viewBox="0 0 12 12" fill="currentColor">
      <rect width="12" height="12" rx="2" />
    </svg>
  </button>
</div>

<style>
  .indicator {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    background: rgba(0, 0, 0, 0.85);
    backdrop-filter: blur(20px);
    border-radius: 20px;
    border: 1px solid rgba(255, 255, 255, 0.1);
    box-shadow: 0 4px 20px rgba(0, 0, 0, 0.3);
    position: fixed;
    top: 40px;
    left: 50%;
    transform: translateX(-50%);
    z-index: 9999;
    -webkit-app-region: drag;
  }

  .recording-dot {
    width: 10px;
    height: 10px;
    background: #ff3b30;
    border-radius: 50%;
    animation: pulse 1s ease-in-out infinite;
  }

  @keyframes pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.4; }
  }

  .time {
    font-family: ui-monospace, 'SF Mono', monospace;
    font-size: 14px;
    font-weight: 500;
    color: white;
    min-width: 50px;
  }

  .stop-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 24px;
    background: #ff3b30;
    border: none;
    border-radius: 6px;
    color: white;
    cursor: pointer;
    transition: background 0.2s;
    -webkit-app-region: no-drag;
  }

  .stop-btn:hover {
    background: #ff5c4d;
  }

  .stop-btn:active {
    background: #d63029;
  }
</style>
