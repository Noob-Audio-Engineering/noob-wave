<script setup>
/**
 * LFO panel: shape buttons, retrigger, the live value indicator, and the
 * rate knob plus one knob per destination (wavetable position, filter
 * cutoff in octaves, pitch in semitones).
 *
 * Parameters: `lfo_rate`, `lfo_shape`, `lfo_pos`, `lfo_cutoff`,
 * `lfo_pitch`, `lfo_retrig`. The live indicator reads
 * `ui.modulation[1]`, the LFO's current output in -1..1 as published by
 * the DSP on the `modulation` stream (App.vue subscribes); it is drawn as a
 * bar growing left or right from the centre line. No props or emits.
 */
import { computed } from 'vue';
import { Knob } from '@noob-audio-engineering/noob-vst-webgui-framework/vue';
import { ui, useSynth } from '../composables/useSynth.js';
import Section from './Section.vue';

const s = useSynth();
// The LFO's current output (-1..1) from the `modulation` stream.
const live = computed(() => ui.modulation[1] || 0);
const shapes = computed(() => s.lfo.shape.labels);
</script>

<template>
  <Section title="LFO" accent="#c77dff">
    <template #tools>
      <button class="chip" :class="{ on: s.lfo.retrig.on }" title="Restart the LFO on every note" @click="s.lfo.retrig.toggle()">Retrig</button>
    </template>
    <div class="h-full flex flex-col gap-2">
      <div class="flex items-center gap-2">
        <div class="flex gap-1">
          <button v-for="(sh, i) in shapes" :key="sh" class="chip" :class="{ on: s.lfo.shape.index === i }" @click="s.lfo.shape.setIndex(i)">{{ sh }}</button>
        </div>
        <div class="ml-auto flex items-center gap-1.5 text-[10px] text-slate-500">
          <span>live</span>
          <div class="relative w-20 h-2 rounded bg-white/[0.06] overflow-hidden">
            <div class="absolute top-0 bottom-0 w-px bg-white/30 left-1/2" />
            <div class="absolute top-0 bottom-0 bg-[#c77dff]" :style="live >= 0 ? { left: '50%', width: `${live * 50}%` } : { right: '50%', width: `${-live * 50}%` }" />
          </div>
        </div>
      </div>
      <div class="flex-1 flex justify-around items-center">
        <Knob :p="s.lfo.rate" :size="56" label="Rate" color="#c77dff" />
        <Knob :p="s.lfo.pos" :size="56" label="→ Pos" color="#c77dff" />
        <Knob :p="s.lfo.cutoff" :size="56" label="→ Cutoff" color="#c77dff" />
        <Knob :p="s.lfo.pitch" :size="56" label="→ Pitch" color="#c77dff" />
      </div>
    </div>
  </Section>
</template>

<style scoped>
@reference '../style.css';
.chip {
  @apply rounded px-2 py-0.5 text-[10px] border border-white/10 bg-white/[0.04] text-slate-300 hover:bg-white/[0.08] normal-case tracking-normal;
}
.chip.on {
  @apply bg-[#c77dff] text-ink-950 border-transparent font-semibold;
}
</style>
