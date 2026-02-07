<script>
  import { listen } from '@tauri-apps/api/event';

  let region = $state(null);

  // Listen for the region coordinates from the Rust backend
  listen('recording-region', (event) => {
    region = event.payload;
    console.log('[recording-overlay] Region received:', region);
  });
</script>

{#if region}
  <div class="overlay">
    <!-- Top dim -->
    <div class="dim" style="top:0;left:0;right:0;height:{region.y}px;"></div>
    <!-- Left dim -->
    <div class="dim" style="top:{region.y}px;left:0;width:{region.x}px;height:{region.height}px;"></div>
    <!-- Right dim -->
    <div class="dim" style="top:{region.y}px;left:{region.x + region.width}px;right:0;height:{region.height}px;"></div>
    <!-- Bottom dim -->
    <div class="dim" style="top:{region.y + region.height}px;left:0;right:0;bottom:0;"></div>
    <!-- Subtle border around the recorded region (2px outside to avoid being captured) -->
    <div
      class="region-border"
      style="left:{region.x - 2}px;top:{region.y - 2}px;width:{region.width + 4}px;height:{region.height + 4}px;"
    ></div>
  </div>
{/if}

<style>
  .overlay {
    position: fixed;
    top: 0;
    left: 0;
    width: 100vw;
    height: 100vh;
    pointer-events: none;
  }

  .dim {
    position: absolute;
    background: rgba(0, 0, 0, 0.25);
    pointer-events: none;
    /* Smooth transition when overlay appears */
    animation: fadeIn 0.3s ease;
  }

  .region-border {
    position: absolute;
    border: 1px solid rgba(59, 130, 246, 0.4);
    border-radius: 2px;
    pointer-events: none;
    box-shadow: 0 0 12px rgba(59, 130, 246, 0.1);
  }

  @keyframes fadeIn {
    from { opacity: 0; }
    to { opacity: 1; }
  }
</style>
