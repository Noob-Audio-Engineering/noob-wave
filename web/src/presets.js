/**
 * Factory presets: `{ id: plain }` maps; anything not listed loads at its
 * default. User presets live in the plug-in's UI store (`client.store`), so
 * they persist with the plug-in state and every window of the instance
 * sees them.
 */
import { getClient } from '@noob-audio-engineering/noob-vst-webgui-framework/vue';

/**
 * @typedef {Object} Preset
 * @property {string} name
 * @property {Object.<string, number>} values
 *   Parameter id → plain value in the parameter's own unit: Hz for
 *   `filter_cutoff` and `lfo_rate`, seconds for envelope times, percent
 *   for levels / sustain / amounts, cents for `unison_detune` and
 *   `osc_fine`, dB for `master`, enum index for `wt_table`, `filter_mode`,
 *   `lfo_shape`, `sub_octave`, and 0 / 1 for toggles. Anything not listed
 *   loads at its default.
 */

/**
 * Factory presets, in menu order. `Init` is the empty canvas (every
 * parameter at its default). Index 0..n of this array followed by the user
 * presets is what `allPresets()` in the composable returns.
 * @type {Preset[]}
 */
export const FACTORY_PRESETS = [
  { name: 'Init', values: {} },
  {
    name: 'Warm Pad',
    values: {
      wt_table: 0, wt_position: 0.35, unison_voices: 5, unison_detune: 22, unison_width: 90, osc_level: 70,
      filter_mode: 1, filter_cutoff: 1800, filter_res: 10, filter_env: 25, filter_key: 40,
      amp_attack: 0.6, amp_decay: 1.0, amp_sustain: 80, amp_release: 1.4,
      filt_attack: 1.2, filt_decay: 1.5, filt_sustain: 40, filt_release: 1.0,
      lfo_rate: 0.3, lfo_shape: 0, lfo_pos: 25, lfo_cutoff: 0.4, master: -9, poly: 8,
    },
  },
  {
    name: 'Pluck',
    values: {
      wt_table: 1, wt_position: 0.55, unison_voices: 2, unison_detune: 8, unison_width: 30, osc_level: 85,
      filter_mode: 0, filter_cutoff: 600, filter_res: 25, filter_env: 70, filter_key: 60,
      amp_attack: 0.002, amp_decay: 0.35, amp_sustain: 0, amp_release: 0.25,
      filt_attack: 0.002, filt_decay: 0.18, filt_sustain: 0, filt_release: 0.2,
      vel_amp: 90, master: -6, poly: 8,
    },
  },
  {
    name: 'Sub Bass',
    values: {
      wt_table: 0, wt_position: 0.1, osc_octave: -1, unison_voices: 1, osc_level: 80, sub_level: 70, sub_octave: 0,
      filter_mode: 1, filter_cutoff: 400, filter_res: 15, filter_env: 40, filter_key: 20,
      amp_attack: 0.005, amp_decay: 0.3, amp_sustain: 90, amp_release: 0.15,
      filt_attack: 0.003, filt_decay: 0.25, filt_sustain: 20, filt_release: 0.15,
      glide: 0.06, master: -4, poly: 1,
    },
  },
  {
    name: 'Super Lead',
    values: {
      wt_table: 2, wt_position: 0.4, unison_voices: 7, unison_detune: 35, unison_width: 100, osc_level: 75,
      filter_mode: 0, filter_cutoff: 6000, filter_res: 10, filter_env: 20, filter_key: 50,
      amp_attack: 0.01, amp_decay: 0.3, amp_sustain: 85, amp_release: 0.35,
      lfo_rate: 5.5, lfo_shape: 0, lfo_pitch: 0.15, lfo_retrig: 1, glide: 0.04, master: -8, poly: 6,
    },
  },
  {
    name: 'Vowel Keys',
    values: {
      wt_table: 3, wt_position: 0.2, unison_voices: 3, unison_detune: 10, unison_width: 60, osc_level: 80,
      filter_mode: 0, filter_cutoff: 9000, filter_res: 5, filter_env: 0,
      amp_attack: 0.01, amp_decay: 0.6, amp_sustain: 55, amp_release: 0.5,
      lfo_rate: 0.8, lfo_shape: 1, lfo_pos: 60, master: -7, poly: 8,
    },
  },
  {
    name: 'Digital Bells',
    values: {
      wt_table: 5, wt_position: 0.7, osc_octave: 1, unison_voices: 2, unison_detune: 6, unison_width: 80, osc_level: 60,
      filter_mode: 0, filter_cutoff: 12000, filter_res: 0, filter_env: 30,
      amp_attack: 0.003, amp_decay: 1.8, amp_sustain: 0, amp_release: 1.6,
      filt_attack: 0.003, filt_decay: 1.0, filt_sustain: 0, filt_release: 1.0,
      lfo_rate: 4, lfo_shape: 0, lfo_pos: 15, master: -8, poly: 10,
    },
  },
  {
    name: 'Sync Stab',
    values: {
      wt_table: 4, wt_position: 0.6, unison_voices: 3, unison_detune: 12, unison_width: 50, osc_level: 80,
      filter_mode: 1, filter_cutoff: 900, filter_res: 35, filter_env: 80, filter_key: 30,
      amp_attack: 0.002, amp_decay: 0.5, amp_sustain: 20, amp_release: 0.3,
      filt_attack: 0.002, filt_decay: 0.3, filt_sustain: 10, filt_release: 0.2,
      lfo_rate: 6, lfo_shape: 4, lfo_pos: 35, lfo_retrig: 1, master: -6, poly: 6,
    },
  },
];

/** UI-store key holding the user presets (`Preset[]`). */
export const USER_KEY = 'presets.user';

/**
 * The user presets currently in the store. Empty until the store has been
 * hydrated (right after connect) and whenever nothing was saved yet.
 * @returns {Preset[]}
 */
export function loadUserPresets() {
  const v = getClient().store.get(USER_KEY, []);
  return Array.isArray(v) ? v : [];
}
/**
 * Replace the user presets. The plug-in persists the value with its state
 * and pushes it to every other window of this instance.
 * @param {Preset[]} list
 */
export function saveUserPresets(list) {
  getClient().store.set(USER_KEY, list);
}
/**
 * Re-run `fn` when the user presets change elsewhere: another window saved
 * one, or the host restored the plug-in's state. Returns an unsubscribe.
 */
export function onUserPresetsChange(fn) {
  return getClient().store.on('*', (k) => {
    if (k == null || k === USER_KEY) fn();
  });
}
