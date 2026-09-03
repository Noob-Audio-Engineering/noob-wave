<script setup>
/**
 * Oscillator panel: the wavetable drawn as a stack of frames with the
 * selected frame highlighted and the live (LFO-modulated) frame shown
 * next to it, plus the table selector and the position / level / sub /
 * pitch / unison knobs.
 *
 * Parameters: `wt_table` (which table; the DSP publishes a new
 * `wavetable` stream frame whenever it changes), `wt_position`,
 * `osc_level`, `sub_level`, `sub_octave`, `osc_octave`, `osc_semi`,
 * `osc_fine`, `unison_voices`, `unison_detune`, `unison_width`,
 * `osc_phase_random`. Streams: `wavetable` (sticky: the whole table as
 * `frames × samples` floats, with `meta.frames`; being sticky, a page that
 * connects after the plug-in published it still receives it), and
 * `modulation[0]` through `ui.modulation` for the live position.
 *
 * The framework `WavetableView` owns the canvas and the drag / wheel
 * gesture on it, which edits `wt_position` directly (it receives the raw
 * `param` object, not the Vue handle). No props or emits.
 *
 * The knob column is taller than this panel's grid cell in a short window,
 * so it scrolls; without that the third row and the two buttons are simply
 * cut off and unreachable, which they were at every size up to about
 * 1000 × 760, the editor's 1080 × 640 default included.
 */
import { onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { WavetableView } from '@noob-audio-engineering/noob-vst-webgui-framework/components';
import { Knob, useStream } from '@noob-audio-engineering/noob-vst-webgui-framework/vue';
import { ui, useSynth } from '../composables/useSynth.js';
import Section from './Section.vue';

const s = useSynth();
const el = ref(null);
let view = null;
let offs = [];

onMounted(() => {
  view = new WavetableView(el.value, { position: s.osc.position.param, color: '#ff8a3d', nearColor: 'rgba(255,138,61,0.55)' });
  const wt = useStream('wavetable');
  const apply = (d) => view.setTable(d, wt.meta.frames || 32);
  // The stream is sticky, so its last frame may already be here from the
  // handshake; draw it now, then follow table changes.
  if (wt.data.length) apply(wt.data);
  offs.push(wt.on(apply));
});
watch(() => ui.modulation, (m) => view?.setLivePosition(m[0]));
onBeforeUnmount(() => {
  offs.forEach((f) => f());
  view?.destroy();
});
</script>

<template>
  <Section title="Oscillator">
    <template #tools>
      <select class="sel" :value="s.osc.table.index" @change="s.osc.table.setIndex(Number($event.target.value))">
        <option v-for="(l, i) in s.osc.table.labels" :key="l" :value="i">{{ l }}</option>
      </select>
    </template>
    <div class="h-full flex gap-3">
      <div ref="el" class="flex-1 min-w-0 rounded-lg bg-ink-950/70 border border-white/[0.05]" title="Drag vertically or scroll to morph"></div>
      <!--
        The three knob rows and the buttons need more height than this cell
        gets in a short window, so the column scrolls rather than clipping
        its last row: Unison, Detune, Width and both buttons stay reachable
        at every window size, including the editor's own default.
      -->
      <div class="shrink-0 min-h-0 overflow-y-auto grid grid-cols-3 gap-x-2 gap-y-1 content-start">
        <Knob :p="s.osc.position" :size="54" label="Position" />
        <Knob :p="s.osc.level" :size="54" label="Level" />
        <Knob :p="s.osc.subLevel" :size="54" label="Sub" color="#58c4ff" />
        <Knob :p="s.osc.octave" :size="46" label="Oct" />
        <Knob :p="s.osc.semi" :size="46" label="Semi" />
        <Knob :p="s.osc.fine" :size="46" label="Fine" />
        <Knob :p="s.osc.unison" :size="46" label="Unison" />
        <Knob :p="s.osc.detune" :size="46" label="Detune" />
        <Knob :p="s.osc.width" :size="46" label="Width" />
        <div class="col-span-3 flex items-center gap-2 text-[10px] text-slate-500 pt-1">
          <button class="chip" :class="{ on: s.osc.phaseRandom.on }" title="Random start phase per note" @click="s.osc.phaseRandom.toggle()">Rand φ</button>
          <button class="chip" :class="{ on: s.osc.subOctave.index === 1 }" title="Sub oscillator octave" @click="s.osc.subOctave.setIndex(s.osc.subOctave.index === 0 ? 1 : 0)">Sub {{ s.osc.subOctave.label }}</button>
        </div>
      </div>
    </div>
  </Section>
</template>

<style scoped>
@reference '../style.css';
.sel {
  @apply rounded bg-ink-700 border border-white/10 px-2 py-0.5 text-[11px] text-slate-200 normal-case tracking-normal;
}
.chip {
  @apply rounded px-2 py-0.5 text-[10px] border border-white/10 bg-white/[0.04] text-slate-300 hover:bg-white/[0.08];
}
.chip.on {
  @apply bg-accent/90 text-ink-950 border-transparent font-semibold;
}
</style>
