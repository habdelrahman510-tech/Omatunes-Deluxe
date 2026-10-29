# OmaTunes Equalizer — implementation brief

**Status:** decided (§13). No code written yet. Ready to execute.
**Target repo:** `~/Projects/omatunes`
**Written for:** an implementing agent (or a later session of me) to execute end-to-end.

---

## 0. How to use this document

> You are implementing an equalizer inside **OmaTunes**, a Rust/iced music player at
> `~/Projects/omatunes`. Read §1 (facts about the existing audio path), obey §3 (hard
> constraints), then pick exactly one option from §5 using the recommendation in §7 unless
> the operator overrides it. Implement the plan in §8 (or §9 for the Easy Effects route),
> and do not declare done until every item in §10 passes. Do not touch the lyrics, text
> shaping, or RTL code — it was recently finished and signed off.

---

## 1. Current state of the audio path (verified, not assumed)

| Fact | Where |
|---|---|
| Decoding: **Symphonia 0.5** → f32 PCM ring buffer | `src/audio/player.rs` |
| Output: **cpal 0.15**, default output device, **fixed 48 000 Hz / 2 ch**, `BufferSize::Default` | `src/audio/player.rs:20-21, 110-128` |
| Per-sample pipeline: `pcm.pop_front() * volume` → visualizer ring buffer | `fill_output()`, `src/audio/player.rs:278` |
| Visualizer taps the **post-volume** signal | `src/audio/player.rs:278-300`, `src/audio/spectrum.rs` |
| Volume already shells out to `pactl list sink-inputs` | `src/audio/player.rs:23` |
| Right panel has tabs **Visualizer / Statistics / Lyrics** | `src/app.rs:57`, rendered at `src/ui/views/player.rs:412+` |
| Right panel is **hidden below ~1429 px** window width | layout guard in `src/ui/views/player.rs` |
| `canvas` feature is enabled and already used (pie chart) | `Cargo.toml:13`, `src/ui/views/charts.rs` |
| iced 0.13 **has no vertical slider** (checked `iced_widget-0.13.4/src/slider.rs`) | — |
| Persistence: `~/.config/omatunes/db.json` (`right_panel_tab`, `last_queue_paths`, …) | `src/app.rs:1060` |
| Audio server: **PipeWire 1.6.9**, PulseAudio-compat 15.0.0, default sink = `alsa_output…HiFi__Headphones__sink` | `pactl info` |
| **Easy Effects is NOT installed** | `command -v easyeffects` → empty |

Consequences that matter for the design:

- OmaTunes owns its own PCM stream. Anything we put **inside** the process is inherently
  per-application and needs no system routing at all.
- Anything we put **outside** the process (Easy Effects, PipeWire filter-chain) affects
  every stream on the machine unless we actively scope it, and Easy Effects' only scoping
  mechanism is an *exclude* list.
- The visualizer reads the same buffer, so EQ placement decides whether the spectrum shows
  pre- or post-EQ audio.

---

## 2. The goal, stated as user outcomes

1. An equalizer that feels like **part of OmaTunes**, not a second app bolted on.
2. Adjustable bands with a visible response curve, and the change audible immediately.
3. Settings survive restart.
4. The player must remain bit-perfect when the EQ is off or flat.
5. No dropouts, no clicks, no added playback latency.
6. Works on this machine **as it is** (no Easy Effects installed) — or fails loudly and
   gracefully, never silently.

---

## 3. Hard constraints

- **Do not touch** `src/ui/txt.rs`, the lyrics branches in `src/ui/views/player.rs`, or the
  RTL/LRM logic. Recently finished, signed off.
- Keep the existing `symphonia → cpal` pipeline; no rewrite to GStreamer/PipeWire-native.
- No new heavyweight runtime dependency (no Qt, no Python, no gstreamer).
- The audio callback must not allocate, lock per sample, or block.
- EQ **off** must be bit-exact passthrough, not "quietly multiplied by 1.0".
- Every new user-visible string is English (no UI translation work).
- Respect the existing iced 0.13 API; no vertical sliders exist, so do not design around them.
- Build must stay `cargo build --release --bin omatunes`; deploy is
  `cp target/release/omatunes ~/.local/bin/omatunes`; rollback is
  `cp ~/.local/bin/omatunes.bak ~/.local/bin/omatunes`.

