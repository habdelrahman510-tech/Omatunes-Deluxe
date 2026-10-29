//! Stereo parametric equalizer.
//!
//! Ten ISO bands implemented as RBJ peaking biquads (Direct Form II
//! transposed), applied inside the cpal output callback. When the equalizer is
//! disabled — or enabled but flat — [`EqProcessor::process_in_place`] returns
//! without touching the buffer, so bypass is bit-exact.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

pub const BAND_COUNT: usize = 10;

/// Centre frequencies, one per band (roughly one octave apart).
pub const BAND_FREQS: [f32; BAND_COUNT] = [
    31.5, 63.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0,
];

/// One octave of bandwidth per band.
pub const BAND_Q: f32 = 1.414;

pub const GAIN_MIN: f32 = -12.0;
pub const GAIN_MAX: f32 = 12.0;
pub const PREAMP_MIN: f32 = -12.0;
pub const PREAMP_MAX: f32 = 0.0;

/// How long a gain change takes to reach its target.
const RAMP_MS: f32 = 20.0;
/// Frames between gain-ramp steps / coefficient rebuilds.
const RAMP_BLOCK: usize = 64;

fn default_auto_preamp() -> bool {
    true
}

fn default_bands() -> [f32; BAND_COUNT] {
    [0.0; BAND_COUNT]
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct EqSettings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub preamp_db: f32,
    #[serde(default = "default_bands")]
    pub bands_db: [f32; BAND_COUNT],
    #[serde(default = "default_auto_preamp")]
    pub auto_preamp: bool,
}

impl Default for EqSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            preamp_db: 0.0,
            bands_db: [0.0; BAND_COUNT],
            auto_preamp: true,
        }
    }
}

impl EqSettings {
    /// Largest positive band gain, floored at 0 dB.
    pub fn max_boost(&self) -> f32 {
        self.bands_db
            .iter()
            .copied()
            .fold(f32::NEG_INFINITY, f32::max)
            .max(0.0)
    }

    /// Preamp actually applied, after the automatic headroom trim.
    pub fn effective_preamp(&self) -> f32 {
        if self.auto_preamp {
            self.preamp_db - self.max_boost()
        } else {
            self.preamp_db
        }
    }

    /// True when the equalizer would not alter the signal at all.
    pub fn is_flat(&self) -> bool {
        self.bands_db.iter().all(|g| g.abs() < 0.001)
            && self.effective_preamp().abs() < 0.001
    }

    /// True when audio should actually be filtered.
    pub fn is_active(&self) -> bool {
        self.enabled && !self.is_flat()
    }
}

// ── Shared settings ───────────────────────────────────────────────────────────

struct EqShared {
    settings: Mutex<EqSettings>,
    generation: AtomicU64,
}

/// Cheap, cloneable handle used by the UI thread to publish new settings.
#[derive(Clone)]
pub struct EqHandle {
    shared: Arc<EqShared>,
}

impl EqHandle {
    pub fn new(settings: EqSettings) -> Self {
        Self {
            shared: Arc::new(EqShared {
                settings: Mutex::new(settings),
                generation: AtomicU64::new(1),
            }),
        }
    }

    pub fn settings(&self) -> EqSettings {
        *self.shared.settings.lock().unwrap()
    }

    /// Publish new settings; the audio callback picks them up on its next block.
    pub fn update(&self, settings: EqSettings) {
        *self.shared.settings.lock().unwrap() = settings;
        self.shared.generation.fetch_add(1, Ordering::SeqCst);
    }

    fn generation(&self) -> u64 {
        self.shared.generation.load(Ordering::SeqCst)
    }
}

// ── Biquad ────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy)]
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: [f32; 2],
    z2: [f32; 2],
}

impl Biquad {
    const IDENTITY: Biquad = Biquad {
        b0: 1.0,
        b1: 0.0,
        b2: 0.0,
        a1: 0.0,
        a2: 0.0,
        z1: [0.0; 2],
        z2: [0.0; 2],
    };

    fn reset(&mut self) {
        self.z1 = [0.0; 2];
        self.z2 = [0.0; 2];
    }

