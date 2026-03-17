<script>
  let {
    background = { type: 'transparent' },
    onBackgroundChange = () => {},
  } = $props();

  const presets = [
    { type: 'transparent', label: 'None', style: 'background: repeating-conic-gradient(#333 0% 25%, #222 0% 50%) 50%/8px 8px' },
    { type: 'gradient', colors: ['#6c5ce7', '#a29bfe'], angle: 135, label: 'Violet' },
    { type: 'gradient', colors: ['#0984e3', '#74b9ff'], angle: 135, label: 'Ocean' },
    { type: 'gradient', colors: ['#e17055', '#fdcb6e'], angle: 135, label: 'Sunset' },
    { type: 'gradient', colors: ['#00b894', '#55efc4'], angle: 135, label: 'Emerald' },
    { type: 'gradient', colors: ['#0d0d0f', '#2d3436'], angle: 135, label: 'Dark' },
    { type: 'solid', color: '#ffffff', label: 'White' },
    { type: 'solid', color: '#0d0d0f', label: 'Black' },
  ];

  function getStyle(p) {
    if (p.type === 'transparent') return p.style;
    if (p.type === 'gradient') return `background: linear-gradient(${p.angle}deg, ${p.colors[0]}, ${p.colors[1]})`;
    return `background: ${p.color}`;
  }

  function isActive(p) {
    if (p.type !== background.type) return false;
    if (p.type === 'transparent') return true;
    if (p.type === 'gradient') return background.colors?.[0] === p.colors[0];
    return background.color === p.color;
  }

  function select(p) {
    if (p.type === 'transparent') onBackgroundChange({ type: 'transparent' });
    else if (p.type === 'gradient') onBackgroundChange({ type: 'gradient', colors: [...p.colors], angle: p.angle });
    else onBackgroundChange({ type: 'solid', color: p.color });
  }

  function handleCustomColor(e) {
    onBackgroundChange({ type: 'solid', color: e.target.value });
  }
</script>

<div class="bg-row">
  {#each presets as p}
    <button
      class="bg-dot"
      class:active={isActive(p)}
      style={getStyle(p)}
      onclick={() => select(p)}
      title={p.label}
    ></button>
  {/each}
  <input
    type="color"
    class="bg-custom"
    value={background.color || background.colors?.[0] || '#6c5ce7'}
    oninput={handleCustomColor}
    title="Custom color"
  />
</div>

<style>
  .bg-row {
    display: flex;
    align-items: center;
    gap: 4px;
    flex-wrap: wrap;
  }

  .bg-dot {
    width: 22px;
    height: 22px;
    border-radius: 50%;
    border: 2px solid rgba(255,255,255,0.06);
    cursor: pointer;
    transition: all 0.15s;
    padding: 0;
    flex-shrink: 0;
  }

  .bg-dot:hover {
    border-color: rgba(255,255,255,0.3);
    transform: scale(1.1);
  }

  .bg-dot.active {
    border-color: #fff;
    box-shadow: 0 0 0 1px rgba(255,255,255,0.3);
  }

  .bg-custom {
    width: 22px;
    height: 22px;
    border-radius: 50%;
    border: 2px dashed rgba(255,255,255,0.15);
    cursor: pointer;
    padding: 0;
    -webkit-appearance: none;
    appearance: none;
    background: transparent;
    flex-shrink: 0;
  }

  .bg-custom::-webkit-color-swatch-wrapper { padding: 0; }
  .bg-custom::-webkit-color-swatch { border: none; border-radius: 50%; }
</style>