---

## 4. Prior art — what already exists, and the gap

**Easy Effects automation (control-plane only):**
- Official **Local Server** — line protocol over `$XDG_RUNTIME_DIR/EasyEffectsServer`
  (`load_preset`, `global_bypass`, `set_property`, `get_property`, `quit_app`, …).
  https://wwmm.github.io/easyeffects/user_interface/local_server.html
- **CLI** — `easyeffects -l <preset>`, `-b 1|2`, `-p`, `-a`, `-s input|output`, `-q`, `-r`, `-w`.
  https://github.com/wwmm/easyeffects/wiki/Command-Line-Options
- **Preset switcher gist** (fiso64) — shell/GTK picker that calls `easyeffects -l`.
- **`danielrosehill/Easy-Effects-Manager`**, **`Digitalone1/EasyEffects-Presets`**,
  **`p-chan5/EasyPulse`** — preset management and preset packs.
- A GNOME Shell extension for switching EE presets.

**In-player equalizers (the other tradition):**
- Clementine / Strawberry / Tauon Music Box / Elisa all ship a **built-in 10-band EQ** —
  i.e. the EQ is part of the player's own DSP, exactly like Option B below.
- Historically PulseEffects exposed a per-application model; Easy Effects dropped that in
  favour of "process all, minus an exclude list".

**Rust DSP building blocks:**
- `biquad` crate (DF1/DF2T, RBJ-style `Type::PeakingEQ`) — small, well-tested.
- `fundsp` — has `bell`/`peak` peaking filters and a ready-made equalizer with listeners.
- `biquad_filters` — `PeakingEQFilter` + `Coefficients`.

**The gap:** every existing Easy Effects integration is a *desktop-level preset picker*.
Nobody has put an EQ surface **inside** a player that drives EE's socket, and nobody has
made the player and the effects host feel like one program. That is the actual novelty
here, and it is a UI/integration problem, not a DSP problem.

---

## 5. The options

### Option A — Easy Effects as a remote DSP (control plane in OmaTunes)

**How it works**
1. OmaTunes ensures EE is running (`easyeffects --service-mode --hide-window`, or via the
   socket; if absent, show "Easy Effects is not installed" and disable the panel).
2. OmaTunes writes a dedicated preset `omatunes-eq.json` into
   `$XDG_DATA_HOME/easyeffects/output/` (fallback `~/.config/easyeffects/output/`) whose
   shape is `{"output": {"equalizer": {...}, "blocklist": []}}`.
3. `load_preset:output:omatunes-eq` over the socket.
4. Live edits: `set_property:output:equalizer:0:left:band0Gain:<dB>` and the same for
   `right`. Reads: `get_property:…`. Bypass: `global_bypass:1|0`.
5. The OmaTunes UI is **ours**; EE never shows its window.

**Per-app scoping:** only via EE's *Excluded Apps* list. To EQ OmaTunes alone you must
exclude every other application, and any new app is processed by default. There is no
"only these apps" allowlist.

**Latency:** EE is a PipeWire filter node → one graph quantum (~5–21 ms) plus its own DSP.
Fine for music, unacceptable for live monitoring.

**Pros:** reuses LSP's parametric EQ (1–32 bands, IIR *and* FIR), convolver, limiter,
autogain — for free. Small OmaTunes-side code (~250 lines). Power users keep their presets.

