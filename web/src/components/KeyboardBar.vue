<script setup>
/**
 * The keyboard bar along the bottom: octave shift buttons, the framework
 * `Keyboard` (mouse, multi-touch and QWERTY playing) and a short legend.
 *
 * Notes leave the page as binary event frames (`client.noteOn` /
 * `noteOff`, sent by the `Keyboard` component) and reach the synth's audio
 * thread without touching the parameter system, which is what keeps the
 * key-to-sound path short. Notes arriving from the host (a DAW's MIDI track)
 * come back on the same channel and light the keys in the "remote" colour.
 *
 * The visible range is C2–C6 (MIDI 36–84) shifted by `ui.octave` (−3…+3,
 * changed here with the buttons or by the Z / X keys the component
 * handles itself; the component's `octave` field is kept in sync so its
 * QWERTY mapping moves too). No parameters, props or emits.
 */
import { onBeforeUnmount, onMounted, ref } from 'vue';
import { Keyboard } from '@noob-audio-engineering/noob-vst-webgui-framework/components';
import { getClient } from '@noob-audio-engineering/noob-vst-webgui-framework/vue';
import { ui } from '../composables/useSynth.js';

const el = ref(null);
let kbd = null;
onMounted(() => {
  kbd = new Keyboard(el.value, getClient(), { low: 36, high: 84, velocity: 0.8 });
});
onBeforeUnmount(() => kbd?.destroy());
/** Shift the visible range and the QWERTY mapping by `d` octaves, within −3…+3. */
function shift(d) {
  ui.octave = Math.max(-3, Math.min(3, ui.octave + d));
  if (kbd) {
    kbd.octave = ui.octave;
    kbd.setRange(36 + ui.octave * 12, 84 + ui.octave * 12);
  }
}
</script>

<template>
  <footer class="h-28 shrink-0 flex items-stretch gap-2 px-2 pb-2 pt-1 border-t border-white/[0.06] bg-ink-900/80">
    <div class="w-16 shrink-0 flex flex-col justify-center gap-1 text-[10px] text-slate-500">
      <button class="chip" @click="shift(-1)">− oct</button>
      <div class="text-center tabular text-slate-300">{{ ui.octave >= 0 ? '+' : '' }}{{ ui.octave }}</div>
      <button class="chip" @click="shift(1)">+ oct</button>
    </div>
    <div ref="el" class="flex-1 min-w-0 rounded-lg overflow-hidden border border-white/[0.06]"></div>
    <div class="w-32 shrink-0 text-[10px] text-slate-500 leading-tight flex flex-col justify-center gap-1">
      <div>Play with the mouse, or the <b class="text-slate-300">A W S E D F …</b> keys.</div>
      <div><b class="text-slate-300">Z</b> / <b class="text-slate-300">X</b> shift octaves.</div>
      <div>Host notes light the keys yellow.</div>
    </div>
  </footer>
</template>

<style scoped>
@reference '../style.css';
.chip {
  @apply rounded px-2 py-0.5 text-[10px] border border-white/10 bg-white/[0.04] text-slate-300 hover:bg-white/[0.08];
}
</style>
