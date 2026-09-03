//! noob-wave-standalone: the synth without a DAW. Audio goes to the default
//! output device through cpal (the callback *is* the real-time thread), notes
//! come from the browser's on-screen keyboard over noob-vst-webgui-framework's event frames,
//! and the SPA is served from `web/dist` (or proxied by `vite`).
//!
//!     cargo run --bin noob-wave-standalone -- [--port 4243] [--open] [--dir path] [--silent]
//!
//! # Flags
//!
//! | flag | effect |
//! |---|---|
//! | `--port N`, `-p N` | insist on port `N` (fails if taken); default: probe from 4243 upwards |
//! | `--open`, `-o` | open the page in the system browser once the server is up |
//! | `--dir path`, `-d path` | serve the page from `path` instead of `web/dist` |
//! | `--silent` | do not open an audio device; render on a paced thread so the UI still works |
//! | `-h`, `--help` | usage |
//!
//! # Threads
//!
//! * **cpal callback** — owns the `Engine`: drains browser events, reads
//!   the parameters, renders, publishes telemetry, converts to the device's
//!   sample format. With `--silent` (or no device) the same engine runs on
//!   a thread paced to real time instead.
//! * **main thread** — `host_loop`: drains page edits (there is no host to
//!   forward them to; they are only counted), handles `reset`, sends a
//!   `status` message once a second, flushes the UI store to disk when it
//!   changed.
//! * noob-vst-webgui-framework's pump and network threads, owned by `noob-vst-webgui-framework`.
//!
//! # Ports, discovery and the UI store
//!
//! The server probes from port 4243 upwards (`ServerConfig::prefer_port`),
//! so a second copy, or noob-q on 4242, never collides; it publishes a
//! discovery record so `/instances` and `tools/instances.mjs` list it. The
//! page's UI store (user presets) is kept in
//! `<per-user data dir>/noob-vst-webgui-framework/noob-wave.store.json` by [`FileStore`]; the
//! plug-in keeps the same store in its host state instead.
//!
//! # Hot reload
//!
//! Run `NOOB_VST_WEBGUI_FRAMEWORK_PORT=<port> npm run dev` in `web/`; Vite serves the page
//! and proxies `/ws` and `/instance*` to this process.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use cpal::Sample;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use noob_vst_webgui_framework::{
    AudioHandle, FileStore, NoobVstWebguiFramework, ServerConfig, UiEvent, event_kind,
};
use noob_wave::dsp::{self, ParamIx, Settings, Synth, Telemetry};
use serde_json::json;

/// Largest chunk rendered in one go; longer cpal buffers are rendered in
/// several passes.
const MAX_BLOCK: usize = 4096;

/// Everything the audio callback owns. Built off-thread, then moved in.
struct Engine {
    /// Audio-thread side of the bridge: parameters and events in,
    /// telemetry out.
    audio: AudioHandle,
    /// Parameter indices resolved from ids.
    ix: ParamIx,
    /// The sound engine.
    synth: Synth,
    /// Telemetry publisher.
    telemetry: Telemetry,
    /// Left render buffer, `MAX_BLOCK` long.
    l: Vec<f32>,
    /// Right render buffer, `MAX_BLOCK` long.
    r: Vec<f32>,
    /// Last snapshot handed to the synth, to skip unchanged blocks.
    settings: Settings,
    /// Blocks rendered so far; read by the status message.
    blocks: Arc<AtomicU64>,
}

impl Engine {
    /// Apply one event from the page: note on (a velocity of 0 counts as
    /// note off), note off, pitch bend (`value` -1..1 → ±2 semitones), and
    /// CC 120 / 123 → all notes off. Other kinds are ignored.
    fn handle_event(&mut self, e: UiEvent) {
        match e.kind {
            event_kind::NOTE_ON if e.value > 0.0 => self.synth.note_on(e.a, e.value),
            event_kind::NOTE_ON | event_kind::NOTE_OFF => self.synth.note_off(e.a),
            event_kind::PITCH_BEND => self.synth.set_pitch_bend(e.value * 2.0),
            event_kind::CONTROL if e.a == 123 || e.a == 120 => self.synth.all_notes_off(),
            _ => {}
        }
    }

