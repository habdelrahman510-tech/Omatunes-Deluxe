//! ReplayGain / EBU R128 loudness normalisation.
//!
//! Two sources of truth, in order:
//!
//! 1. ReplayGain tags written by any other tagger (`REPLAYGAIN_TRACK_GAIN`,
//!    `REPLAYGAIN_TRACK_PEAK`, and the `ALBUM_*` pair). These are free to read.
//! 2. An integrated-loudness measurement of our own, following ITU-R BS.1770-4:
//!    K-weighting, 400 ms gating blocks at 75 % overlap, −70 LUFS absolute
//!    gate and a −10 LU relative gate. Results are cached in
//!    `~/.config/omatunes/loudness.json` so a track is analysed once, ever.
//!
//! Analysis runs on a background thread. The first time an untagged track
//! plays it gets unity gain and is measured for next time; the engine never
//! blocks a playback command on a full-file decode.
//!
//! ReplayGain tags are referenced to −18 LUFS (ReplayGain 2.0 / R128 hybrid),
//! which is the target we normalise to as well.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// Reference loudness for ReplayGain 2.0.
pub const TARGET_LUFS: f64 = -18.0;
/// Ceiling left below 0 dBFS when a peak is known.
const PEAK_CEILING: f64 = 0.98;
/// Anything quieter than this is not worth normalising.
const SILENCE_LUFS: f64 = -70.0;
/// Never boost or cut by more than this.
const MAX_GAIN_DB: f64 = 24.0;

const BLOCK_SECS: f64 = 0.400;
const HOP_SECS: f64 = 0.100;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReplayGainMode {
    /// Tags and loudness ignored; bit-exact output.
    #[default]
    Off,
    /// Per-track normalisation.
    Track,
    /// Album gain when the file carries `REPLAYGAIN_ALBUM_*`, track otherwise.
    Album,
}

/// What we know about one file.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LoudnessEntry {
    /// Integrated loudness in LUFS, measured by us.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lufs: Option<f64>,
    /// True peak (sample peak) in linear units.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peak: Option<f64>,
}

#[derive(Default, Serialize, Deserialize)]
struct CacheFile {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    tracks: HashMap<String, LoudnessEntry>,
}

fn cache_path() -> PathBuf {
    crate::paths::loudness()
}

struct Cache {
    entries: HashMap<String, LoudnessEntry>,
    /// Paths whose measurement has been queued or is running.
    in_flight: std::collections::HashSet<String>,
    dirty: bool,
}

fn cache() -> &'static Mutex<Cache> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    CACHE.get_or_init(|| {
        let entries = std::fs::read_to_string(cache_path())
            .ok()
            .and_then(|s| serde_json::from_str::<CacheFile>(&s).ok())
            .map(|c| c.tracks)
            .unwrap_or_default();
        Mutex::new(Cache {
            entries,
            in_flight: Default::default(),
            dirty: false,
        })
    })
}

fn flush_cache() {
    let (tracks, dirty) = {
        let mut guard = match cache().lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        if !guard.dirty {
            return;
        }
        guard.dirty = false;
        (guard.entries.clone(), true)
    };
    if !dirty {
        return;
    }
    if let Ok(json) = serde_json::to_string_pretty(&CacheFile { version: 1, tracks }) {
        if let Some(parent) = cache_path().parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let tmp = cache_path().with_extension("json.tmp");
        if std::fs::write(&tmp, json).is_ok() {
            let _ = std::fs::rename(&tmp, cache_path());
        }
    }
}

