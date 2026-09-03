<script setup>
/**
 * Noob-Wave root component: a header, a fixed 3 × 2 grid of panels
 * (oscillator, filter, output / oscillator, envelopes, LFO, global) and the
 * on-screen keyboard along the bottom. Sized for the plug-in's default
 * 1080 × 640 editor; the grid uses fractions, so it scales with the window.
 *
 * Nothing but a "connecting…" placeholder renders until `ready` is true,
 * because every panel asks for parameter handles, which need the manifest.
 *
 * Streams subscribed here and shared through `ui`: `voices` (per-voice
 * level and note, for the header count and the Global panel) and
 * `modulation` (live wavetable position and LFO value, for the wavetable
 * view and the LFO indicator). They are attached with a short poll for
 * `ready` because `useStream` needs the manifest too and this component
 * mounts before it arrives.
 *
 * Keyboard (window level; ignored while an input has focus): Ctrl/Cmd+Z
 * undo, Ctrl/Cmd+Y or Ctrl/Cmd+Shift+Z redo, Ctrl/Cmd+B A/B compare. Note
 * keys (A W S E D …) are handled by the framework `Keyboard` component in
 * KeyboardBar.vue.
 */
import { onBeforeUnmount, onMounted } from 'vue';
import { useStream } from '@noob-audio-engineering/noob-vst-webgui-framework/vue';
import { ui, useNoobVstWebguiFramework } from './composables/useSynth.js';
import Header from './components/Header.vue';
import WavetablePanel from './components/WavetablePanel.vue';
import FilterPanel from './components/FilterPanel.vue';
import EnvPanel from './components/EnvPanel.vue';
import LfoPanel from './components/LfoPanel.vue';
import MasterPanel from './components/MasterPanel.vue';
import ScopePanel from './components/ScopePanel.vue';
import KeyboardBar from './components/KeyboardBar.vue';

const { ready, connected, client, history } = useNoobVstWebguiFramework();
let offs = [];

function onKey(e) {
  const t = e.target;
  if (t && (t.tagName === 'INPUT' || t.tagName === 'SELECT' || t.tagName === 'TEXTAREA')) return;
  const mod = e.ctrlKey || e.metaKey;
  if (mod && e.key.toLowerCase() === 'z' && !e.shiftKey) {
    history.undo();
    e.preventDefault();
  } else if ((mod && e.key.toLowerCase() === 'y') || (mod && e.shiftKey && e.key.toLowerCase() === 'z')) {
    history.redo();
    e.preventDefault();
  } else if (mod && e.key.toLowerCase() === 'b') {
    history.toggleAB();
    e.preventDefault();
  }
}
onMounted(() => {
  window.addEventListener('keydown', onKey);
  // Streams need the manifest; poll briefly for it, then subscribe once.
  const stop = setInterval(() => {
    if (!ready.value) return;
    clearInterval(stop);
    offs.push(useStream('voices').on((d) => (ui.voices = d)));
    offs.push(useStream('modulation').on((d) => (ui.modulation = d)));
  }, 50);
});
onBeforeUnmount(() => {
  window.removeEventListener('keydown', onKey);
  offs.forEach((f) => f());
});
</script>

<template>
  <div class="h-full flex flex-col bg-ink-950 text-slate-200 overflow-hidden">
    <template v-if="ready">
      <Header />
      <main class="flex-1 min-h-0 grid gap-2 p-2" style="grid-template-columns: 1.5fr 1fr 1fr; grid-template-rows: 1fr 1fr">
        <WavetablePanel class="row-span-1" />
        <FilterPanel />
        <ScopePanel />
        <EnvPanel />
        <LfoPanel />
        <MasterPanel />
      </main>
      <KeyboardBar />
    </template>
    <div v-else class="flex-1 grid place-items-center text-slate-500 text-sm">
      <div class="text-center">
        <div class="mb-1">{{ connected ? 'waiting for manifest…' : 'connecting to synth…' }}</div>
        <div class="text-[11px] text-slate-600 tabular">{{ client.url }}</div>
      </div>
    </div>
  </div>
</template>
