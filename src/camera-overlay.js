import CameraOverlay from './lib/CameraOverlay.svelte';
import { mount } from 'svelte';

try {
  const target = document.getElementById('app');
  if (!target) throw new Error('#app element not found');
  target.innerHTML = '';
  mount(CameraOverlay, { target });
  console.log('[camera-overlay] Component mounted');
} catch (e) {
  console.error('[camera-overlay] Mount error:', e);
}
