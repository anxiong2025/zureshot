<script>
  /** @type {{ onAction: (action: string, data?: any) => void, activePanel: string }} */
  let { onAction = () => {}, activePanel = '' } = $props();
</script>

<div class="toolbar">
  <div class="tool-group">
    <button class="tool-btn" onclick={() => onAction('trim')} title="Quick Trim">
      <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
        <circle cx="6" cy="6" r="3"/><circle cx="6" cy="18" r="3"/><line x1="20" y1="4" x2="8.12" y2="15.88"/><line x1="14.47" y1="14.48" x2="20" y2="20"/><line x1="8.12" y1="8.12" x2="12" y2="12"/>
      </svg>
      <span>Trim</span>
    </button>
    <button class="tool-btn" class:active={activePanel === 'style'} onclick={() => onAction('toggle-style')} title="Background & Style">
      <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
        <path d="M12 2L2 7l10 5 10-5-10-5z"/><path d="M2 17l10 5 10-5"/><path d="M2 12l10 5 10-5"/>
      </svg>
      <span>Style</span>
    </button>
  </div>
</div>

<style>
  .toolbar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 16px;
    background: #141416;
    border-bottom: 1px solid rgba(255,255,255,0.04);
  }

  .tool-group {
    display: flex;
    gap: 2px;
  }

  .tool-btn {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 10px;
    border: none;
    border-radius: 5px;
    background: transparent;
    color: #888;
    font-size: 11px;
    font-weight: 500;
    cursor: pointer;
    transition: all 0.15s;
  }

  .tool-btn:hover {
    background: rgba(255,255,255,0.08);
    color: #fff;
  }

  .tool-btn.active {
    background: rgba(108, 92, 231, 0.2);
    color: #6c5ce7;
  }
</style>