    /// Render `frames` samples (capped at `MAX_BLOCK`) into the internal
    /// buffers: drain up to 64 browser events (they carry no timing, so all
    /// apply at the block start), read the parameters and reconfigure the
    /// synth if anything changed, render, publish telemetry.
    fn render(&mut self, frames: usize) {
        let mut pending = [None; 64];
        let mut n = 0;
        self.audio.drain_events(|e| {
            if n < pending.len() {
                pending[n] = Some(e);
                n += 1;
            }
        });
        for e in pending.iter().take(n).flatten() {
            self.handle_event(*e);
        }
        let s = dsp::read_settings(&self.audio, &self.ix);
        if s != self.settings {
            self.settings = s;
            self.synth.configure(&s);
        }
        let frames = frames.min(MAX_BLOCK);
        let (l, r) = (&mut self.l[..frames], &mut self.r[..frames]);
        self.synth.render(l, r);
        self.telemetry.publish(&mut self.audio, &self.synth, l, r);
        self.blocks.fetch_add(1, Ordering::Relaxed);
    }
}

/// Fill a cpal buffer of `channels` interleaved samples from the engine,
/// rendering in `MAX_BLOCK` pieces and converting from `f32` to the
/// device's sample type. Channels beyond the second get silence.
fn write_interleaved<T: cpal::Sample + cpal::FromSample<f32>>(
    engine: &mut Engine,
    data: &mut [T],
    channels: usize,
) {
    let frames = data.len() / channels.max(1);
    let mut done = 0;
    while done < frames {
        let n = (frames - done).min(MAX_BLOCK);
        engine.render(n);
        for i in 0..n {
            let base = (done + i) * channels;
            data[base] = T::from_sample(engine.l[i]);
            if channels > 1 {
                data[base + 1] = T::from_sample(engine.r[i]);
            }
            for c in 2..channels {
                data[base + c] = T::from_sample(0.0);
            }
        }
        done += n;
    }
}

/// Open the default output device and move the engine into its callback.
/// On failure the engine comes back so the caller can run it silently.
///
/// The device's own sample rate is adopted (written to `want_sr`) and the
/// synth retuned to it. `f32`, `i16` and `u16` output formats are
/// supported; anything else is treated as a failure.
///
/// Ownership: the engine sits in an `Arc<Mutex<Option<Engine>>>` shared by
/// the callback and this function. The callback uses `try_lock`, so it can
/// never block on the audio thread: the lock is contended only during
/// start-up, when a failed `play()` takes the engine back out, and an
/// uncontended `try_lock` costs a few nanoseconds. If the callback does
/// find the lock taken it outputs one buffer of silence.
// The engine is handed back on failure on purpose, so the caller can run it silently.
#[allow(clippy::result_large_err)]
fn start_audio(mut engine: Engine, want_sr: &mut f32) -> Result<cpal::Stream, Engine> {
    let host = cpal::default_host();
    let Some(device) = host.default_output_device() else {
        return Err(engine);
    };
    let config = match device.default_output_config() {
        Ok(c) => c,
        Err(e) => {
            log::error!("audio: no default config: {e}");
            return Err(engine);
        }
    };
    *want_sr = config.sample_rate().0 as f32;
    let channels = config.channels() as usize;
    log::info!(
        "audio: {} @ {} Hz, {} ch, {:?}",
        device.name().unwrap_or_default(),
        config.sample_rate().0,
        channels,
        config.sample_format()
    );
    engine.synth.set_sample_rate(*want_sr);
    let stream_config: cpal::StreamConfig = config.clone().into();
    let err = |e| log::error!("audio stream error: {e}");
    // The callback shares the engine through a mutex that is only ever
    // contended at startup (uncontended lock: a few nanoseconds); if cpal
    // refuses the stream, the engine is taken back out and run silently.
    let cell = Arc::new(Mutex::new(Some(engine)));
    macro_rules! callback {
        ($t:ty) => {{
            let cell = cell.clone();
            move |d: &mut [$t], _: &cpal::OutputCallbackInfo| {
                if let Ok(mut g) = cell.try_lock() {
                    if let Some(en) = g.as_mut() {
                        write_interleaved(en, d, channels);
                        return;
                    }
                }
                d.iter_mut().for_each(|s| *s = <$t>::from_sample(0.0f32));
            }
        }};
    }
    let built = match config.sample_format() {
        cpal::SampleFormat::F32 => {
            device.build_output_stream(&stream_config, callback!(f32), err, None)
        }
        cpal::SampleFormat::I16 => {
            device.build_output_stream(&stream_config, callback!(i16), err, None)
        }
        cpal::SampleFormat::U16 => {
            device.build_output_stream(&stream_config, callback!(u16), err, None)
        }
        other => {
            log::error!("audio: unsupported sample format {other:?}");
            return Err(cell.lock().unwrap().take().expect("engine"));
        }
    };
    let give_back = || cell.lock().unwrap().take().expect("engine");
    match built {
        Ok(s) => match s.play() {
            Ok(()) => Ok(s),
            Err(e) => {
                log::error!("audio: could not start the output stream: {e}");
                drop(s);
                Err(give_back())
            }
        },
        Err(e) => {
            log::error!("audio: could not open the output stream: {e}");
            Err(give_back())
        }
    }
}