    /// RBJ peaking EQ coefficients, normalised by a0.
    fn set_peaking(&mut self, sample_rate: f32, freq: f32, q: f32, gain_db: f32) {
        let amp = 10f32.powf(gain_db / 40.0);
        let w0 = 2.0 * std::f32::consts::PI * freq / sample_rate;
        let cos_w0 = w0.cos();
        let alpha = w0.sin() / (2.0 * q);

        let a0 = 1.0 + alpha / amp;
        self.b0 = (1.0 + alpha * amp) / a0;
        self.b1 = (-2.0 * cos_w0) / a0;
        self.b2 = (1.0 - alpha * amp) / a0;
        self.a1 = (-2.0 * cos_w0) / a0;
        self.a2 = (1.0 - alpha / amp) / a0;
    }

    #[inline]
    fn process(&mut self, channel: usize, x: f32) -> f32 {
        let y = self.b0 * x + self.z1[channel];
        self.z1[channel] = flush(self.b1 * x - self.a1 * y + self.z2[channel]);
        self.z2[channel] = flush(self.b2 * x - self.a2 * y);
        y
    }
}

#[inline]
fn flush(v: f32) -> f32 {
    if v.abs() < 1e-20 {
        0.0
    } else {
        v
    }
}

/// Unity below the threshold, asymptotic to ±1 above it.
#[inline]
fn soft_clip(x: f32) -> f32 {
    const T: f32 = 0.95;
    if x.abs() <= T {
        return x;
    }
    let sign = x.signum();
    let over = (x.abs() - T) / (1.0 - T);
    sign * (T + (1.0 - T) * over.tanh())
}

// ── Processor (audio thread) ──────────────────────────────────────────────────

pub struct EqProcessor {
    handle: EqHandle,
    generation: u64,
    sample_rate: f32,
    bands: [Biquad; BAND_COUNT],
    current_db: [f32; BAND_COUNT],
    target_db: [f32; BAND_COUNT],
    preamp_cur: f32,
    preamp_target: f32,
    active: bool,
    /// Dry/wet crossfade position, so toggling bypass never clicks.
    wet: f32,
    /// Fraction of the remaining distance covered per ramp block.
    ramp_step: f32,
}

impl EqProcessor {
    pub fn new(handle: EqHandle, sample_rate: f32) -> Self {
        let settings = handle.settings();
        let active = settings.is_active();
        let mut processor = Self {
            generation: handle.generation(),
            handle,
            sample_rate,
            bands: [Biquad::IDENTITY; BAND_COUNT],
            current_db: settings.bands_db,
            target_db: settings.bands_db,
            preamp_cur: settings.effective_preamp(),
            preamp_target: settings.effective_preamp(),
            active,
            wet: if active { 1.0 } else { 0.0 },
            ramp_step: (RAMP_BLOCK as f32) / ((RAMP_MS / 1000.0) * sample_rate),
        };
        processor.rebuild_coefficients();
        processor
    }

    fn rebuild_coefficients(&mut self) {
        for (i, band) in self.bands.iter_mut().enumerate() {
            band.set_peaking(self.sample_rate, BAND_FREQS[i], BAND_Q, self.current_db[i]);
        }
    }

    fn pull_settings(&mut self) {
        let settings = self.handle.settings();
        self.target_db = settings.bands_db;
        self.preamp_target = settings.effective_preamp();
        self.active = settings.is_active();
        self.settle_if_bypassed();
    }

    /// Once the crossfade has fully reached dry, park the filters so a later
    /// enable starts from a clean state.
    fn settle_if_bypassed(&mut self) {
        if self.active || self.wet > 0.0 {
            return;
        }
        self.current_db = self.target_db;
        self.preamp_cur = self.preamp_target;
        for band in self.bands.iter_mut() {
            band.reset();
        }
        self.rebuild_coefficients();
    }

    fn advance_ramp(&mut self) {
        let mut changed = false;
        for i in 0..BAND_COUNT {
            let delta = self.target_db[i] - self.current_db[i];
            if delta.abs() <= 1e-4 {
                continue;
            }
            self.current_db[i] += delta * self.ramp_step;
            if (self.target_db[i] - self.current_db[i]).abs() < 0.01 {
                self.current_db[i] = self.target_db[i];
            }
            changed = true;
        }
        if changed {
            self.rebuild_coefficients();
        }

        let delta = self.preamp_target - self.preamp_cur;
        if delta.abs() > 1e-4 {
            self.preamp_cur += delta * self.ramp_step;
        }

        let target_wet = if self.active { 1.0 } else { 0.0 };
        let delta = target_wet - self.wet;
        if delta.abs() > 1e-4 {
            self.wet = (self.wet + delta * self.ramp_step).clamp(0.0, 1.0);
        } else {
            self.wet = target_wet;
        }

        self.settle_if_bypassed();
    }

