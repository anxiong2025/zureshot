<script>
  import { onMount } from 'svelte';

  let count = $state(3);
  let visible = $state(true);

  onMount(() => {
    const timer = setInterval(() => {
      count--;
      if (count <= 0) {
        clearInterval(timer);
        visible = false;
      }
    }, 1000);

    return () => clearInterval(timer);
  });
</script>

{#if visible}
<div class="countdown-overlay">
  <div class="countdown-circle">
    {#key count}
    <span class="countdown-number">{count}</span>
    {/key}
  </div>
</div>
{/if}

<style>
  .countdown-overlay {
    width: 100%;
    height: 100%;
    display: flex;
    align-items: center;
    justify-content: center;
    background: transparent;
    user-select: none;
    -webkit-user-select: none;
  }

  .countdown-circle {
    width: 120px;
    height: 120px;
    border-radius: 50%;
    background: rgba(0, 0, 0, 0.7);
    backdrop-filter: blur(30px);
    -webkit-backdrop-filter: blur(30px);
    display: flex;
    align-items: center;
    justify-content: center;
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.4);
  }

  .countdown-number {
    font-family: -apple-system, BlinkMacSystemFont, 'SF Pro Display', 'Helvetica Neue', sans-serif;
    font-size: 56px;
    font-weight: 300;
    color: white;
    line-height: 1;
    animation: pop 0.3s cubic-bezier(0.175, 0.885, 0.32, 1.275);
  }

  /* Re-trigger animation on each count change via Svelte key block below */

  @keyframes fadeIn {
    from { opacity: 0; transform: scale(0.8); }
    to { opacity: 1; transform: scale(1); }
  }

  @keyframes pop {
    0% { transform: scale(0.5); opacity: 0.3; }
    60% { transform: scale(1.15); }
    100% { transform: scale(1); opacity: 1; }
  }
</style>