/// No audio device: keep the synth running on a paced thread so the UI
/// still shows everything. Renders 256-sample blocks at wall-clock rate;
/// if the thread falls more than 200 ms behind (a suspended laptop) it
/// re-syncs instead of racing to catch up.
fn silent_thread(mut engine: Engine, sr: f32) {
    let block = 256usize;
    let dur = Duration::from_secs_f64(block as f64 / sr as f64);
    let mut next = Instant::now();
    loop {
        engine.render(block);
        next += dur;
        let now = Instant::now();
        if next > now {
            thread::sleep(next - now);
        } else if now - next > Duration::from_millis(200) {
            next = now;
        }
    }
}

/// Open `url` with the platform's default browser.
fn open_browser(url: &str) {
    #[cfg(target_os = "windows")]
    let r = std::process::Command::new("cmd")
        .args(["/C", "start", "", url])
        .spawn();
    #[cfg(target_os = "macos")]
    let r = std::process::Command::new("open").arg(url).spawn();
    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    let r = std::process::Command::new("xdg-open").arg(url).spawn();
    if let Err(e) = r {
        log::warn!("could not open browser: {e}");
    }
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    // Command line (see the module docs).
    let mut port: Option<u16> = None;
    let mut open = false;
    let mut silent = false;
    let mut dir: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--port" | "-p" => port = args.next().and_then(|v| v.parse().ok()),
            "--open" | "-o" => open = true,
            "--silent" => silent = true,
            "--dir" | "-d" => dir = args.next().map(PathBuf::from),
            "-h" | "--help" => {
                println!("noob-wave-standalone [--port N] [--open] [--dir path] [--silent]");
                return;
            }
            other => log::warn!("ignoring argument {other}"),
        }
    }
    // The page: `web/dist` next to this crate unless `--dir` says otherwise.
    // `built` only decides whether to print the build hint below.
    let web = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/web"));
    let dist = web.join("dist");
    let (dir, built) = match dir {
        Some(d) => (d, true),
        None => (dist.clone(), dist.join("index.html").is_file()),
    };
    let dir = dir.canonicalize().unwrap_or(dir);

    // Bridge and engine. The synth builds its wavetables here, on the main
    // thread, before anything real-time starts.
    let mut sr = 48_000.0f32;
    let (bridge, ix) = dsp::build_bridge("noob-wave", sr);
    let audio = bridge.take_audio().expect("audio handle");
    let blocks = Arc::new(AtomicU64::new(0));
    let synth = Synth::new(sr);
    let engine = Engine {
        audio,
        ix,
        synth,
        telemetry: Telemetry::new(),
        l: vec![0.0; MAX_BLOCK],
        r: vec![0.0; MAX_BLOCK],
        settings: Settings::default(),
        blocks: blocks.clone(),
    };

    // Real audio if possible, otherwise the paced silent thread.
    let (stream, leftover) = if silent {
        (None, Some(engine))
    } else {
        match start_audio(engine, &mut sr) {
            Ok(s) => (Some(s), None),
            Err(e) => (None, Some(e)),
        }
    };
    if let Some(engine) = leftover {
        if !silent {
            log::warn!("no audio output device; running silently (use the UI, no sound)");
        }
        thread::Builder::new()
            .name("fake-audio".into())
            .spawn(move || silent_thread(engine, sr))
            .expect("spawn audio thread");
    }

    // User presets the page keeps in `client.store` persist in a file next
    // to the discovery records (the plug-in keeps them in its host state).
    let store = FileStore::attach(&bridge, FileStore::default_path("noob-wave"));

    // `--port N` insists on that port; otherwise start at 4243 and walk up,
    // so a second copy (or another noob-vst-webgui-framework app) does not collide.
    let cfg = match port {
        Some(p) => ServerConfig::default().port(p),
        None => ServerConfig::default().prefer_port(4243),
    };
    let server =
        noob_vst_webgui_framework::serve(&bridge, cfg.assets_dir(&dir)).expect("start server");
    println!();
    println!("  noob-wave standalone {}", server.url());
    println!("  websocket            {}", server.ws_url());
    println!("  assets               {}", dir.display());
    println!("  ui store             {}", store.path().display());
    println!("  instances            node tools/instances.mjs");
    println!(
        "  audio                {}",
        if stream.is_some() {
            format!("{sr} Hz, default output device")
        } else {
            "silent".into()
        }
    );
    if !built {
        println!();
        println!("  web/dist not found. Either build the SPA once:");
        println!("      cd web && npm install && npm run build");
        println!("  or develop with hot reload (proxies /ws to this server):");
        println!(
            "      cd web && NOOB_VST_WEBGUI_FRAMEWORK_PORT={} npm run dev",
            server.port()
        );
    }
    println!();
    if open {
        open_browser(&server.url());
    }

    host_loop(&bridge, &server, &store, blocks, sr);
}