**Cons:** hard runtime dependency on a Qt/Kirigami app that is **not installed here**.
State lives in two places (our db + EE's preset files) and can drift. If EE restarts,
crashes, or updates its socket protocol, playback goes un-EQ'd or silent. "One app" is a
polite fiction: EE is still a separate daemon with its own config directory.

**Effort:** S–M (≈1 day) + install/documentation burden.

---

### Option B — In-app biquad DSP in the cpal callback ⭐

**How it works**
Insert a stereo parametric EQ between the PCM ring buffer and the volume multiply inside
`fill_output()` (`src/audio/player.rs:278`). Ten ISO bands as RBJ peaking biquads,
Direct-Form-II-transposed, per-channel state, plus a preamp and a soft clip. Settings live
in `db.json`; the UI is a new right-panel tab.

**Pros:** no runtime dependency at all; genuinely one program; per-application by
construction; **zero added latency** (IIR is causal, no buffering); works on any output
device; deterministic and unit-testable with FFTs (`rustfft` is already a dependency);
bit-exact bypass is easy to guarantee and easy to prove.

**Cons:** we own the DSP. A 10-band parametric EQ is the ceiling unless we later add
convolver/AutoEQ import. Realtime discipline is required (no allocation, one lock per
buffer, denormal handling).

**Effort:** M (≈1.5–2 days including UI and tests).

---

### Option C — Dedicated PipeWire filter-chain virtual sink

**How it works**
`libpipewire-module-filter-chain` with biquad nodes (or
`libpipewire-module-parametric-equalizer` with an AutoEQ/Squiglink profile) creates a
virtual sink `omatunes.eq.sink`. OmaTunes targets that device by name from cpal; the chain
feeds the hardware sink.

**Pros:** OS-level; professional LSP/LV2 filters; survives app restarts; any app can be
routed in.

**Cons:** OmaTunes becomes responsible for a piece of the user's audio graph — a config
file, a module lifecycle (systemd user unit or spawned `pipewire -c`), and device selection
by name inside cpal. Reloading the chain produces an audible glitch. Debugging spans three
daemons (app → PipeWire → session manager). High blast radius for a music player.

**Effort:** M–L (2–3 days).

---

### Option D — Hybrid: B as the engine, A as an optional bridge ⭐⭐ (recommended)

Ship **B** as the native equalizer. Add a single toggle in the EQ header —
**"Use Easy Effects instead"** — which, when EE is present, hands the same UI over to
Option A (same band layout, same presets, different backend). One data model, two engines,
one interface. If EE is absent the toggle is disabled with a one-line explanation.

**Why:** the operator asked for "binding Easy Effects through the OmaTunes interface making
them one". Option A alone makes OmaTunes a remote control for a program that isn't
installed. Option B alone makes them one but ignores the request. Option D delivers the
integration *and* works today.

**Effort:** M+ (B first, A as a second pass).

---

### Option E — LV2 host in-process (LSP plugins via `lilv`)

**Pros:** pro-grade filters inside the process.
**Cons:** C dependencies, `unsafe`, LV2 worker/threading rules, large effort, and no
capability a 10-band biquad EQ lacks.
**Verdict: reject for v1.** Revisit only if convolution/AutoEQ becomes a requirement.

---

## 6. Comparison

| | A: EE bridge | B: in-app DSP | C: filter-chain sink | D: hybrid |
|---|---|---|---|---|
| Runtime dependency | Easy Effects (absent) | none | PipeWire module | none / optional |
| Affects only OmaTunes | only via exclude-list | **yes, inherently** | yes (if routed) | yes |
| Added latency | ~5–21 ms | **0** | ~5–21 ms | 0 / 5–21 ms |
| CPU | external process | **negligible** | external node | varies |
| UI ownership | ours | ours | ours | ours |
| Bit-exact bypass | no | **yes** | no | yes |
| Effort | S–M | M | M–L | M+ |
| Blast radius | app + EE | **app only** | app + audio graph | app (+EE) |
| Works on this machine today | ✗ | **✓** | ✓ | ✓ |

---

## 7. Recommendation

**Do Option B now; add Option A as the optional bridge in a second pass (Option D).**
Do not do Option C. Do not do Option E.

Rationale, in one line: the only way OmaTunes and an equalizer can genuinely be *one* app is
for the EQ to live in OmaTunes' own signal path — everything else is a remote control with
a daemon attached.

**Decided by the operator:** Option D, in that order — in-app DSP (Option B) first, the Easy
Effects bridge (Option A) as a second pass. Options C and E are closed.

---

## 8. Detailed plan — Option B

### 8.1 Data model and persistence

New file `src/audio/eq.rs`:

```rust
pub const BAND_COUNT: usize = 10;
pub const BAND_FREQS: [f32; BAND_COUNT] =
    [31.5, 63.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0];
pub const BAND_Q: f32 = 1.414; // one octave per band

#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct EqSettings {
    pub enabled: bool,
    pub preamp_db: f32,          // applied before the bands
    pub bands_db: [f32; BAND_COUNT],
    pub auto_preamp: bool,       // preamp = -max(0, max(bands_db))
}

impl Default for EqSettings {
    fn default() -> Self { Self { enabled: false, preamp_db: 0.0,
                                  bands_db: [0.0; BAND_COUNT], auto_preamp: true } }
}
```

Persist as a new field `equalizer: EqSettings` in the same struct that already persists
`right_panel_tab` and `last_queue_paths` (`~/.config/omatunes/db.json`). Add
`#[serde(default)]` so old configs load unchanged.

Named presets go in `~/.config/omatunes/eq_presets.json`:

```json
{ "version": 1,
  "presets": [ { "name": "Bass boost", "preamp_db": -6.0,
                 "bands_db": [6,5,4,2,0,0,0,0,0,0] } ] }
```

Built-in presets (not written to disk unless the user saves them): Flat, Bass boost, Vocal,
Rock, Podcast.

### 8.2 DSP core

RBJ peaking EQ, per band, per channel:

```
A     = 10^(gain_db / 40)
w0    = 2π · f0 / fs
alpha = sin(w0) / (2Q)

b0 = 1 + alpha·A     b1 = -2·cos(w0)     b2 = 1 - alpha·A
a0 = 1 + alpha/A     a1 = -2·cos(w0)     a2 = 1 - alpha/A
```

Normalise by `a0`; store `a1, a2` as negative-denominator form for DF2T.

- `fs` is the fixed `OUTPUT_RATE` (48 000), so coefficients are computed once per change.
- State: `z1, z2` per band **per channel** (20 biquads total — trivial CPU).
- **Bypass must short-circuit**: `if !enabled || all_zero { return input }` — no multiply,
  no filter, bit-exact.
- **Gain ramp**: hold `target_db[i]` and `current_db[i]`; move `current` toward `target`
  by `1.0 / (0.020 * fs)` per sample (20 ms full-scale). Recompute that band's
  coefficients when `current` changes by more than 0.01 dB. This kills zipper noise
  without allocating.
- **Denormals**: add `#[inline] fn flush(x: f32) -> f32 { if x.abs() < 1e-20 { 0.0 } else { x } }`
  on the state updates, or enable FTZ where available.
- **Headroom**: after the bands, apply `preamp_db` then a soft clip
  `x / (1.0 + x.abs())`-style knee above 0.9, or simply `x.clamp(-1.0, 1.0)` for v1 —
  state which, and document it.
- **Auto preamp**: when `auto_preamp`, effective preamp is
  `preamp_db - max(0.0, bands_db.iter().cloned().fold(f32::MIN, f32::max))`, so a +9 dB
  boost cannot clip.

Public surface:

```rust
pub struct EqHandle { /* Arc'd shared params + generation counter */ }
impl EqHandle {
    pub fn new(settings: EqSettings) -> Self;
    pub fn update(&self, settings: EqSettings);   // called from the UI thread
    pub fn snapshot(&self) -> EqSettings;
}
pub struct EqProcessor { /* owned by the audio callback */ }
impl EqProcessor {
    pub fn new(handle: EqHandle, sample_rate: f32) -> Self;
    pub fn process_in_place(&mut self, interleaved: &mut [f32]); // stereo
}
```

`EqHandle` is cheap-cloneable and `Send`. `EqProcessor` lives inside the cpal closure and
never allocates after construction.

### 8.3 Wiring into the audio path

In `fill_output()` (`src/audio/player.rs:278`), insert EQ **before** the volume multiply so
EQ gain is independent of the volume slider, and **before** the visualizer tap so the
spectrum reflects what you hear:

```
pcm.pop_front() → EQ (stereo, per-frame) → × volume → output + vis ring buffer
```

Both the `f32` and `I16` stream branches call `fill_output`; keep one code path so the EQ
is applied identically. Change the signature to take `&mut EqProcessor` and have each
closure own its processor.

Update the processor's settings when the UI sends a change: the app already owns a handle
to the audio thread; add `AudioCommand::SetEq(EqSettings)` and forward it to `EqHandle`.

### 8.4 UI

New right-panel tab **Equalizer**, alongside Visualizer / Statistics / Lyrics
(`src/app.rs:57`, tab button at `src/ui/views/player.rs:321`). Add `ICON_EQUALIZER` to
`src/ui/icons.rs`.

Layout (450 px wide, ~470 px tall):

```
┌──────────────────────────────────────────┐
│ [on/off]  preset ▾   preamp ────  -3.0 dB│
│ ┌──────────────────────────────────────┐ │
│ │   response curve (canvas, ~110 px)   │ │
│ └──────────────────────────────────────┘ │
│ 31 Hz  ──────●──────────────  +4.0 dB    │  ← scrollable
│ 63 Hz  ─────●───────────────  +3.0 dB    │     column
│ …                                        │
│ 16 kHz ──────────────●──────  -2.0 dB    │
│                        [Reset] [Save…]   │
└──────────────────────────────────────────┘
```

- iced 0.13 has **no vertical slider** → use horizontal `slider()` per band, one row each,
  inside a `scrollable`. Each row: frequency label, slider (−12…+12 dB, step 0.5), value.
- Response curve: a `canvas::Program` (pattern: `src/ui/views/charts.rs`) drawing a log-x
  grid from 20 Hz to 20 kHz, 0 dB line, and the summed magnitude response of the 10
  biquads. 10 bands × 128 points is cheap; recompute only when settings change.
- Preset picker: `pick_list` over built-ins + user presets.
- Double-click a band to zero it (stretch).
- Disabled state: sliders greyed, curve flat, audio untouched.

Because the right panel is hidden below ~1429 px, add a small **EQ button in the player
bar** that focuses the panel; if the window is too narrow, open the same content in a
dialog (reuse the `settings_dialog`/`playlist_dialog` pattern). *This is a decision point —
see §13.*

### 8.5 Messages

```rust
EqToggled(bool)
EqBandChanged { band: usize, db: f32 }
EqPreampChanged(f32)
EqAutoPreampToggled(bool)
EqPresetSelected(String)
EqPresetSaved(String)
EqReset
```

Each handler: mutate `AppState.equalizer`, push `AudioCommand::SetEq(..)`, mark db dirty.
Do **not** rebuild the audio stream — settings flow through the handle.

### 8.6 Failure and degradation

- EQ disabled or flat → bit-exact passthrough (asserted in tests).
- Missing/corrupt `eq_presets.json` → fall back to built-ins, log once, never panic.
- Old `db.json` without `equalizer` → `serde(default)`, EQ off.
- Audio device change / stream rebuild → construct a fresh `EqProcessor` from the current
  `EqSettings`.

### 8.7 Option A bridge (second pass, only if chosen)

- Detect EE: socket exists at `$XDG_RUNTIME_DIR/EasyEffectsServer`, or `easyeffects` on PATH.
- Write `omatunes-eq.json`, `load_preset:output:omatunes-eq`, then per-band
  `set_property:output:equalizer:0:left:band0Gain:<dB>` (+`right`), debounced ~50 ms.
- Map on/off → `global_bypass:1|0`.
- If EE dies mid-session, detect the socket error and fall back to the in-app engine with a
  toast — never leave the user in silence.

---

## 9. Easy Effects command reference (for Option A)

Socket: `$XDG_RUNTIME_DIR/EasyEffectsServer` (e.g. `/run/user/1000/EasyEffectsServer`),
line-oriented plain text, e.g.

```bash
echo "load_preset:output:omatunes-eq" | socat - UNIX-CONNECT:$XDG_RUNTIME_DIR/EasyEffectsServer
echo "set_property:output:equalizer:0:left:band0Gain:1.5" | socat - UNIX-CONNECT:$XDG_RUNTIME_DIR/EasyEffectsServer
echo "get_property:output:equalizer:0:left:band0Gain"  | socat - UNIX-CONNECT:$XDG_RUNTIME_DIR/EasyEffectsServer
echo "global_bypass:1" | socat - UNIX-CONNECT:$XDG_RUNTIME_DIR/EasyEffectsServer
```

General commands: `show_window`, `hide_window`, `quit_app`, `global_bypass 1|0`,
`get_global_bypass`, `toggle_global_bypass`, `load_preset:<pipeline>:<name>`,
`get_last_loaded_preset:<pipeline>`, `microphone_monitoring`, `audio_sharing`.

Preset file shape (`$XDG_DATA_HOME/easyeffects/output/omatunes-eq.json`):

```json
{ "output": {
    "blocklist": [],
    "equalizer": {
      "bypass": false, "input-gain": 0.0, "output-gain": 0.0,
      "mode": "IIR", "num-bands": 10, "split-channels": false, "balance": 0.0,
      "left":  { "band0": { "frequency": 31.5, "gain": 4.0, "q": 1.41,
                            "type": "Bell", "mode": "RLC (BT)", "slope": "x1",
                            "mute": false, "solo": false } },
      "right": { "band0": { "frequency": 31.5, "gain": 4.0, "q": 1.41,
                            "type": "Bell", "mode": "RLC (BT)", "slope": "x1",
                            "mute": false, "solo": false } }
    }
} }
```

---

## 10. Acceptance criteria (all must pass)

1. **Bit-exact bypass.** With `enabled: false`, a rendered WAV of a reference track is
   byte-identical to the pre-change build's render. (Add a tiny `#[test]`-driven render
   helper if no CLI renderer exists.)