    /// Filter a stereo interleaved buffer in place.
    ///
    /// Returns immediately — leaving `buffer` untouched — when the equalizer is
    /// off or flat and the bypass crossfade has finished, which is what makes
    /// bypass bit-exact.
    pub fn process_in_place(&mut self, buffer: &mut [f32]) {
        let generation = self.handle.generation();
        if generation != self.generation {
            self.generation = generation;
            self.pull_settings();
        }
        if !self.active && self.wet <= 0.0 {
            return;
        }

        let frames = buffer.len() / 2;
        let mut frame = 0;
        while frame < frames {
            let end = (frame + RAMP_BLOCK).min(frames);
            self.advance_ramp();
            let preamp = 10f32.powf(self.preamp_cur / 20.0);
            let wet = self.wet;
            let dry = 1.0 - wet;

            for i in frame..end {
                let dry_left = buffer[i * 2];
                let dry_right = buffer[i * 2 + 1];

                let mut left = dry_left;
                let mut right = dry_right;
                for band in self.bands.iter_mut() {
                    left = band.process(0, left);
                    right = band.process(1, right);
                }

                buffer[i * 2] = soft_clip(left * preamp) * wet + dry_left * dry;
                buffer[i * 2 + 1] = soft_clip(right * preamp) * wet + dry_right * dry;
            }
            frame = end;
        }
    }
}

// ── Response curve (UI thread) ────────────────────────────────────────────────

/// Magnitude response of the whole chain at `freq`, in dB.
pub fn response_db(settings: &EqSettings, freq: f32, sample_rate: f32) -> f32 {
    let mut db = settings.effective_preamp();

    let w = 2.0 * std::f32::consts::PI * freq / sample_rate;
    let (cos_w, sin_w) = (w.cos(), w.sin());
    let (cos_2w, sin_2w) = ((2.0 * w).cos(), (2.0 * w).sin());

    for (i, gain_db) in settings.bands_db.iter().enumerate() {
        if gain_db.abs() < 0.001 {
            continue;
        }

        let amp = 10f32.powf(gain_db / 40.0);
        let w0 = 2.0 * std::f32::consts::PI * BAND_FREQS[i] / sample_rate;
        let cos_w0 = w0.cos();
        let alpha = w0.sin() / (2.0 * BAND_Q);
        let a0 = 1.0 + alpha / amp;

        let b0 = (1.0 + alpha * amp) / a0;
        let b1 = (-2.0 * cos_w0) / a0;
        let b2 = (1.0 - alpha * amp) / a0;
        let a1 = (-2.0 * cos_w0) / a0;
        let a2 = (1.0 - alpha / amp) / a0;

        let num_re = b0 + b1 * cos_w + b2 * cos_2w;
        let num_im = -(b1 * sin_w + b2 * sin_2w);
        let den_re = 1.0 + a1 * cos_w + a2 * cos_2w;
        let den_im = -(a1 * sin_w + a2 * sin_2w);

        let num = (num_re * num_re + num_im * num_im).sqrt();
        let den = (den_re * den_re + den_im * den_im).sqrt().max(1e-9);

        db += 20.0 * (num / den).max(1e-9).log10();
    }

    db
}

// ── Presets ───────────────────────────────────────────────────────────────────

pub struct BuiltinPreset {
    pub name: &'static str,
    pub preamp_db: f32,
    pub bands_db: [f32; BAND_COUNT],
}