/// The main-thread loop, ticking every 5 ms: count page edits (a real host
/// would forward them as parameter gestures; here the bridge already holds
/// the value), handle the page's `reset` message (every parameter back to
/// its default) and ignore `resize` (meaningful only inside a plug-in
/// window), send a `status` message once a second (clients, blocks, edits,
/// dropped UI changes, sample rate), and write the UI store to disk when a
/// key changed. Never returns.
fn host_loop(
    bridge: &NoobVstWebguiFramework,
    server: &noob_vst_webgui_framework::ServerHandle,
    store: &FileStore,
    blocks: Arc<AtomicU64>,
    sr: f32,
) {
    let mut last_status = Instant::now();
    let mut edits = 0u64;
    loop {
        bridge.drain_edits(|_| edits += 1);
        while let Some(m) = bridge.poll_message() {
            match m.topic.as_str() {
                "reset" => {
                    for i in 0..bridge.param_count() {
                        let d = bridge.spec(i).map(|s| s.default).unwrap_or(0.0);
                        bridge.set_param(i, d);
                    }
                }
                "resize" => {}
                other => log::info!("message from client {}: {other} {}", m.client, m.data),
            }
        }
        if last_status.elapsed() >= Duration::from_secs(1) {
            last_status = Instant::now();
            bridge.send_json(
                "status",
                json!({
                    "clients": server.client_count(),
                    "blocks": blocks.load(Ordering::Relaxed),
                    "edits": edits,
                    "dropped": bridge.dropped_ui_changes(),
                    "sample_rate": sr,
                }),
            );
        }
        if let Err(e) = store.flush() {
            log::warn!("could not save the UI store: {e}");
        }
        thread::sleep(Duration::from_millis(5));
    }
}
