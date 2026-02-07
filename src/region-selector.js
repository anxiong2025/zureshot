import RegionSelector from './lib/RegionSelector.svelte';
import { mount } from 'svelte';

console.log('[region-selector] JS loaded, mounting component...');

try {
  const target = document.getElementById('app');
  if (!target) {
    throw new Error('#app element not found');
  }
  // Clear the loading fallback
  target.innerHTML = '';
  const app = mount(RegionSelector, { target });
  console.log('[region-selector] Component mounted successfully');
} catch (e) {
  console.error('[region-selector] Mount error:', e);
  const el = document.getElementById('app');
  if (el) {
    el.innerHTML = `<pre style="color:#ff6b6b;background:#1a1a2e;padding:24px;font-size:14px;white-space:pre-wrap;word-break:break-all;margin:0;height:100vh;overflow:auto;">
Region Selector Error:\n${e.message}\n\n${e.stack}</pre>`;
  }
}
