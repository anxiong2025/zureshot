<script>
  /**
   * CursorSettings — sidebar panel for configuring cursor overlay appearance.
   */
  let {
    cursorStyle = 'pointer',
    cursorSize = 24,
    cursorColor = '#000000',
    showHighlight = true,
    showClickRipple = true,
    onStyleChange = () => {},
    onSizeChange = () => {},
    onColorChange = () => {},
    onHighlightToggle = () => {},
    onRippleToggle = () => {},
  } = $props();

  const styles = [
    { id: 'pointer', label: 'Arrow' },
    { id: 'dot', label: 'Dot' },
    { id: 'ring', label: 'Ring' },
    { id: 'crosshair', label: 'Cross' },
  ];

  const colorPresets = [
    '#000000', '#ffffff', '#007AFF', '#FF3B30',
    '#FF9500', '#34C759', '#AF52DE', '#FFD60A',
  ];

  let isPointer = $derived(cursorStyle === 'pointer');
</script>

<div class="cursor-settings">
  <div class="section-header">Cursor</div>

  <!-- Style Picker -->
  <div class="style-row">
    {#each styles as s}
      <button
        class="style-btn"
        class:active={cursorStyle === s.id}
        onclick={() => onStyleChange(s.id)}
        title={s.label}
      >
        <div class="style-preview"
          class:preview-pointer={s.id === 'pointer'}
          class:preview-dot={s.id === 'dot'}
          class:preview-ring={s.id === 'ring'}
          class:preview-cross={s.id === 'crosshair'}
          style="--cursor-color: {cursorColor}"
        ></div>
        <span>{s.label}</span>
      </button>
    {/each}
  </div>

  <!-- Size Slider -->
  <div class="slider-row">
    <span class="slider-label">Size</span>
    <input
      type="range"
      min="12"
      max="48"
      step="2"
      value={cursorSize}
      oninput={(e) => onSizeChange(Number(e.target.value))}
    />
    <span class="slider-val">{cursorSize}px</span>
  </div>

  <!-- Color Picker (hidden for pointer style since it's always black/white) -->
  {#if !isPointer}
    <div class="color-section">
      <span class="slider-label">Color</span>
      <div class="color-row">
        {#each colorPresets as color}
          <button
            class="color-swatch"
            class:active={cursorColor === color}
            style="background: {color}"
            onclick={() => onColorChange(color)}
          ></button>
        {/each}
        <label class="color-custom" title="Custom color">
          <input type="color" value={cursorColor} oninput={(e) => onColorChange(e.target.value)} />
          <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M12 2L2 7l10 5 10-5-10-5z"/><path d="M2 17l10 5 10-5"/></svg>
        </label>
      </div>
    </div>
  {/if}

  <!-- Toggles -->
  <div class="toggle-section">
    <div class="toggle-row">
      <span class="toggle-label">Spotlight glow</span>
      <button
        class="toggle-btn"
        class:active={showHighlight}
        onclick={() => onHighlightToggle(!showHighlight)}
      >
        {showHighlight ? 'ON' : 'OFF'}
      </button>
    </div>
    <div class="toggle-row">
      <span class="toggle-label">Click ripple</span>
      <button
        class="toggle-btn"
        class:active={showClickRipple}
        onclick={() => onRippleToggle(!showClickRipple)}
      >
        {showClickRipple ? 'ON' : 'OFF'}
      </button>
    </div>
  </div>
</div>

<style>
  .cursor-settings {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 12px;
    background: #141416;
    border-radius: 10px;
    border: 1px solid rgba(255,255,255,0.06);
  }

  .section-header {
    font-size: 11px;
    font-weight: 700;
    color: #888;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    margin-bottom: 2px;
  }

  /* ─── Style Picker ─── */
  .style-row {
    display: flex;
    gap: 5px;
  }

  .style-btn {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 4px;
    padding: 8px 2px;
    border: 1px solid rgba(255,255,255,0.08);
    border-radius: 8px;
    background: rgba(255,255,255,0.03);
    color: #777;
    font-size: 9px;
    font-weight: 600;
    cursor: pointer;
    transition: all 0.15s;
  }

  .style-btn:hover {
    background: rgba(255,255,255,0.06);
    border-color: rgba(255,255,255,0.15);
    color: #ccc;
  }

  .style-btn.active {
    background: rgba(108, 92, 231, 0.12);
    border-color: rgba(108, 92, 231, 0.4);
    color: #b4adff;
  }

  .style-preview {
    width: 20px;
    height: 20px;
    position: relative;
  }

  /* Arrow pointer preview */
  .preview-pointer {
    background: transparent;
  }

  .preview-pointer::before {
    content: '';
    position: absolute;
    top: 1px;
    left: 4px;
    width: 0;
    height: 0;
    border-left: 6px solid #ddd;
    border-top: 3px solid transparent;
    border-bottom: 9px solid transparent;
    transform: rotate(-15deg);
    filter: drop-shadow(0 0.5px 1px rgba(0,0,0,0.6));
  }

  .preview-dot {
    border-radius: 50%;
    background: var(--cursor-color, #007AFF);
    opacity: 0.85;
    box-shadow: 0 0 6px color-mix(in srgb, var(--cursor-color) 50%, transparent);
  }

  .preview-ring {
    border-radius: 50%;
    border: 2px solid var(--cursor-color, #007AFF);
    background: color-mix(in srgb, var(--cursor-color) 15%, transparent);
  }

  .preview-cross {
    background: transparent;
  }

  .preview-cross::before,
  .preview-cross::after {
    content: '';
    position: absolute;
    background: var(--cursor-color, #007AFF);
    border-radius: 1px;
  }

  .preview-cross::before {
    top: 50%; left: 2px; right: 2px; height: 2px;
    transform: translateY(-50%);
  }

  .preview-cross::after {
    left: 50%; top: 2px; bottom: 2px; width: 2px;
    transform: translateX(-50%);
  }

  /* ─── Slider ─── */
  .slider-row {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .slider-label {
    font-size: 10px;
    font-weight: 600;
    color: #666;
    text-transform: uppercase;
    letter-spacing: 0.3px;
    width: 52px;
    flex-shrink: 0;
  }

  .slider-row input[type="range"] {
    flex: 1;
    height: 4px;
    -webkit-appearance: none;
    appearance: none;
    background: rgba(255,255,255,0.08);
    border-radius: 2px;
    outline: none;
  }

  .slider-row input[type="range"]::-webkit-slider-thumb {
    -webkit-appearance: none;
    appearance: none;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: #fff;
    cursor: pointer;
    box-shadow: 0 1px 4px rgba(0,0,0,0.5);
  }

  .slider-val {
    font-size: 10px;
    font-weight: 600;
    color: #888;
    font-variant-numeric: tabular-nums;
    width: 36px;
    text-align: right;
    flex-shrink: 0;
  }

  /* ─── Color ─── */
  .color-section {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .color-row {
    display: flex;
    gap: 5px;
    flex-wrap: wrap;
  }

  .color-swatch {
    width: 22px;
    height: 22px;
    border-radius: 6px;
    border: 2px solid rgba(255,255,255,0.08);
    cursor: pointer;
    transition: all 0.12s;
  }

  .color-swatch:hover {
    transform: scale(1.1);
    border-color: rgba(255,255,255,0.25);
  }

  .color-swatch.active {
    border-color: #fff;
    box-shadow: 0 0 0 1px rgba(255,255,255,0.3);
  }

  .color-custom {
    width: 22px;
    height: 22px;
    border-radius: 6px;
    border: 2px dashed rgba(255,255,255,0.15);
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    position: relative;
    color: #555;
    overflow: hidden;
  }

  .color-custom input[type="color"] {
    position: absolute;
    inset: 0;
    opacity: 0;
    cursor: pointer;
    width: 100%;
    height: 100%;
  }

  .color-custom:hover {
    border-color: rgba(255,255,255,0.35);
    color: #aaa;
  }

  /* ─── Toggles ─── */
  .toggle-section {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding-top: 4px;
    border-top: 1px solid rgba(255,255,255,0.04);
  }

  .toggle-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .toggle-label {
    font-size: 10px;
    font-weight: 600;
    color: #666;
    text-transform: uppercase;
    letter-spacing: 0.3px;
  }

  .toggle-btn {
    padding: 3px 10px;
    border: 1px solid rgba(255,255,255,0.1);
    border-radius: 4px;
    background: transparent;
    color: #666;
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.5px;
    cursor: pointer;
    transition: all 0.15s;
  }

  .toggle-btn.active {
    background: rgba(108,92,231,0.2);
    border-color: #6c5ce7;
    color: #6c5ce7;
  }
</style>
