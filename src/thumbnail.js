import Thumbnail from './lib/Thumbnail.svelte';
import { mount } from 'svelte';

const app = mount(Thumbnail, {
  target: document.getElementById('app'),
});

export default app;
