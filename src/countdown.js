import Countdown from './lib/Countdown.svelte';
import { mount } from 'svelte';

try {
  const target = document.getElementById('app');
  if (!target) throw new Error('#app element not found');
  target.innerHTML = '';
  mount(Countdown, { target });
  console.log('[countdown] Component mounted');
} catch (e) {
  console.error('[countdown] Mount error:', e);
}