pub const BUILTIN_PRESETS: [BuiltinPreset; 16] = [
    BuiltinPreset {
        name: "Flat",
        preamp_db: 0.0,
        bands_db: [0.0; BAND_COUNT],
    },
    BuiltinPreset {
        name: "Loudness",
        preamp_db: 0.0,
        bands_db: [6.0, 4.0, 1.0, 0.0, -2.0, -1.0, 0.0, 2.0, 4.0, 5.0],
    },
    BuiltinPreset {
        name: "Bass boost",
        preamp_db: 0.0,
        bands_db: [6.0, 5.0, 4.0, 2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
    },
    BuiltinPreset {
        name: "Bass cut",
        preamp_db: 0.0,
        bands_db: [-6.0, -5.0, -3.0, -1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
    },
    BuiltinPreset {
        name: "Treble boost",
        preamp_db: 0.0,
        bands_db: [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 2.0, 3.0, 4.0, 5.0],
    },
    BuiltinPreset {
        name: "Treble cut",
        preamp_db: 0.0,
        bands_db: [0.0, 0.0, 0.0, 0.0, 0.0, -1.0, -2.0, -3.0, -4.0, -5.0],
    },
    BuiltinPreset {
        name: "Vocal",
        preamp_db: 0.0,
        bands_db: [-2.0, -1.0, 0.0, 2.0, 3.5, 3.0, 1.5, 0.0, -1.0, -2.0],
    },
    BuiltinPreset {
        name: "Podcast",
        preamp_db: 0.0,
        bands_db: [-6.0, -4.0, -1.0, 2.0, 3.0, 3.0, 2.0, 0.0, -1.0, -3.0],
    },
    BuiltinPreset {
        name: "Rock",
        preamp_db: 0.0,
        bands_db: [4.0, 3.0, 1.0, -1.0, -1.5, 0.0, 1.5, 2.5, 3.0, 3.5],
    },
    BuiltinPreset {
        name: "Pop",
        preamp_db: 0.0,
        bands_db: [-1.0, 0.0, 1.0, 3.0, 4.0, 3.0, 1.0, -1.0, -1.0, 0.0],
    },
    BuiltinPreset {
        name: "Jazz",
        preamp_db: 0.0,
        bands_db: [3.0, 2.0, 1.0, 2.0, -1.0, -1.0, 0.0, 1.0, 2.0, 3.0],
    },
    BuiltinPreset {
        name: "Classical",
        preamp_db: 0.0,
        bands_db: [3.0, 2.0, 1.0, 0.0, -1.0, -1.0, 0.0, 2.0, 3.0, 4.0],
    },
    BuiltinPreset {
        name: "Acoustic",
        preamp_db: 0.0,
        bands_db: [3.0, 2.0, 1.0, 0.0, 1.0, 2.0, 2.0, 1.0, 1.0, 0.0],
    },
    BuiltinPreset {
        name: "Electronic",
        preamp_db: 0.0,
        bands_db: [5.0, 4.0, 1.0, 0.0, -2.0, 1.0, 0.0, 1.0, 4.0, 5.0],
    },
    BuiltinPreset {
        name: "Hip-Hop",
        preamp_db: 0.0,
        bands_db: [7.0, 6.0, 3.0, 0.0, -1.0, -1.0, 1.0, 2.0, 3.0, 4.0],
    },
    BuiltinPreset {
        name: "Night",
        preamp_db: 0.0,
        bands_db: [-8.0, -5.0, -1.0, 1.0, 2.0, 2.0, 1.0, -1.0, -3.0, -5.0],
    },
];

pub const CUSTOM_PRESET: &str = "Custom";

/// Name of the built-in preset matching `settings`, or `"Custom"`.
pub fn preset_name(settings: &EqSettings) -> &'static str {
    for preset in BUILTIN_PRESETS.iter() {
        let bands_match = preset
            .bands_db
            .iter()
            .zip(settings.bands_db.iter())
            .all(|(a, b)| (a - b).abs() < 0.01);
        if bands_match && (preset.preamp_db - settings.preamp_db).abs() < 0.01 {
            return preset.name;
        }
    }
    CUSTOM_PRESET
}

/// Apply a built-in preset by name. Returns false if the name is unknown.
pub fn apply_preset(settings: &mut EqSettings, name: &str) -> bool {
    for preset in BUILTIN_PRESETS.iter() {
        if preset.name == name {
            settings.bands_db = preset.bands_db;
            settings.preamp_db = preset.preamp_db;
            return true;
        }
    }
    false
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const FS: f32 = 48_000.0;

    fn flat() -> EqSettings {
        EqSettings::default()
    }

    #[test]
    fn disabled_is_bit_exact() {
        let handle = EqHandle::new(EqSettings {
            enabled: false,
            ..flat()
        });
        let mut processor = EqProcessor::new(handle, FS);
        let mut buffer = vec![0.5f32; 512];
        let original = buffer.clone();
        processor.process_in_place(&mut buffer);
        assert_eq!(buffer, original);
    }

    #[test]
    fn enabled_but_flat_is_bit_exact() {
        let handle = EqHandle::new(EqSettings {
            enabled: true,
            ..flat()
        });
        let mut processor = EqProcessor::new(handle, FS);
        let mut buffer = vec![0.25f32; 512];
        let original = buffer.clone();
        processor.process_in_place(&mut buffer);
        assert_eq!(buffer, original);
    }

    #[test]
    fn boost_measures_correctly() {
        let mut settings = flat();
        settings.auto_preamp = false; // measure the raw band, not the trimmed chain
        settings.bands_db[5] = 12.0; // 1 kHz
        let at_1k = response_db(&settings, 1000.0, FS);
        assert!((at_1k - 12.0).abs() < 0.5, "1 kHz boost was {at_1k}");

        let at_100 = response_db(&settings, 100.0, FS);
        assert!(at_100.abs() < 0.5, "100 Hz changed by {at_100}");
    }

    #[test]
    fn auto_preamp_offsets_boost() {
        let mut settings = flat();
        settings.enabled = true;
        settings.bands_db[5] = 6.0; // 1 kHz
        assert!((settings.effective_preamp() + 6.0).abs() < 0.001);

        // The +6 dB peak is cancelled by the -6 dB preamp, so 1 kHz comes back flat.
        assert!(response_db(&settings, 1000.0, FS).abs() < 0.01);
    }

    #[test]
    fn processor_actually_boosts() {
        let mut settings = flat();
        settings.enabled = true;
        settings.auto_preamp = false;
        settings.bands_db[5] = 12.0; // 1 kHz, +12 dB

        let handle = EqHandle::new(settings);
        let mut processor = EqProcessor::new(handle, FS);

        // One second of a 1 kHz sine, interleaved stereo.
        let frames = FS as usize;
        let mut buffer = Vec::with_capacity(frames * 2);
        for n in 0..frames {
            let sample = (2.0 * std::f32::consts::PI * 1000.0 * n as f32 / FS).sin() * 0.05;
            buffer.push(sample);
            buffer.push(sample);
        }

        let rms_before = rms(&buffer);
        processor.process_in_place(&mut buffer);
        let rms_after = rms(&buffer);

        // Skip the filter's settling tail.
        let ratio = 20.0 * (rms_after / rms_before).log10();
        assert!(
            (ratio - 12.0).abs() < 1.0,
            "expected ~+12 dB, measured {ratio} dB"
        );
    }

    #[test]
    fn bypass_fades_then_is_bit_exact() {
        let mut settings = flat();
        settings.enabled = true;
        settings.auto_preamp = false;
        settings.bands_db[5] = 12.0; // 1 kHz, +12 dB

        let handle = EqHandle::new(settings);
        let mut processor = EqProcessor::new(handle.clone(), FS);

        // Warm the filter state up so a hard switch would visibly click.
        let mut warm = vec![0.1f32; 8192];
        processor.process_in_place(&mut warm);

        // Switch the equalizer off mid-stream.
        settings.enabled = false;
        handle.update(settings);

        // Give the crossfade time to finish.
        let mut fade = vec![0.1f32; (FS as usize) * 2];
        processor.process_in_place(&mut fade);

        // Now the buffer must pass through untouched.
        let mut buffer = vec![0.37f32; 1024];
        let original = buffer.clone();
        processor.process_in_place(&mut buffer);
        assert_eq!(buffer, original, "bypass was not bit-exact after the fade");
    }

    fn rms(buffer: &[f32]) -> f32 {
        // Measure the second half only, past any start-up transient.
        let tail = &buffer[buffer.len() / 2..];
        (tail.iter().map(|s| s * s).sum::<f32>() / tail.len() as f32).sqrt()
    }

    #[test]
    fn preset_round_trip() {
        let mut settings = flat();
        assert!(apply_preset(&mut settings, "Bass boost"));
        assert_eq!(preset_name(&settings), "Bass boost");
        settings.bands_db[3] += 1.0;
        assert_eq!(preset_name(&settings), CUSTOM_PRESET);
    }

    #[test]
    fn every_preset_round_trips() {
        for preset in BUILTIN_PRESETS.iter() {
            let mut settings = flat();
            assert!(apply_preset(&mut settings, preset.name));
            assert_eq!(
                preset_name(&settings),
                preset.name,
                "preset {} did not round-trip",
                preset.name
            );
        }
    }
}
