<script>
  import { onMount, onDestroy } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import RecordingIndicator from './lib/RecordingIndicator.svelte';

  let isRecording = $state(false);
  let duration = $state(0);
  let outputPath = $state('');
  let timer = null;
  let unlisteners = [];

  onMount(async () => {
    // Listen for tray events
    unlisteners.push(
      await listen('tray-start-recording', async () => {
        await startRecording();
      })
    );

    unlisteners.push(
      await listen('tray-stop-recording', async () => {
        await stopRecording();
      })
    );

    unlisteners.push(
      await listen('recording-started', (event) => {
        outputPath = event.payload;
        isRecording = true;
        startTimer();
      })
    );

    unlisteners.push(
      await listen('recording-stopped', (event) => {
        isRecording = false;
        stopTimer();
        // Thumbnail window will be shown by the backend
      })
    );

    // Check initial state
    try {
      const status = await invoke('get_recording_status');
      isRecording = status.is_recording;
      if (isRecording) {
        duration = status.duration_secs;
        startTimer();
      }
    } catch (e) {
      console.error('Failed to get recording status:', e);
    }
  });

  onDestroy(() => {
    stopTimer();
    unlisteners.forEach((unlisten) => unlisten());
  });

  function startTimer() {
    timer = setInterval(() => {
      duration += 0.1;
    }, 100);
  }

  function stopTimer() {
    if (timer) {
      clearInterval(timer);
      timer = null;
    }
    duration = 0;
  }

  async function startRecording() {
    try {
      await invoke('start_recording', { outputPath: null });
    } catch (e) {
      console.error('Failed to start recording:', e);
    }
  }

  async function stopRecording() {
    try {
      await invoke('stop_recording');
    } catch (e) {
      console.error('Failed to stop recording:', e);
    }
  }
</script>

<!-- This app runs in background, UI is minimal -->
<!-- Main interaction is through tray icon -->

{#if isRecording}
  <RecordingIndicator {duration} onStop={stopRecording} />
{/if}

<style>
  :global(body) {
    margin: 0;
    padding: 0;
    background: transparent;
  }
</style>