/// Linear gain the mixer should apply before this track hits the fader.
///
/// Cheap and lock-light: it never decodes anything. If a file has neither tags
/// nor a cached measurement, a background measurement is queued and unity gain
/// is returned for this play.
pub fn gain_for(path: &Path, mode: ReplayGainMode) -> f32 {
    if mode == ReplayGainMode::Off {
        return 1.0;
    }
    let Some(key) = path.to_str() else {
        return 1.0;
    };

    // Cached measurement wins: it carries both loudness and peak.
    if let Some(entry) = lookup(key) {
        if entry.lufs.is_some() || entry.peak.is_some() {
            let mut gain_db = entry.lufs.map(|lufs| TARGET_LUFS - lufs).unwrap_or(0.0);
            if let Some(peak) = entry.peak {
                gain_db = limit_by_peak(gain_db, peak);
            }
            return db_to_linear(gain_db);
        }
    }

    match read_tags(path, mode) {
        Some(Tags {
            gain_db: Some(gain_db),
            peak,
        }) => {
            let limited = match peak {
                Some(peak) => limit_by_peak(gain_db, peak),
                None => gain_db,
            };
            db_to_linear(limited)
        }
        _ => {
            queue_analysis(key.to_string(), path.to_path_buf());
            1.0
        }
    }
}

/// Force-upsert a measurement, used by the analyser and by tests.
pub fn put(key: &str, entry: LoudnessEntry) {
    let Ok(mut guard) = cache().lock().map_err(|p| p.into_inner()) else {
        return;
    };
    guard.entries.insert(key.to_string(), entry);
    guard.dirty = true;
    drop(guard);
    flush_cache();
}

fn lookup(key: &str) -> Option<LoudnessEntry> {
    let guard = cache().lock().ok()?;
    guard.entries.get(key).cloned()
}

fn queue_analysis(key: String, path: PathBuf) {
    {
        let mut guard = match cache().lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        if guard.entries.contains_key(&key) || !guard.in_flight.insert(key.clone()) {
            return;
        }
    }
    std::thread::spawn(move || {
        let entry = match analyse(&path) {
            Ok(entry) => entry,
            Err(_) => LoudnessEntry::default(),
        };
        if let Ok(mut guard) = cache().lock().map_err(|p| p.into_inner()) {
            guard.in_flight.remove(&key);
            // Keep even a silent file's entry: a measurement ran, so there is
            // no point running it again on every play.
            if entry.peak.is_some() {
                guard.entries.insert(key, entry);
                guard.dirty = true;
            }
        }
        flush_cache();
    });
}

// ── Tags ────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug)]
struct Tags {
    gain_db: Option<f64>,
    peak: Option<f64>,
}

/// Pull ReplayGain tags with lofty. Peak values in the wild are sometimes in
/// dBFS and sometimes linear; both are handled.
fn read_tags(path: &Path, mode: ReplayGainMode) -> Option<Tags> {
    use lofty::file::TaggedFileExt;
    use lofty::prelude::*;
    use lofty::tag::ItemKey;

    let tagged = lofty::read_from_path(path).ok()?;
    let tag = tagged.primary_tag().or_else(|| tagged.first_tag())?;

    let get = |key: ItemKey| tag.get_string(&key);
    let parse_gain = |s: &str| {
        let cleaned = s
            .trim()
            .trim_end_matches(|c: char| c.is_ascii_alphabetic())
            .trim();
        cleaned.parse::<f64>().ok()
    };

    let track_gain = get(ItemKey::ReplayGainTrackGain).and_then(|s| parse_gain(s));
    let album_gain = get(ItemKey::ReplayGainAlbumGain).and_then(|s| parse_gain(s));
    let track_peak = get(ItemKey::ReplayGainTrackPeak).and_then(|s| parse_peak(s));
    let album_peak = get(ItemKey::ReplayGainAlbumPeak).and_then(|s| parse_peak(s));

    match mode {
        ReplayGainMode::Album if album_gain.is_some() => Some(Tags {
            gain_db: album_gain,
            peak: album_peak.or(track_peak),
        }),
        _ => Some(Tags {
            gain_db: track_gain,
            peak: track_peak,
        }),
    }
    .filter(|t| t.gain_db.is_some())
}

fn parse_peak(s: &str) -> Option<f64> {
    let s = s.trim();
    let value = s.parse::<f64>().ok()?;
    // Peaks above 1.0 are almost always dBFS from older writers.
    if value < 0.0 || value > 4.0 {
        Some(10f64.powf(value / 20.0))
    } else {
        Some(value)
    }
}

