<script setup>
/**
 * Filter panel: mode buttons (LP 12, LP 24, BP, HP), the magnitude
 * response drawn from the current settings, and the cutoff / resonance /
 * envelope amount / key tracking knobs.
 *
 * Parameters: `filter_mode`, `filter_cutoff`, `filter_res`, `filter_env`,
 * `filter_key`. No streams: the curve is computed in the browser from the
 * parameters (`responseDb`), mirroring the Rust TPT state-variable filter
 * in `examples/noob-wave/src/dsp/filter.rs`, so the drawing follows knob
 * drags with zero round-trip. It shows the static response: envelope, key
 * tracking and LFO offsets are not folded in. No props or emits.
 *
 * Drawing is coalesced with `requestAnimationFrame` (`schedule`): several
 * parameter changes in one frame cost one redraw, and a `ResizeObserver`
 * redraws on layout changes. The canvas is sized for the device pixel
 * ratio so lines stay crisp on high-DPI screens.
 */
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { Knob } from '@noob-audio-engineering/noob-vst-webgui-framework/vue';
import { useSynth } from '../composables/useSynth.js';
import Section from './Section.vue';

const s = useSynth();
const canvas = ref(null);
let raf = null;

/**
 * Magnitude in dB at `freq` of the synth's TPT state-variable filter, for
 * display only. Mirrors `examples/noob-wave/src/dsp/filter.rs`:
 *   g = tan(π·fc/fs)          the prewarped cutoff,
 *   k = 2 − 1.98·res          the damping (res 0..1 → k 2..0.02, so full
 *                             resonance is just short of self-oscillation),
 * and the analogue prototype evaluated at the prewarped frequency
 * x = tan(π·f/fs) / g, which gives the exact response of the discrete
 * filter rather than the analogue approximation. Modes: 0 LP 12 dB,
 * 1 LP 24 dB (a second low-pass stage with slightly less resonance, as
 * the DSP cascades it), 2 band-pass, 3 high-pass.
 */
function responseDb(freq, cutoff, res, mode, sr) {
  const g = Math.tan((Math.PI * Math.min(cutoff, sr * 0.45)) / sr);
  const k = 2 - 1.98 * res;
  const w = Math.tan((Math.PI * Math.min(freq, sr * 0.499)) / sr); // prewarped
  // Analog prototype magnitude with s = j*w/g
  const x = w / g;
  const den = Math.sqrt(Math.pow(1 - x * x, 2) + Math.pow(k * x, 2));
  let mag;
  if (mode === 2) mag = (k * x) / den;
  else if (mode === 3) mag = (x * x) / den;
  else mag = 1 / den;
  let db = 20 * Math.log10(Math.max(mag, 1e-6));
  if (mode === 1) db += 20 * Math.log10(Math.max(1 / Math.sqrt(Math.pow(1 - x * x, 2) + Math.pow((2 - 1.98 * res * 0.7) * x, 2)), 1e-6));
  return db;
}

function draw() {
  const c = canvas.value;
  if (!c) return;
  const dpr = window.devicePixelRatio || 1;
  const w = c.clientWidth;
  const h = c.clientHeight;
  c.width = w * dpr;
  c.height = h * dpr;
  const ctx = c.getContext('2d');
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, w, h);
  const sr = 48000;
  const minHz = 20;
  const maxHz = 20000;
  const xFor = (f) => (Math.log(f / minHz) / Math.log(maxHz / minHz)) * w;
  const yFor = (db) => h * 0.15 + (-db / 48) * h * 0.8;
  ctx.strokeStyle = 'rgba(255,255,255,0.06)';
  for (const f of [100, 1000, 10000]) {
    ctx.beginPath();
    ctx.moveTo(xFor(f), 0);
    ctx.lineTo(xFor(f), h);
    ctx.stroke();
  }
  ctx.beginPath();
  ctx.moveTo(0, yFor(0));
  ctx.lineTo(w, yFor(0));
  ctx.stroke();
  const cutoff = s.filter.cutoff.plain;
  const res = s.filter.res.plain / 100;
  const mode = s.filter.mode.index;
  ctx.beginPath();
  for (let x = 0; x <= w; x++) {
    const f = minHz * Math.pow(maxHz / minHz, x / w);
    const y = yFor(responseDb(f, cutoff, res, mode, sr));
    if (x === 0) ctx.moveTo(x, y);
    else ctx.lineTo(x, y);
  }
  ctx.strokeStyle = '#58c4ff';
  ctx.lineWidth = 2;
  ctx.stroke();
  ctx.lineTo(w, h);
  ctx.lineTo(0, h);
  ctx.closePath();
  ctx.fillStyle = 'rgba(88,196,255,0.12)';
  ctx.fill();
}
const schedule = () => {
  if (raf) return;
  raf = requestAnimationFrame(() => {
    raf = null;
    draw();
  });
};
watch(() => [s.filter.cutoff.norm, s.filter.res.norm, s.filter.mode.index], schedule);
let ro = null;
onMounted(() => {
  ro = new ResizeObserver(schedule);
  ro.observe(canvas.value);
  schedule();
});
onBeforeUnmount(() => {
  ro?.disconnect();
  if (raf) cancelAnimationFrame(raf);
});
const modes = computed(() => s.filter.mode.labels);
</script>

<template>
  <Section title="Filter" accent="#58c4ff">
    <template #tools>
      <div class="flex gap-1">
        <button v-for="(m, i) in modes" :key="m" class="chip" :class="{ on: s.filter.mode.index === i }" @click="s.filter.mode.setIndex(i)">{{ m }}</button>
      </div>
    </template>
    <div class="h-full flex flex-col gap-2">
      <canvas ref="canvas" class="flex-1 min-h-0 w-full rounded-lg bg-ink-950/70 border border-white/[0.05]"></canvas>
      <div class="flex justify-around">
        <Knob :p="s.filter.cutoff" :size="56" label="Cutoff" color="#58c4ff" />
        <Knob :p="s.filter.res" :size="56" label="Reso" color="#58c4ff" />
        <Knob :p="s.filter.env" :size="56" label="Env" color="#58c4ff" />
        <Knob :p="s.filter.key" :size="56" label="Key" color="#58c4ff" />
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
  @apply bg-accent-2/90 text-ink-950 border-transparent font-semibold;
}
</style>
