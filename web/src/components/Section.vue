<script setup>
/**
 * A panel frame used by every section of the grid: a rounded card with a
 * header line (coloured dot, upper-case title, an optional tool area on
 * the right) and a body that fills the rest.
 *
 * Props: `title` (String), `accent` (CSS colour for the dot; defaults to
 * the theme accent, the filter and output panels pass the blue
 * `accent-2`, the LFO its purple). Slots: default (the body; it gets
 * `min-h-0` so canvases inside can shrink with the grid) and `tools`
 * (small controls in the header, e.g. the wavetable selector or the filter
 * mode buttons).
 */
defineProps({ title: { type: String, default: '' }, accent: { type: String, default: 'var(--color-accent)' } });
</script>

<template>
  <section class="min-h-0 flex flex-col rounded-xl border border-white/[0.07] bg-ink-900/70 overflow-hidden">
    <div class="flex items-center gap-2 px-3 py-1.5 border-b border-white/[0.06] text-[10px] uppercase tracking-[0.14em] text-slate-400">
      <span class="w-1.5 h-1.5 rounded-full" :style="{ background: accent }" />
      <span>{{ title }}</span>
      <div class="ml-auto flex items-center gap-2"><slot name="tools" /></div>
    </div>
    <div class="flex-1 min-h-0 p-2.5"><slot /></div>
  </section>
</template>