// ── Math ────────────────────────────────────────────────────────────────────

fn db_to_linear(db: f64) -> f32 {
    let db = db.clamp(-MAX_GAIN_DB, MAX_GAIN_DB);
    10f64.powf(db / 20.0) as f32
}

fn limit_by_peak(gain_db: f64, peak: f64) -> f64 {
    if peak <= 0.0 {
        return 0.0;
    }
    let headroom = 20.0 * (PEAK_CEILING / peak).log10();
    gain_db.min(headroom).clamp(-MAX_GAIN_DB, MAX_GAIN_DB)
}

fn loudness_of_power(power: f64) -> f64 {
    -0.691 + 10.0 * power.log10()
}

/// ITU-R BS.1770-4 gated integrated loudness over block powers
/// (sum of per-channel mean squares).
pub(crate) fn integrated_loudness(block_powers: &[f64]) -> Option<f64> {
    let abs_gate_power = 10f64.powf((SILENCE_LUFS + 0.691) / 10.0);

    let above_abs: Vec<f64> = block_powers
        .iter()
        .copied()
        .filter(|p| *p > abs_gate_power)
        .collect();
    if above_abs.is_empty() {
        return None;
    }

    let mean_abs = above_abs.iter().sum::<f64>() / above_abs.len() as f64;
    let rel_gate_loudness = loudness_of_power(mean_abs) - 10.0;
    let rel_gate_power = 10f64.powf((rel_gate_loudness + 0.691) / 10.0);

    let above_rel: Vec<f64> = above_abs
        .iter()
        .copied()
        .filter(|p| *p > rel_gate_power)
        .collect();
    let blocks = if above_rel.is_empty() {
        &above_abs
    } else {
        &above_rel
    };

    let mean = blocks.iter().sum::<f64>() / blocks.len() as f64;
    if mean <= 0.0 {
        return None;
    }
    Some(loudness_of_power(mean))
}

// ── Analysis ────────────────────────────────────────────────────────────────

struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    x1: f64,
    x2: f64,
    y1: f64,
    y2: f64,
}

impl Biquad {
    #[inline]
    fn process(&mut self, x: f64) -> f64 {
        let y = self.b0 * x + self.b1 * self.x1 + self.b2 * self.x2
            - self.a1 * self.y1
            - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

/// Stage 1 of the K-weighting curve: a +4 dB high shelf.
fn k_shelf(rate: f64) -> Biquad {
    let f0 = 1681.974_450_955_533;
    let gain_db = 3.999_843_853_973_347;
    let q = 0.707_175_236_955_419_6;

    let k = (std::f64::consts::PI * f0 / rate).tan();
    let vh = 10f64.powf(gain_db / 20.0);
    let vb = vh.powf(0.499_666_774_154_541_6);
    let a0 = 1.0 + k / q + k * k;

    Biquad {
        b0: (vh + vb * k / q + k * k) / a0,
        b1: 2.0 * (k * k - vh) / a0,
        b2: (vh - vb * k / q + k * k) / a0,
        a1: 2.0 * (k * k - 1.0) / a0,
        a2: (1.0 - k / q + k * k) / a0,
        x1: 0.0,
        x2: 0.0,
        y1: 0.0,
        y2: 0.0,
    }
}

/// Stage 2 of the K-weighting curve: the R128 high-pass at 38 Hz.
fn k_highpass(rate: f64) -> Biquad {
    let f0 = 38.135_470_876_024_44;
    let q = 0.500_327_037_323_877_3;
    let k = (std::f64::consts::PI * f0 / rate).tan();
    let a0 = 1.0 + k / q + k * k;

    Biquad {
        b0: 1.0,
        b1: -2.0,
        b2: 1.0,
        a1: 2.0 * (k * k - 1.0) / a0,
        a2: (1.0 - k / q + k * k) / a0,
        x1: 0.0,
        x2: 0.0,
        y1: 0.0,
        y2: 0.0,
    }
}

/// Measure one file. Returns integrated loudness and sample peak.
pub fn analyse(path: &Path) -> Result<LoudnessEntry> {
    let file = std::fs::File::open(path)?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|e| anyhow!("unsupported format: {e}"))?;
    let mut format = probed.format;

    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or_else(|| anyhow!("no audio track"))?;

    let track_id = track.id;
    let rate = track.codec_params.sample_rate.unwrap_or(44100) as f64;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| anyhow!("decoder: {e}"))?;

