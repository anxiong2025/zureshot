import ScreenshotPreview from './lib/ScreenshotPreview.svelte';
import { mount } from 'svelte';

const app = mount(ScreenshotPreview, {
  target: document.getElementById('app'),
});

export default app;
