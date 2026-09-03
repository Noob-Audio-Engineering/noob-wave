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
 *
 * The analyser is given the host's real sample rate explicitly, from
 * `useSampleRate()`. Left to itself it would read the `spectrum` stream's
 * own `meta.sample_rate`, which is fixed at 48000 when the plug-in builds
 * its manifest and can never be corrected, so every peak would land at the
 * wrong frequency in a session at any other rate. The component reads the
 * rate once, in its constructor, so it is rebuilt when the rate changes.
 */
import { onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { Scope, Spectrum } from '@noob-audio-engineering/noob-vst-webgui-framework/components';
import { useStream } from '@noob-audio-engineering/noob-vst-webgui-framework/vue';
import { useSampleRate } from '../composables/useSynth.js';
import Section from './Section.vue';

const scopeEl = ref(null);
const specEl = ref(null);
const sampleRate = useSampleRate();
let scope = null;
let spec = null;

function buildSpectrum() {
  spec?.destroy();
  const sr = sampleRate.value;
  spec = new Spectrum(specEl.value, useStream('spectrum'), {
    sampleRate: sr,
    minHz: 20,
    maxHz: Math.min(20000, sr * 0.5),
    minDb: -90,
    maxDb: 6,
    slopeDbPerOct: 3,
    color: 'rgba(88,196,255,0.85)',
    fillColor: 'rgba(88,196,255,0.14)',
    grid: true,
    releaseMs: 200,
  });
}
onMounted(() => {
  scope = new Scope(scopeEl.value, useStream('scope'), { colors: ['#ff8a3d'], fill: true, lineWidth: 1.5 });
  buildSpectrum();
});
watch(sampleRate, () => {
  if (specEl.value) buildSpectrum();
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
