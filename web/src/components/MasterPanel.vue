<script setup>
/**
 * Global panel: velocity → amp, glide, polyphony and master knobs, the
 * 16-slot voice activity strip, and the output level meter.
 *
 * Parameters: `vel_amp`, `glide`, `poly`, `master`. Streams: `voices`
 * through `ui.voices` (App.vue subscribes) — 32 floats, `[0..16)` the
 * envelope level of each voice slot (0 = idle) and `[16..32)` its MIDI note
 * or -1; slots beyond the current polyphony are dimmed — and `meter_out`
 * through the framework `LevelMeter`. No props or emits.
 */
import { computed } from 'vue';
import { Knob, LevelMeter } from '@noob-audio-engineering/noob-vst-webgui-framework/vue';
import { noteName } from '@noob-audio-engineering/noob-vst-webgui-framework/vue';
import { ui, useSynth } from '../composables/useSynth.js';
import Section from './Section.vue';

const s = useSynth();
// One entry per voice slot from the `voices` stream: levels in the first
// half of the array, MIDI notes (or -1 when idle) in the second half.
const voices = computed(() => Array.from({ length: 16 }, (_, i) => ({ level: ui.voices[i] || 0, note: ui.voices[16 + i] })));
</script>

<template>
  <Section title="Global">
    <div class="h-full flex gap-3">
      <div class="flex-1 flex flex-col gap-2">
        <div class="flex justify-around">
          <Knob :p="s.global.velAmp" :size="52" label="Vel→Amp" />
          <Knob :p="s.global.glide" :size="52" label="Glide" />
          <Knob :p="s.global.poly" :size="52" label="Voices" />
          <Knob :p="s.global.master" :size="52" label="Master" />
        </div>
        <div class="flex-1 min-h-0 grid grid-cols-8 gap-1 content-end">
          <div
            v-for="(v, i) in voices"
            :key="i"
            class="relative h-7 rounded bg-white/[0.05] overflow-hidden text-[9px] text-center text-slate-400 leading-7"
            :class="{ 'opacity-40': i >= s.global.poly.plain }"
            :title="`Voice ${i + 1}`"
          >
            <div class="absolute left-0 right-0 bottom-0 bg-accent/70" :style="{ height: `${Math.min(1, v.level) * 100}%` }" />
            <span class="relative">{{ v.note >= 0 ? noteName(v.note) : '' }}</span>
          </div>
        </div>
      </div>
      <div class="w-7 shrink-0 flex flex-col items-center gap-1">
        <div class="flex-1 min-h-0 w-full"><LevelMeter stream="meter_out" :min-db="-60" :max-db="6" /></div>
        <span class="text-[9px] uppercase tracking-wider text-slate-500">Out</span>
      </div>
    </div>
  </Section>
</template>
