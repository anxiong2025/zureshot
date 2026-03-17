<script>
  /**
   * CursorOverlay — renders a realistic cursor over the video preview,
   * synced to playback time. Supports macOS pointer, dot, ring, crosshair styles.
   *
   * Uses Catmull-Rom spline interpolation for silky-smooth cursor motion.
   */
  let {
    mouseTrack = null,
    currentTime = 0,
    videoWidth = 1920,
    videoHeight = 1080,
    zoom = { zoom: 1.0, centerX: 0.5, centerY: 0.5 },
    cursorStyle = 'pointer',    // 'pointer' | 'dot' | 'ring' | 'crosshair'
    cursorSize = 24,
    cursorColor = '#000000',
    showHighlight = true,
    showClickRipple = true,
    enabled = true,
  } = $props();

  // ─── Active click ripples ───
  let ripples = $state([]);
  let lastClickTime = $state(-1);

  // ─── Catmull-Rom spline interpolation for smooth cursor motion ───
  function catmullRom(p0, p1, p2, p3, t) {
    const t2 = t * t;
    const t3 = t2 * t;
    return 0.5 * (
      (2 * p1) +
      (-p0 + p2) * t +
      (2 * p0 - 5 * p1 + 4 * p2 - p3) * t2 +
      (-p0 + 3 * p1 - 3 * p2 + p3) * t3
    );
  }

  // ─── Cursor position at current time (Catmull-Rom interpolated) ───
  let cursorPos = $derived.by(() => {
    if (!mouseTrack || !mouseTrack.samples || mouseTrack.samples.length === 0 || !enabled) {
      return null;
    }

    const samples = mouseTrack.samples;
    const t = currentTime;

    if (t <= samples[0].time) return { x: samples[0].x, y: samples[0].y, clicked: samples[0].clicked };
    if (t >= samples[samples.length - 1].time) {
      const last = samples[samples.length - 1];
      return { x: last.x, y: last.y, clicked: last.clicked };
    }

    // Binary search for bracketing samples
    let lo = 0, hi = samples.length - 1;
    while (hi - lo > 1) {
      const mid = (lo + hi) >> 1;
      if (samples[mid].time <= t) lo = mid;
      else hi = mid;
    }

    const a = samples[lo], b = samples[hi];
    const span = b.time - a.time;
    const frac = span > 0 ? (t - a.time) / span : 0;

    // Get 4 control points for Catmull-Rom (clamp at boundaries)
    const i0 = Math.max(0, lo - 1);
    const i3 = Math.min(samples.length - 1, hi + 1);
    const p0 = samples[i0], p3 = samples[i3];

    const x = catmullRom(p0.x, a.x, b.x, p3.x, frac);
    const y = catmullRom(p0.y, a.y, b.y, p3.y, frac);

    // Click detection: check if ANY sample in a small window is clicked
    // (catches short clicks that might fall between frames)
    let clicked = b.clicked;
    if (!clicked) {
      const windowStart = Math.max(0, lo - 1);
      const windowEnd = Math.min(samples.length - 1, hi + 1);
      for (let k = windowStart; k <= windowEnd; k++) {
        if (samples[k].clicked && Math.abs(samples[k].time - t) < 0.06) {
          clicked = true;
          break;
        }
      }
    }

    return { x, y, clicked };
  });

  // ─── Normalized position (0-1) and viewport transform ───
  let displayPos = $derived.by(() => {
    if (!cursorPos) return null;

    const nx = cursorPos.x / videoWidth;
    const ny = cursorPos.y / videoHeight;

    const z = zoom.zoom;
    if (z > 1.01) {
      const halfW = 0.5 / z;
      const halfH = 0.5 / z;
      const visLeft = zoom.centerX - halfW;
      const visTop = zoom.centerY - halfH;

      const vx = (nx - visLeft) / (halfW * 2);
      const vy = (ny - visTop) / (halfH * 2);

      if (vx < -0.05 || vx > 1.05 || vy < -0.05 || vy > 1.05) return null;

      return { x: vx * 100, y: vy * 100, clicked: cursorPos.clicked };
    }

    return { x: nx * 100, y: ny * 100, clicked: cursorPos.clicked };
  });

  // ─── Click ripple detection ───
  $effect(() => {
    if (!displayPos || !displayPos.clicked || !showClickRipple) return;

    if (currentTime - lastClickTime < 0.12) return;

    lastClickTime = currentTime;
    const id = Date.now() + Math.random();
    ripples = [...ripples, { id, x: displayPos.x, y: displayPos.y, born: performance.now() }];

    setTimeout(() => {
      ripples = ripples.filter(r => r.id !== id);
    }, 700);
  });

  // Pointer style uses black arrow, ignore cursorColor
  let isPointer = $derived(cursorStyle === 'pointer');
