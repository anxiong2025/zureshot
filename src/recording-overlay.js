import RecordingOverlay from './lib/RecordingOverlay.svelte';
import { mount } from 'svelte';

try {
  const target = document.getElementById('app');
  if (!target) throw new Error('#app element not found');
  target.innerHTML = '';
  mount(RecordingOverlay, { target });
  console.log('[recording-overlay] Component mounted');
} catch (e) {
  console.error('[recording-overlay] Mount error:', e);
}
