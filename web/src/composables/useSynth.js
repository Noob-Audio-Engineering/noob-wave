/**
 * Noob-Wave specifics on top of the generic `@noob-audio-engineering/noob-vst-webgui-framework/vue` bridge:
 * grouped parameter handles, presets and a little UI state. The generic
 * pieces (`useNoobVstWebguiFramework`, `useParam`, `hasParam`, `getClient`, `loadState`,
 * `stateToJson`) are re-exported so components import everything from
 * this one module.
 *
 * Rules of use: `useNoobVstWebguiFramework()` is safe at any time; `useSynth()` and the
 * preset helpers need the manifest, so call them only once `ready` is true
 * (App.vue renders the panels under `v-if="ready"`). Handles are created
 * once and cached, so every panel shares the same reactive objects.
 */
import { reactive } from 'vue';
import { getClient, hasParam, loadState, stateToJson, useParam, useNoobVstWebguiFramework } from '@noob-audio-engineering/noob-vst-webgui-framework/vue';
import { FACTORY_PRESETS, loadUserPresets, saveUserPresets } from '../presets.js';

export { useNoobVstWebguiFramework, useParam, hasParam, getClient, loadState, stateToJson };

let groups = null;

/**
 * Every parameter as a reactive handle (`useParam`), grouped the way the
 * panels are:
 *
 *   osc:    table, position, octave, semi, fine, unison, detune, width,
 *           level, phaseRandom, subLevel, subOctave
 *   filter: mode, cutoff, res, env, key
 *   amp:    attack, decay, sustain, release      (amp envelope)
 *   filt:   attack, decay, sustain, release      (filter envelope)
 *   lfo:    rate, shape, pos, cutoff, pitch, retrig
 *   global: velAmp, glide, master, poly
 *
 * Each handle exposes `norm` / `plain` / `text` / `index` / `label` / `on`
 * reactively and `set` / `setPlain` / `setIndex` / `toggle` / `begin` /
 * `end` / `reset` for edits (see `@noob-audio-engineering/noob-vst-webgui-framework/vue`). Built once; needs
 * the manifest, so call after `ready`.
 */
export function useSynth() {
  if (groups) return groups;
  const p = (id) => useParam(id);
  groups = reactive({
    osc: {
      table: p('wt_table'),
      position: p('wt_position'),
      octave: p('osc_octave'),
      semi: p('osc_semi'),
      fine: p('osc_fine'),
      unison: p('unison_voices'),
      detune: p('unison_detune'),
      width: p('unison_width'),
      level: p('osc_level'),
      phaseRandom: p('osc_phase_random'),
      subLevel: p('sub_level'),
      subOctave: p('sub_octave'),
    },
    filter: {
      mode: p('filter_mode'),
      cutoff: p('filter_cutoff'),
      res: p('filter_res'),
      env: p('filter_env'),
      key: p('filter_key'),
    },
    amp: { attack: p('amp_attack'), decay: p('amp_decay'), sustain: p('amp_sustain'), release: p('amp_release') },
    filt: { attack: p('filt_attack'), decay: p('filt_decay'), sustain: p('filt_sustain'), release: p('filt_release') },
    lfo: {
      rate: p('lfo_rate'),
      shape: p('lfo_shape'),
      pos: p('lfo_pos'),
      cutoff: p('lfo_cutoff'),
      pitch: p('lfo_pitch'),
      retrig: p('lfo_retrig'),
    },
    global: { velAmp: p('vel_amp'), glide: p('glide'), master: p('master'), poly: p('poly') },
  });
  return groups;
}

/**
 * UI-only state shared by the panels (never sent to the plug-in):
 * - `preset`: `{ name, index }` of the current preset in `allPresets()` order
 * - `octave`: keyboard octave shift, −3…+3 (KeyboardBar)
 * - `voices`: latest `voices` stream frame, 32 floats: `[0..16)` per-slot
 *   level, `[16..32)` per-slot MIDI note or -1 (App.vue subscribes)
 * - `modulation`: latest `modulation` stream frame: `[0]` live wavetable
 *   position 0..1, `[1]` LFO output -1..1
 */
export const ui = reactive({
  preset: { name: 'Init', index: 0 },
  octave: 0,
  voices: new Float32Array(32),
  modulation: new Float32Array(2),
});

/** Factory presets followed by the user presets from the plug-in's store. */
export function allPresets() {
  return [...FACTORY_PRESETS, ...loadUserPresets()];
}

/**
 * Load preset number `i` of `allPresets()`; the index wraps, so `index ± 1`
 * steps through the list. Unlisted parameters reset to their defaults.
 */
export function loadPreset(i) {
  const list = allPresets();
  if (!list.length) return;
  const idx = ((i % list.length) + list.length) % list.length;
  loadState(list[idx].values || {});
  ui.preset = { name: list[idx].name, index: idx };
}

/**
 * Save the current state as a user preset called `name` (replacing one with
 * the same name) in the plug-in's UI store, and make it current.
 */
export function savePresetAs(name) {
  const list = loadUserPresets().filter((p) => p.name !== name);
  list.push({ name, values: stateToJson() });
  saveUserPresets(list);
  ui.preset = { name, index: allPresets().findIndex((p) => p.name === name) };
}