    let block_frames = (BLOCK_SECS * rate).round() as usize;
    let hop_frames = (HOP_SECS * rate).round() as usize;
    if block_frames == 0 || hop_frames == 0 {
        return Err(anyhow!("bad sample rate"));
    }

    let mut meter = Meter::new(rate, block_frames, hop_frames);

    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymphoniaError::IoError(_)) => break,
            Err(SymphoniaError::ResetRequired) => {
                decoder.reset();
                continue;
            }
            Err(e) => return Err(anyhow!("packet: {e}")),
        };
        if packet.track_id() != track_id {
            continue;
        }

        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(e) => return Err(anyhow!("decode: {e}")),
        };

        let spec = *decoded.spec();
        let channels = spec.channels.count();
        let mut conv = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
        conv.copy_interleaved_ref(decoded);
        let samples = conv.samples();

        for frame in samples.chunks(channels) {
            let l = frame.first().copied().unwrap_or(0.0) as f64;
            // A mono source counts as one channel, not as two identical ones:
            // duplicating it would inflate loudness by 3 dB.
            let r = if channels >= 2 {
                frame.get(1).copied().map(|v| v as f64)
            } else {
                None
            };
            meter.push(l, r);
        }
    }

    Ok(LoudnessEntry {
        lufs: integrated_loudness(&meter.block_powers),
        peak: Some(meter.peak),
    })
}

/// Streaming BS.1770 meter: K-weights frames and emits one gating-block power
/// every 100 ms.
struct Meter {
    shelf: Biquad,
    highpass: Biquad,
    ring: Vec<f64>,
    ring_pos: usize,
    filled: usize,
    since_hop: usize,
    hop_frames: usize,
    block_powers: Vec<f64>,
    peak: f64,
}

impl Meter {
    fn new(rate: f64, block_frames: usize, hop_frames: usize) -> Self {
        Self {
            shelf: k_shelf(rate),
            highpass: k_highpass(rate),
            ring: vec![0.0; block_frames],
            ring_pos: 0,
            filled: 0,
            since_hop: 0,
            hop_frames,
            block_powers: Vec::new(),
            peak: 0.0,
        }
    }