</script>

{#if enabled && displayPos}
  <div class="cursor-layer" style="--cc: {cursorColor}; --cc-a85: {cursorColor}d9; --cc-a50: {cursorColor}80; --cc-a20: {cursorColor}33; --cc-a10: {cursorColor}1a; --cc-full: {cursorColor}ff;">
    <!-- Cursor highlight (spotlight glow) -->
    {#if showHighlight}
      <div
        class="cursor-highlight"
        class:pointer-highlight={isPointer}
        style="left: {displayPos.x}%; top: {displayPos.y}%; width: {cursorSize * 3}px; height: {cursorSize * 3}px;"
      ></div>
    {/if}

    <!-- macOS Pointer Arrow (matches Screenize's CG path: 17x25 design space) -->
    {#if cursorStyle === 'pointer'}
      <div
        class="cursor-pointer"
        style="left: {displayPos.x}%; top: {displayPos.y}%; width: {cursorSize * 0.68}px; height: {cursorSize}px;"
        class:clicking={displayPos.clicked}
      >
        <svg viewBox="0 0 17 25" fill="none" xmlns="http://www.w3.org/2000/svg">
          <g filter="url(#shadow)">
            <path d="M1.5 1L1.5 18.5L5.5 14.5L9.5 22.5L12 21.5L8 13.5L13.5 13.5Z"
              fill="black" stroke="white" stroke-width="1.2" stroke-linejoin="round"/>
          </g>
          <defs>
            <filter id="shadow" x="-2" y="-1" width="22" height="30" filterUnits="userSpaceOnUse">
              <feDropShadow dx="0" dy="1" stdDeviation="1.2" flood-opacity="0.35"/>
            </filter>
          </defs>
        </svg>
      </div>
    <!-- Dot -->
    {:else if cursorStyle === 'dot'}
      <div
        class="cursor-dot"
        style="left: {displayPos.x}%; top: {displayPos.y}%; width: {cursorSize}px; height: {cursorSize}px;"
        class:clicking={displayPos.clicked}
      ></div>
    <!-- Ring -->
    {:else if cursorStyle === 'ring'}
      <div
        class="cursor-ring"
        style="left: {displayPos.x}%; top: {displayPos.y}%; width: {cursorSize}px; height: {cursorSize}px;"
        class:clicking={displayPos.clicked}
      ></div>
    <!-- Crosshair -->
    {:else if cursorStyle === 'crosshair'}
      <div
        class="cursor-crosshair"
        style="left: {displayPos.x}%; top: {displayPos.y}%; width: {cursorSize * 1.5}px; height: {cursorSize * 1.5}px;"
        class:clicking={displayPos.clicked}
      ></div>
    {/if}

    <!-- Click ripples -->
    {#each ripples as ripple (ripple.id)}
      <div
        class="click-ripple"
        class:pointer-ripple={isPointer}
        style="left: {ripple.x}%; top: {ripple.y}%;"
      ></div>
    {/each}
  </div>
{/if}

<style>
  .cursor-layer {
    position: absolute;
    inset: 0;
    pointer-events: none;
    z-index: 10;
    overflow: hidden;
  }

  /* ─── Cursor Highlight (spotlight glow) ─── */
  .cursor-highlight {
    position: absolute;
    transform: translate(-50%, -50%);
    border-radius: 50%;
    background: radial-gradient(circle, var(--cc-a10, rgba(0, 0, 0, 0.08)) 0%, transparent 70%);
    pointer-events: none;
    transition: left 0.07s ease-out, top 0.07s ease-out;
  }

  .cursor-highlight.pointer-highlight {
    background: radial-gradient(circle, rgba(0, 122, 255, 0.08) 0%, transparent 70%);
  }

  /* ─── macOS Pointer Arrow ─── */
  .cursor-pointer {
    position: absolute;
    /* Arrow tip is at top-left of the SVG, so offset by 0 (no centering) */
    transform: translate(-12%, -8%);
    pointer-events: none;
    transition: left 0.07s ease-out, top 0.07s ease-out;
    filter: drop-shadow(0 1px 2px rgba(0,0,0,0.35));
  }

  .cursor-pointer svg {
    width: 100%;
    height: 100%;
  }

  .cursor-pointer.clicking {
    transform: translate(-12%, -8%) scale(0.85);
    transition: left 0.07s ease-out, top 0.07s ease-out, transform 0.08s ease-out;
  }

  /* ─── Dot Style ─── */
  .cursor-dot {
    position: absolute;
    transform: translate(-50%, -50%);
    border-radius: 50%;
    background: var(--cc-a85, rgba(255, 80, 80, 0.85));
    box-shadow: 0 0 8px var(--cc-a50, rgba(255, 80, 80, 0.5)), 0 0 2px rgba(0,0,0,0.4);
    transition: left 0.07s ease-out, top 0.07s ease-out, transform 0.1s ease;
    pointer-events: none;
  }

  .cursor-dot.clicking {
    transform: translate(-50%, -50%) scale(0.7);
    background: var(--cc-full, rgba(255, 50, 50, 1));
    box-shadow: 0 0 16px var(--cc-a85, rgba(255, 50, 50, 0.8));
  }

  /* ─── Ring Style ─── */
  .cursor-ring {
    position: absolute;
    transform: translate(-50%, -50%);
    border-radius: 50%;
    border: 2px solid var(--cc-a85, rgba(255, 80, 80, 0.8));
    background: var(--cc-a10, rgba(255, 80, 80, 0.1));
    transition: left 0.07s ease-out, top 0.07s ease-out, transform 0.1s ease;
    pointer-events: none;
  }

  .cursor-ring.clicking {
    transform: translate(-50%, -50%) scale(0.75);
    border-color: var(--cc-full, rgba(255, 50, 50, 1));
    background: var(--cc-a50, rgba(255, 50, 50, 0.3));
    box-shadow: 0 0 12px var(--cc-a50, rgba(255, 50, 50, 0.6));
  }

  /* ─── Crosshair Style ─── */
  .cursor-crosshair {
    position: absolute;
    transform: translate(-50%, -50%);
    pointer-events: none;
    transition: left 0.07s ease-out, top 0.07s ease-out;
  }

  .cursor-crosshair::before,
  .cursor-crosshair::after {
    content: '';
    position: absolute;
    background: var(--cc-a85, rgba(255, 80, 80, 0.8));
    border-radius: 1px;
  }

  .cursor-crosshair::before {
    top: 50%; left: 0; right: 0; height: 2px;
    transform: translateY(-50%);
  }

  .cursor-crosshair::after {
    left: 50%; top: 0; bottom: 0; width: 2px;
    transform: translateX(-50%);
  }

  .cursor-crosshair.clicking::before,
  .cursor-crosshair.clicking::after {
    background: var(--cc-full, rgba(255, 50, 50, 1));
    box-shadow: 0 0 6px var(--cc-a50, rgba(255, 50, 50, 0.6));
  }

  /* ─── Click Ripple ─── */
  .click-ripple {
    position: absolute;
    width: 50px;
    height: 50px;
    transform: translate(-50%, -50%) scale(0.2);
    border-radius: 50%;
    border: 2px solid var(--cc-a85, rgba(255, 80, 80, 0.8));
    background: var(--cc-a20, rgba(255, 80, 80, 0.12));
    pointer-events: none;
    animation: ripple-expand 0.6s ease-out forwards;
  }

  .click-ripple.pointer-ripple {
    border-color: rgba(0, 122, 255, 0.6);
    background: rgba(0, 122, 255, 0.08);
  }

  @keyframes ripple-expand {
    0% {
      transform: translate(-50%, -50%) scale(0.2);
      opacity: 1;
      border-width: 3px;
    }
    100% {
      transform: translate(-50%, -50%) scale(2.0);
      opacity: 0;
      border-width: 0.5px;
    }
  }
</style>
