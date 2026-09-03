<script setup>
/**
 * Envelopes panel: the amp and filter ADSRs, each as a draggable shape
 * (framework `Envelope`: drag the attack, decay / sustain and release
 * handles) next to four small knobs for the same values.
 *
 * Parameters: `amp_attack` / `amp_decay` / `amp_sustain` / `amp_release`
 * and the `filt_*` four. Times are seconds (skewed range up to 10 s),
 * sustain is a percentage. No streams, props or emits.
 *
 * The `Envelope` component expects a parameter-like object for each stage
 * (`plain`, `on`, `beginEdit`, `endEdit`, `setPlain`) with sustain in
 * 0..1; the sustain parameters are in percent, so `pct` wraps a Vue handle
 * into that shape and converts both ways. The time stages hand over the
 * raw `param` objects, which already have that interface.
 */
import { onBeforeUnmount, onMounted, ref } from 'vue';
import { Envelope } from '@noob-audio-engineering/noob-vst-webgui-framework/components';
import { Knob } from '@noob-audio-engineering/noob-vst-webgui-framework/vue';
import { useSynth } from '../composables/useSynth.js';
import Section from './Section.vue';

const s = useSynth();
const ampEl = ref(null);
const filtEl = ref(null);
let envs = [];

// The sustain params are in percent; the Envelope wants 0..1, so wrap them.
const pct = (h) => ({
  get plain() {
    return h.plain / 100;
  },
  on: (fn) => h.param.on(fn),
  beginEdit: () => h.begin(),
  endEdit: () => h.end(),
  setPlain: (v) => h.setPlain(v * 100),
});
onMounted(() => {
  envs.push(new Envelope(ampEl.value, { attack: s.amp.attack.param, decay: s.amp.decay.param, sustain: pct(s.amp.sustain), release: s.amp.release.param, color: '#ff8a3d' }));
  envs.push(new Envelope(filtEl.value, { attack: s.filt.attack.param, decay: s.filt.decay.param, sustain: pct(s.filt.sustain), release: s.filt.release.param, color: '#58c4ff' }));
});
onBeforeUnmount(() => envs.forEach((e) => e.destroy()));
</script>

<template>
  <Section title="Envelopes">
    <div class="h-full grid grid-rows-2 gap-2">
      <div class="flex gap-2 min-h-0">
        <div class="w-10 shrink-0 text-[10px] uppercase tracking-wider text-accent pt-1">Amp</div>
        <div ref="ampEl" class="flex-1 min-w-0 rounded-lg bg-ink-950/70 border border-white/[0.05]"></div>
        <div class="flex gap-1 shrink-0">
          <Knob :p="s.amp.attack" :size="42" label="A" />
          <Knob :p="s.amp.decay" :size="42" label="D" />
          <Knob :p="s.amp.sustain" :size="42" label="S" />
          <Knob :p="s.amp.release" :size="42" label="R" />
        </div>
      </div>
      <div class="flex gap-2 min-h-0">
        <div class="w-10 shrink-0 text-[10px] uppercase tracking-wider text-accent-2 pt-1">Filter</div>
        <div ref="filtEl" class="flex-1 min-w-0 rounded-lg bg-ink-950/70 border border-white/[0.05]"></div>
        <div class="flex gap-1 shrink-0">
          <Knob :p="s.filt.attack" :size="42" label="A" color="#58c4ff" />
          <Knob :p="s.filt.decay" :size="42" label="D" color="#58c4ff" />
          <Knob :p="s.filt.sustain" :size="42" label="S" color="#58c4ff" />
          <Knob :p="s.filt.release" :size="42" label="R" color="#58c4ff" />
        </div>
      </div>
    </div>
  </Section>
</template>