    fn push(&mut self, l: f64, r: Option<f64>) {
        self.peak = self
            .peak
            .max(l.abs())
            .max(r.map(|v| v.abs()).unwrap_or(0.0));

        let wl = self.highpass.process(self.shelf.process(l));
        let mut power = wl * wl;
        if let Some(r) = r {
            let wr = self.highpass.process(self.shelf.process(r));
            power += wr * wr;
        }

        self.ring[self.ring_pos] = power;
        self.ring_pos = (self.ring_pos + 1) % self.ring.len();
        self.filled = (self.filled + 1).min(self.ring.len());
        self.since_hop += 1;

        if self.since_hop >= self.hop_frames && self.filled == self.ring.len() {
            self.since_hop = 0;
            let sum: f64 = self.ring.iter().sum();
            self.block_powers.push(sum / self.ring.len() as f64);
        }
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silence_has_no_loudness() {
        assert_eq!(integrated_loudness(&[0.0; 10]), None);
        assert_eq!(integrated_loudness(&[]), None);
    }

    #[test]
    fn steady_power_matches_the_formula() {
        // A one-channel-independent power of 0.005 is -23.7 LUFS.
        let power = 0.005;
        let lufs = integrated_loudness(&[power; 20]).unwrap();
        let expected = -0.691 + 10.0 * power.log10();
        assert!((lufs - expected).abs() < 1e-9);
    }

    #[test]
    fn relative_gate_ignores_quiet_blocks() {
        // Loud blocks at 0.1 (≈ -10.7 LUFS), quiet blocks 20 dB down. The
        // gate drops everything more than 10 LU below the mean, so the quiet
        // tail must not drag the result down.
        let loud = 0.1;
        let quiet = 0.001;
        let mut blocks = vec![loud; 20];
        blocks.extend(std::iter::repeat(quiet).take(20));
        let lufs = integrated_loudness(&blocks).unwrap();
        let loud_only = integrated_loudness(&[loud; 20]).unwrap();
        assert!((lufs - loud_only).abs() < 1e-9);
    }

    #[test]
    fn absolute_gate_drops_near_silence() {
        let blocks = vec![1e-9; 30];
        assert_eq!(integrated_loudness(&blocks), None);
    }

    #[test]
    fn db_to_linear_roundtrip() {
        assert!((db_to_linear(0.0) - 1.0).abs() < 1e-6);
        assert!((db_to_linear(-6.0206) - 0.5).abs() < 1e-4);
        assert!((db_to_linear(6.0206) - 2.0).abs() < 1e-3);
    }

    #[test]
    fn peak_limiting_never_clips() {
        // +12 dB asked for, peak at full scale: capped to ~-0.18 dB.
        let limited = limit_by_peak(12.0, 1.0);
        assert!(limited < 0.0);
        assert!((db_to_linear(limited) as f64 - PEAK_CEILING).abs() < 1e-6);
        // A quiet track is allowed its full boost.
        assert_eq!(limit_by_peak(12.0, 0.1), 12.0);
    }

    #[test]
    fn k_weighting_is_stable_and_boosts_highs() {
        let rate = 48000.0;
        // Feed a 1 kHz tone and a 10 kHz tone; the shelf must lift the highs.
        let mut responses = [0f64; 2];
        for (i, freq) in [1000.0f64, 10000.0].iter().enumerate() {
            let mut shelf = k_shelf(rate);
            let mut hp = k_highpass(rate);
            let mut peak = 0f64;
            let mut sum = 0f64;
            let n = 48000;
            for s in 0..n {
                let x = (2.0 * std::f64::consts::PI * freq * s as f64 / rate).sin() * 0.5;
                let y = hp.process(shelf.process(x));
                if s > 4800 {
                    sum += y * y;
                    peak = peak.max(y.abs());
                }
            }
            responses[i] = (sum / (n - 4800) as f64).sqrt();
            assert!(peak.is_finite(), "filter blew up at {freq} Hz");
        }
        assert!(
            responses[1] > responses[0] * 1.3,
            "expected a high-frequency lift, got {responses:?}"
        );
    }

    /// End-to-end: write a real WAV, decode it with symphonia, meter it.
    #[test]
    fn analyses_a_wav_file_end_to_end() {
        let dir = std::env::temp_dir().join("omatunes-loudness-tests");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("sine-1k.wav");

        let rate = 48_000u32;
        let secs = 3u32;
        let frames = rate * secs;
        let mut pcm = Vec::with_capacity(frames as usize * 2);
        for i in 0..frames {
            let t = i as f64 / rate as f64;
            let sample = (2.0 * std::f64::consts::PI * 1000.0 * t).sin() * 0.5;
            pcm.extend_from_slice(&((sample * i16::MAX as f64) as i16).to_le_bytes());
        }

        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + pcm.len() as u32).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
        wav.extend_from_slice(&1u16.to_le_bytes()); // mono
        wav.extend_from_slice(&rate.to_le_bytes());
        wav.extend_from_slice(&(rate * 2).to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
        wav.extend_from_slice(&pcm);
        std::fs::write(&path, &wav).unwrap();

        let entry = analyse(&path).unwrap();
        let lufs = entry.lufs.expect("tone should have loudness");
        // A 1 kHz sine at 0.5 amplitude is about -9 LUFS after K-weighting.
        assert!(
            (-10.0..=-8.0).contains(&lufs),
            "unexpected loudness {lufs} LUFS"
        );
        assert!((entry.peak.unwrap() - 0.5).abs() < 0.01);

        std::fs::remove_file(&path).ok();
    }
}