2. **Bit-exact flat.** `enabled: true`, all bands 0 dB, preamp 0 → `max|y − x| < 1e-6`.
3. **Correct gain.** White noise through a +12 dB band at 1 kHz measures **12 ± 0.5 dB** at
   1 kHz and **< 0.5 dB** change at 100 Hz and 10 kHz (FFT via `rustfft`).
4. **No clicks.** A 0 → +12 dB sweep of one band yields no sample-to-sample step > 0.1
   (20 ms ramp test).
5. **No dropouts.** 10 minutes with all bands at +12 dB: no underruns, no allocation in the
   callback, no lock taken per sample (code review + a debug counter).
6. **Zero added latency.** EQ is causal and unbuffered; the visualizer/position relationship
   is unchanged.
7. **CPU.** < 1 % of one core at 48 kHz stereo, 10 bands.
8. **Persistence.** Set a curve, quit, relaunch → identical curve and enabled state.
9. **Graceful absence.** No Easy Effects installed → the app builds, runs, and the in-app EQ
   works; the EE bridge toggle is disabled with an explanation.
10. **No regressions.** Lyrics (synced and plain, Arabic and English) render exactly as
    before; the app builds with the same warning count (±0).

---

## 11. Test plan (proving it without ears)

- `src/audio/eq.rs` unit tests: identity coefficients, analytic vs measured magnitude at
  three frequencies, stability (impulse response decays), denormal flush.
