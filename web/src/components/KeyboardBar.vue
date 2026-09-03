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
 * The visible range is C2–C6 (MIDI 36–84) shifted by `ui.octave` (−3…+3),
 * changed with the buttons here or with the Z / X keys the component
 * handles itself. Both routes end in `applyOctave`, so the drawn keys, the
 * computer keys and the read-out always agree and `a` is the lowest visible
 * C whichever was used. No parameters, props or emits.
 *
 * Keeping them in step takes a little care. The component derives its
 * QWERTY base note as `low + 12·octave`, and `setRange` is what sets `low`,
 * so moving both fields shifts the letter keys twice as far as the drawn
 * ones. `ui.octave` is therefore the single source of truth: it moves the
 * drawn range, and the component's own `octave` is held at zero. Z and X
 * change that field directly, so a `keydown` listener registered after the
 * component's folds whatever it did back into `ui.octave` and zeroes it
 * again.
 */
import { onBeforeUnmount, onMounted, ref } from 'vue';
import { Keyboard } from '@noob-audio-engineering/noob-vst-webgui-framework/components';
import { getClient } from '@noob-audio-engineering/noob-vst-webgui-framework/vue';
import { ui } from '../composables/useSynth.js';

/** Lowest and highest drawn note at octave 0, and the shift limits. */
const LOW = 36;
const HIGH = 84;
const MIN_OCT = -3;
const MAX_OCT = 3;

const el = ref(null);
let kbd = null;

/** Move the drawn range to `oct` octaves and put the QWERTY base back on its lowest key. */
function applyOctave(oct) {
  ui.octave = Math.max(MIN_OCT, Math.min(MAX_OCT, oct));
  if (!kbd) return;
  kbd.octave = 0;
  kbd.setRange(LOW + ui.octave * 12, HIGH + ui.octave * 12);
}

// Z / X move the component's own octave field. Runs after the component's
// handler, which is registered on `window` in its constructor.
function onKeyDown() {
  if (kbd && kbd.octave !== 0) applyOctave(ui.octave + kbd.octave);
}

onMounted(() => {
  kbd = new Keyboard(el.value, getClient(), { low: LOW, high: HIGH, velocity: 0.8 });
  applyOctave(ui.octave);
  window.addEventListener('keydown', onKeyDown);
});
onBeforeUnmount(() => {
  window.removeEventListener('keydown', onKeyDown);
  kbd?.destroy();
});
/** Shift the visible range and the QWERTY mapping by `d` octaves, within −3…+3. */
function shift(d) {
  applyOctave(ui.octave + d);
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
    <div class="w-40 shrink-0 text-[10px] text-slate-500 leading-tight flex flex-col justify-center gap-1">
      <div>Play with the mouse, or the computer keys.</div>
      <div><b class="text-slate-300">a</b> is the lowest visible C; <b class="text-slate-300">Z</b> / <b class="text-slate-300">X</b> and the buttons shift octaves together.</div>
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
