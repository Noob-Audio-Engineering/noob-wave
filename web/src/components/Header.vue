<script setup>
/**
 * Header: connection dot and byline, undo / redo / A-B, previous / current /
 * next preset with a dropdown menu (factory presets, user presets, Save
 * As…), the active voice count, the live edit→echo round trip and the
 * sample rate the host reported.
 *
 * No props or emits. Uses the framework `History` for undo / redo / A-B,
 * `modified` (set by the framework when any local edit completes, cleared
 * on preset load) for the asterisk, `stats.echoAvgMs` measured by the
 * client, the once-a-second `status` message (`sample_rate`), and
 * `ui.voices` to count slots with a level above zero. User presets are
 * stored in the plug-in's UI store under `presets.user` (see presets.js).
 */
import { computed, ref } from 'vue';
import { ContextMenu } from '@noob-audio-engineering/noob-vst-webgui-framework/vue';
import { allPresets, loadPreset, savePresetAs, ui, useNoobVstWebguiFramework } from '../composables/useSynth.js';

const { history, historyState, connected, stats, status, modified } = useNoobVstWebguiFramework();
const menu = ref({ open: false, x: 0, y: 0 });
const fmt = (ms) => (Number.isNaN(ms) ? '–' : ms < 1 ? `${Math.round(ms * 1000)} µs` : `${ms.toFixed(2)} ms`);
// Active voices = slots whose level (first half of the `voices` stream) is above zero.
const voices = computed(() => {
  let n = 0;
  for (let i = 0; i < 16; i++) if (ui.voices[i] > 0) n++;
  return n;
});
// The preset menu: one entry per preset (factory first, then user), then Save As.
// Built on open, so a preset saved in another window shows up next time.
const items = computed(() => [
  ...allPresets().map((p, i) => ({ label: p.name, checked: ui.preset.name === p.name, action: () => loadPreset(i) })),
  { divider: true },
  {
    label: 'Save As…',
    action: () => {
      const name = window.prompt('Preset name', ui.preset.name === 'Init' ? 'My Preset' : ui.preset.name);
      if (name) savePresetAs(name);
    },
  },
]);
function openMenu(e) {
  const r = e.currentTarget.getBoundingClientRect();
  menu.value = { open: true, x: r.left, y: r.bottom + 4 };
}
</script>

<template>
  <header class="h-10 shrink-0 flex items-center gap-2 px-3 border-b border-white/[0.06] bg-ink-900/80 select-none">
    <div class="flex items-center gap-2 mr-2">
      <span class="w-2 h-2 rounded-full" :class="connected ? 'bg-emerald-400 shadow-[0_0_8px_rgba(52,211,153,.8)]' : 'bg-red-500'" />
      <span class="font-bold tracking-wide text-[13px]"><span class="text-accent">NOOB</span>-WAVE</span>
      <span class="text-slate-600 text-[11px]">wavetable synth · by Ely Erin Fox</span>
    </div>
    <button class="tb" :disabled="!historyState.canUndo" title="Undo (Ctrl+Z)" @click="history.undo()">↶</button>
    <button class="tb" :disabled="!historyState.canRedo" title="Redo (Ctrl+Y)" @click="history.redo()">↷</button>
    <button class="tb w-9" title="A/B (Ctrl+B)" @click="history.toggleAB()"><b class="text-accent">{{ historyState.ab }}</b><span class="text-slate-500">/{{ historyState.ab === 'A' ? 'B' : 'A' }}</span></button>
    <div class="flex items-center gap-1 mx-2">
      <button class="tb" title="Previous preset" @click="loadPreset(ui.preset.index - 1)">‹</button>
      <button class="tb min-w-[180px] text-center" :class="{ 'text-slate-400': modified }" @click="openMenu">{{ ui.preset.name }}<span v-if="modified"> *</span></button>
      <button class="tb" title="Next preset" @click="loadPreset(ui.preset.index + 1)">›</button>
    </div>
    <div class="ml-auto flex items-center gap-3 text-[11px] text-slate-500 tabular">
      <span>voices <b class="text-slate-200">{{ voices }}</b></span>
      <span title="Time from sending an edit until the synth echoes it back">edit→echo <b class="text-emerald-300 font-medium">{{ fmt(stats.echoAvgMs) }}</b></span>
      <span v-if="status?.sample_rate">{{ status.sample_rate }} Hz</span>
    </div>
    <ContextMenu :open="menu.open" :x="menu.x" :y="menu.y" :items="items" @close="menu.open = false" />
  </header>
</template>

<style scoped>
@reference '../style.css';
.tb {
  @apply rounded px-2 py-1 text-[11px] border border-white/10 bg-white/[0.04] text-slate-200 hover:bg-white/[0.09] disabled:opacity-30 disabled:hover:bg-white/[0.04] transition-colors leading-4;
}
</style>