- A `#[cfg(test)]` harness that runs a log sweep through `EqProcessor` and FFTs it with
  `rustfft` (already a dependency) — compare against the closed-form RBJ response.
- Manual: play a bass-heavy track, +9 dB at 63 Hz, confirm no clipping with auto-preamp on.
- Manual: drag a slider fast during playback → no zipper noise, no stutter.
- Manual: toggle off mid-track → no click (bypass is a hard short-circuit, so this must be
  sample-continuous; if it isn't, cross-fade over 5 ms).

---

## 12. Risks and rollback

| Risk | Mitigation |
|---|---|
| Mutex in the audio callback | one lock per buffer, never per sample; or atomics + generation counter |
| Zipper noise | 20 ms gain ramp |
| Clipping from boosts | auto preamp + soft clip |
| Denormals on silence | flush-to-zero on state updates |
| Signature change to `fill_output` breaks the I16 path | single code path, tested on both branches |
| UI regression in the right panel | new tab only; existing tabs untouched |
| Build time (≈6–7 min) | iterate with `cargo check`, release-build once |

Rollback: `cp ~/.local/bin/omatunes.bak ~/.local/bin/omatunes`; EQ is off by default and the
db field is additive, so no migration is needed.

---

## 13. Decisions

**Locked**

1. **Engine** — in-app biquad DSP (Option B) now; Easy Effects bridge (Option A) as a second
   pass, behind a "Use Easy Effects instead" toggle. Options C and E rejected.
2. **Placement** — a new **Equalizer** tab in the right panel, alongside Visualizer /
   Statistics / Lyrics. Accepted consequence: the panel (and therefore the EQ) is
   unreachable when the window is narrower than ~1429 px; the player-bar entry point in
   §8.4 remains a stretch goal, not v1 scope.

**Still open (can be answered during implementation, defaults in brackets)**

3. **Band count** — [fixed 10-band graphic].
4. **Presets** — [built-in five + user presets].
5. **Visualizer** — [post-EQ], so the spectrum shows what you hear.
6. **Scope** — [one global curve].
7. **Easy Effects install** — not required for v1; only relevant when the bridge is built.
