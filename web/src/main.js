/**
 * Noob-Wave entry point. Vite serves this file in development and bundles
 * it into `dist/` for production, where the standalone binary serves it
 * from disk and the plug-in embeds it (`include_dir!` under `--features
 * plugin`).
 *
 * No global state is set up here: the noob-vst-webgui-framework client is created lazily by
 * the first `useNoobVstWebguiFramework()` call (App.vue) and connects to
 * `ws://<page origin>/ws`, which is the synth's own server, or the Vite
 * dev proxy during development.
 */
import { createApp } from 'vue';
import './style.css';
import App from './App.vue';

createApp(App).mount('#app');
