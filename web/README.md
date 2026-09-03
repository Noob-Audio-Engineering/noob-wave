# Noob-Wave UI

The user interface of **Noob-Wave**, the free wavetable synth by Noob Audio Engineering built on
[noob-vst-webgui-framework](https://github.com/Noob-Audio-Engineering/noob-vst-webgui-framework). It is a Vue 3 +
Tailwind v4 single-page app built with Vite. Inside a DAW it renders in the
plug-in's own window, which is the operating system's web view; during
development it renders in any browser. Either way it talks to the Rust
synth over one loopback WebSocket using the `@noob-audio-engineering/noob-vst-webgui-framework` client library,
and notes played on the page reach the audio thread as binary event frames.

Everything specific to the synth lives here and in the Rust crate one level
up (this repository, see its [README](../README.md)). Everything generic, the client,
the reactive parameter handles, undo / redo, the knob, the scope, spectrum,
wavetable, envelope and keyboard renderers, lives in the framework's browser
package (see [its README](https://github.com/Noob-Audio-Engineering/noob-vst-webgui-framework/blob/main/crates/noob-vst-webgui-framework/web/README.md)).

## Development workflow

Requirements: Node 20 or newer and the Rust toolchain for the standalone.

```sh
cd web
npm install          # fetches @noob-audio-engineering/noob-vst-webgui-framework from its GitHub repository
npm run build        # writes dist/, which the Rust side serves or embeds
```

Run the page against the real synth without a DAW (audio goes to the
default output device):

```sh
# from the repository root: serves web/dist on port 4243 (or the next free port)
cargo run --bin noob-wave-standalone -- --open
```

Hot reload while editing the UI:

```sh
# terminal 1: the standalone (keep it running; note the port it printed)
cargo run --bin noob-wave-standalone
# terminal 2: Vite on http://localhost:5174, proxying to the standalone
cd web && NOOB_VST_WEBGUI_FRAMEWORK_PORT=4243 npm run dev
```

The dev server proxies `/ws` (the WebSocket) and `/instance` + `/instances`
(the discovery endpoints) to `NOOB_VST_WEBGUI_FRAMEWORK_PORT`. The standalone prefers 4243
and walks up if that port is taken, so use the port from its start-up
banner. Noob-Q's dev server uses 5173, so both can run at once.

The plug-in build embeds `dist/` into the binary (`include_dir!`), so run
`npm run build` before `cargo build --features plugin`; the root README has
the full plug-in build steps.

## How the page talks to the synth

1. `useNoobVstWebguiFramework()` creates one `NoobVstWebguiFrameworkClient` connected to
   `ws://<page origin>/ws`. The manifest it receives describes the 35
   parameters (id, name, unit, range, taper table, enum labels) and the
   telemetry streams.
2. Components use reactive **parameter handles** (`useSynth()` groups them
   by panel). Reading `handle.plain` / `text` / `index` is reactive;
   `set()` / `setPlain()` / `setIndex()` / `toggle()` send edits;
   `begin()` / `end()` bracket a drag into one host automation gesture.
3. **Notes** go out as binary events (`client.noteOn` / `noteOff`, sent by
   the framework `Keyboard`), bypassing the parameter system, and notes
   from the host come back the same way to light the keys.
4. **Streams** deliver telemetry as `Float32Array`s: `scope`, `spectrum`,
   `meter_out`, `voices`, `modulation`, and the sticky `wavetable` (the
   whole table, republished when `wt_table` changes).
5. The **UI store** (`client.store`) holds the user presets under
   `presets.user`, persisted with the plug-in state and shared by every
   window of the instance.
6. Messages: the page can send `reset`; the host sends `status` once a
   second and `sample_rate` on initialise.

Parameter ids by group: `wt_table`, `wt_position`, `osc_octave`,
`osc_semi`, `osc_fine`, `unison_voices`, `unison_detune`, `unison_width`,
`osc_level`, `osc_phase_random`, `sub_level`, `sub_octave`; `filter_mode`,
`filter_cutoff`, `filter_res`, `filter_env`, `filter_key`; `amp_attack`,
`amp_decay`, `amp_sustain`, `amp_release` and the `filt_*` four;
`lfo_rate`, `lfo_shape`, `lfo_pos`, `lfo_cutoff`, `lfo_pitch`,
`lfo_retrig`; `vel_amp`, `glide`, `master`, `poly`.

## Component tree

```
App.vue                     3 × 2 panel grid, stream subscriptions shared through `ui`, undo / redo keys
├── Header.vue              connection dot, undo / redo / A-B, preset menu (factory, user, Save As), voices, edit→echo, sample rate
├── WavetablePanel.vue      wavetable stack with the live frame, table selector, position / level / sub / pitch / unison knobs
├── FilterPanel.vue         mode buttons, response curve computed in the browser, cutoff / reso / env / key knobs
├── ScopePanel.vue          oscilloscope and spectrum of the output
├── EnvPanel.vue            amp and filter ADSRs as draggable shapes plus knobs
├── LfoPanel.vue            shape, retrigger, live value bar, rate and destination knobs
├── MasterPanel.vue         velocity / glide / voices / master knobs, voice activity strip, output meter
├── KeyboardBar.vue         octave buttons, the on-screen keyboard, legend
├── Section.vue             the card frame every panel uses (title, accent dot, `tools` slot)
└── (from @noob-audio-engineering/noob-vst-webgui-framework/vue) Knob, ContextMenu, LevelMeter
```

Every component starts with a doc block listing its props, emits, the
parameters and streams it touches and any drawing or performance trick.

## The composable layer (`src/composables/useSynth.js`)

| Export | Purpose |
|---|---|
| `useNoobVstWebguiFramework()` | Connection state (`ready`, `connected`, `manifest`, `stats`, `status`, `modified`) and `history`. Safe before `ready`. |
| `useSynth()` | Every handle grouped as `osc`, `filter`, `amp`, `filt`, `lfo`, `global`. Built once, needs the manifest. |
| `allPresets()` / `loadPreset(i)` / `savePresetAs(name)` | Factory + user presets, wrap-around loading, saving into the store. |
| `ui` | `preset { name, index }`, `octave`, and the latest `voices` / `modulation` frames. |
| re-exports | `useParam`, `hasParam`, `getClient`, `loadState`, `stateToJson` from `@noob-audio-engineering/noob-vst-webgui-framework/vue`. |

Stream layouts used by the panels: `voices` is 32 floats, `[0..16)` the
level of each voice slot and `[16..32)` its MIDI note or −1; `modulation`
is `[position 0..1, lfo −1..1]`; `wavetable` is `frames × samples` with
`meta.frames`.

## Presets and the store

`src/presets.js` holds the factory presets (`{ name, values }`, `values`
maps parameter id → plain value in the parameter's own unit; anything
unlisted loads at its default) and the store helpers: `loadUserPresets()`,
`saveUserPresets(list)`, `onUserPresetsChange(fn)`. The store key is
`presets.user`. Nothing uses `localStorage`.

## Styling

Tailwind v4 in CSS-first mode (`src/style.css`): `@theme` declares the
`ink-*` surface palette, the orange `accent` (oscillator, amp envelope,
scope) and the blue `accent-2` (filter, filter envelope, spectrum); the LFO
uses a literal purple. `@source` adds the shared Vue components of
`@noob-audio-engineering/noob-vst-webgui-framework` to Tailwind's scan; the `--noob-vst-webgui-framework-*` custom properties
colour the framework's canvas components, including the keyboard's key
colours. A scoped `<style>` that uses `@apply` starts with
`@reference '../style.css'`.

## Adding things

**A new control**: get the handle from `useSynth()` and drop a
`<Knob :p="handle" />` in a panel, or wire a `.chip` button to
`handle.toggle()` / `handle.setIndex(i)`.

**A new parameter**: add it on the Rust side (the `Params` struct and
`param_map` in `src/plugin.rs`, `build_bridge` and `read_settings` in
`src/dsp/mod.rs`, and the `Settings` field the DSP reads), rebuild, then add
it to the matching group in `useSynth()`.

**A new panel**: create a component wrapped in `<Section title="…">`, give
it the doc block the others have, and add it to the grid in `App.vue`
(adjust `grid-template-columns` / `rows` there).

**A new telemetry stream**: declare it in `src/dsp/mod.rs`, publish it from
`Telemetry::publish`, then `useStream(id).on(frame => …)` in the component,
or subscribe once in `App.vue` and share it through `ui` as `voices` and
`modulation` are.

## Keyboard and mouse

Undo / redo Ctrl+Z / Ctrl+Y, A/B Ctrl+B. Play with the mouse or touch on
the keyboard, or with the computer keys A W S E D F T G Y H U J K (a
chromatic octave from C), Z / X shift the octave. Drag or scroll the
wavetable view to morph, drag the envelope handles, double-click a knob to
type a value, Ctrl+click a knob to reset it.

## See also

- [`../README.md`](../README.md): the Rust crate (DSP, plug-in, standalone)
- [the framework's browser package](https://github.com/Noob-Audio-Engineering/noob-vst-webgui-framework/blob/main/crates/noob-vst-webgui-framework/web/README.md): the `@noob-audio-engineering/noob-vst-webgui-framework`
  client library and Vue layer this UI is built on
- [the framework's `docs/`](https://github.com/Noob-Audio-Engineering/noob-vst-webgui-framework/tree/main/docs): the wire format, ports, the UI store
  and the rest of the framework documentation
