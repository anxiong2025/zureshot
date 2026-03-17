import VideoEditor from './lib/editor/VideoEditor.svelte';
import { mount } from 'svelte';

const app = mount(VideoEditor, {
  target: document.getElementById('app'),
});

export default app;
