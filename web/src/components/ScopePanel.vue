<script setup>
/**
 * Output panel: an oscilloscope over a spectrum analyzer, both framework
 * canvas components driven straight by their streams.
 *
 * Streams: `scope` (a block of mono output samples per frame; the `Scope`
 * component triggers on the rising zero crossing so a steady tone stands
 * still) and `spectrum` (dB per FFT bin; drawn 20 Hz … 20 kHz with a
 * 3 dB/oct tilt and 200 ms fall-off). The components subscribe and
 * unsubscribe themselves, so nothing is sent for this panel once it is
 * destroyed. No parameters, props or emits.
 */
import { onBeforeUnmount, onMounted, ref } from 'vue';
import { Scope, Spectrum } from '@noob-audio-engineering/noob-vst-webgui-framework/components';
import { useStream } from '@noob-audio-engineering/noob-vst-webgui-framework/vue';
import Section from './Section.vue';

const scopeEl = ref(null);
const specEl = ref(null);
let scope = null;
let spec = null;
onMounted(() => {
  scope = new Scope(scopeEl.value, useStream('scope'), { colors: ['#ff8a3d'], fill: true, lineWidth: 1.5 });
  spec = new Spectrum(specEl.value, useStream('spectrum'), { minHz: 20, maxHz: 20000, minDb: -90, maxDb: 6, slopeDbPerOct: 3, color: 'rgba(88,196,255,0.85)', fillColor: 'rgba(88,196,255,0.14)', grid: true, releaseMs: 200 });
});
onBeforeUnmount(() => {
  scope?.destroy();
  spec?.destroy();
});
</script>

<template>
  <Section title="Output" accent="#58c4ff">
    <div class="h-full grid grid-rows-2 gap-2">
      <div ref="scopeEl" class="min-h-0 rounded-lg bg-ink-950/70 border border-white/[0.05]"></div>
      <div ref="specEl" class="min-h-0 rounded-lg bg-ink-950/70 border border-white/[0.05]"></div>
    </div>
  </Section>
</template>
