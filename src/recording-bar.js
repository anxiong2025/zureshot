import RecordingBar from './lib/RecordingBar.svelte';
import { mount } from 'svelte';

try {
  const target = document.getElementById('app');
  if (!target) throw new Error('#app element not found');
  target.innerHTML = '';
  const app = mount(RecordingBar, { target });
  console.log('[recording-bar] Component mounted');
} catch (e) {
  console.error('[recording-bar] Mount error:', e);
}
