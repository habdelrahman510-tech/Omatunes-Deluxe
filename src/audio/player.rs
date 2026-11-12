//! Audio output engine.
//!
//! Two decks ("current" and "next") feed one cpal callback. The app decides
//! what comes next and issues [`AudioCommand::Preload`]; the engine splices or
//! crossfades into it at the sample boundary, so a track change no longer goes
//! through a stop/start cycle and no longer truncates the tail of the previous
//! track.
//!
//! Everything the callback needs lives behind one mutex: the decks, the fade
//! position, the volume and the paused flag. Decoders write into per-deck ring
//! buffers that the callback drains; nothing in the callback allocates or
//! blocks.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{anyhow, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, SampleRate, StreamConfig};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::{FormatOptions, SeekMode, SeekTo};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::core::units::Time as SymphoniaTime;
use tokio::sync::mpsc;

use crate::audio::eq::{EqHandle, EqProcessor, EqSettings};
use crate::audio::loudness::{self, ReplayGainMode};

const OUTPUT_RATE: u32 = 48000;
const OUTPUT_CHANNELS: u16 = 2;
/// One second of stereo audio, the most a decoder may run ahead of playback.
const BUFFER_SAMPLES: usize = OUTPUT_RATE as usize * 2;
/// How often the mixer reports position to the UI.
const PROGRESS_FRAMES: u64 = OUTPUT_RATE as u64 / 10;
/// Short fade applied when a track is started or seeked by hand, so a manual
/// jump never clicks.
const FADE_IN_FRAMES: u64 = OUTPUT_RATE as u64 * 60 / 1000;

fn sync_system_volume(volume: f32) {
    let vol_pct = format!("{}%", (volume * 100.0).round() as u32);
    std::thread::spawn(move || {
        if let Ok(out) = std::process::Command::new("pactl")
            .arg("list")
            .arg("sink-inputs")
            .output()
        {
            let text = String::from_utf8_lossy(&out.stdout);
            let mut current_id = None;
            for line in text.lines() {
                let line = line.trim();
                if line.starts_with("Sink Input #") {
                    if let Some(id_str) = line.strip_prefix("Sink Input #") {
                        current_id = id_str.parse::<u32>().ok();
                    }
                } else if line.contains("omatunes") {
                    if let Some(id) = current_id {
                        let _ = std::process::Command::new("pactl")
                            .arg("set-sink-input-volume")
                            .arg(id.to_string())
                            .arg(&vol_pct)
                            .status();
                        break;
                    }
                }
            }
        }
    });
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackState {
    Stopped,
    Playing,
    Paused,
}

#[derive(Debug)]
pub enum AudioCommand {
    Play(PathBuf),
    /// Queue the next track. The engine starts it at the sample boundary
    /// (gapless) or fades into it when a crossfade is configured.
    Preload(Option<PathBuf>),
    Pause,
    Resume,
    Stop,
    Seek(Duration),
    SetVolume(f32),
    SetCrossfade(f32),
    SetReplayGain(ReplayGainMode),
}

#[derive(Debug, Clone)]
pub enum AudioEvent {
    Playing {
        path: PathBuf,
    },
    Paused,
    Stopped,
    Progress {
        position: Duration,
        duration: Duration,
    },
    Error(String),
    /// The engine ran out of audio with nothing preloaded.
    TrackEnded,
    /// Gapless/crossfade advance: the app should update its idea of the
    /// current track without issuing a new Play.
    Spliced {
        path: PathBuf,
    },
}

pub struct AudioPlayer {
    pub cmd_tx: mpsc::UnboundedSender<AudioCommand>,
    pub event_rx: mpsc::UnboundedReceiver<AudioEvent>,
    pub sample_buffer: Arc<Mutex<VecDeque<f32>>>,
    pub eq: EqHandle,
}

impl AudioPlayer {
    pub fn spawn() -> Self {
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let sample_buffer: Arc<Mutex<VecDeque<f32>>> =
            Arc::new(Mutex::new(VecDeque::with_capacity(8192)));

        let eq = EqHandle::new(EqSettings::default());
        let eq_thread = eq.clone();
        let buf_clone = sample_buffer.clone();
        std::thread::spawn(move || audio_thread(cmd_rx, event_tx, buf_clone, eq_thread));

        AudioPlayer {
            cmd_tx,
            event_rx,
            sample_buffer,
            eq,
        }
    }

    pub fn send(&self, cmd: AudioCommand) {
        let _ = self.cmd_tx.send(cmd);
    }
}

// ── Sources ───────────────────────────────────────────────────────────────────

/// One playable stream: a path, a ring buffer fed by a decoder thread, and the
/// bookkeeping the mixer needs to know where it is.
#[derive(Clone)]
struct Source {
    path: PathBuf,
    buf: Arc<Mutex<VecDeque<f32>>>,
    done: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
    total_frames: Arc<AtomicU64>,
    gain: f32,
    /// Frames already handed to the callback.
    played: u64,
    /// Frame offset this source started at (non-zero after a seek).
    start_frame: u64,
    /// Frames at the last Progress event.
    reported: u64,
    ended: bool,
}

impl Source {
    fn new(path: PathBuf, gain: f32, start_frame: u64) -> Self {
        Self {
            path,
            buf: Arc::new(Mutex::new(VecDeque::with_capacity(BUFFER_SAMPLES))),
            done: Arc::new(AtomicBool::new(false)),
            cancel: Arc::new(AtomicBool::new(false)),
            total_frames: Arc::new(AtomicU64::new(0)),
            gain,
            played: 0,
            start_frame,
            reported: 0,
            ended: false,
        }
    }

    fn stop(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }

    /// Pop one stereo frame. Missing samples read as silence.
    #[inline]
    fn pop_frame(&self) -> (f32, f32) {
        let mut buf = self.buf.lock().unwrap();
        let l = buf.pop_front().unwrap_or(0.0);
        let r = buf.pop_front().unwrap_or(0.0);
        drop(buf);
        (l * self.gain, r * self.gain)
    }

    fn is_empty(&self) -> bool {
        self.buf.lock().unwrap().is_empty()
    }

    fn frames_done(&self) -> bool {
        self.done.load(Ordering::SeqCst) && self.is_empty()
    }

    fn position(&self) -> Duration {
        Duration::from_secs_f64((self.start_frame + self.played) as f64 / OUTPUT_RATE as f64)
    }

    fn duration(&self) -> Duration {
        let total = self.total_frames.load(Ordering::SeqCst);
        if total > 0 {
            Duration::from_secs_f64(total as f64 / OUTPUT_RATE as f64)
        } else {
            // Unknown length (Opus/OGG): mirror position so the UI stays sane.
            self.position()
        }
    }
}

// ── Mixer ────────────────────────────────────────────────────────────────────

struct Mixer {
    current: Option<Source>,
    next: Option<Source>,
    crossfade_frames: u64,
    fade_frames: u64,
    fading: bool,
    paused: bool,
    volume: f32,
    /// Frames left in the short anti-click ramp started by a manual jump.
    fade_in_left: u64,
    fade_in_total: u64,
    tx: mpsc::UnboundedSender<AudioEvent>,
    /// Events produced during a callback, sent once the lock is released.
    pending: Vec<AudioEvent>,
}

impl Mixer {
    fn emit(&mut self, event: AudioEvent) {
        self.pending.push(event);
    }

    fn flush_events(&mut self) {
        for event in self.pending.drain(..) {
            let _ = self.tx.send(event);
        }
    }

    fn start_fade_if_due(&mut self) {
        if self.fading || self.crossfade_frames == 0 {
            return;
        }
        let Some(current) = self.current.as_ref() else {
            return;
        };
        // Do not start a fade into a deck that has not produced audio yet:
        // the decoder may still be opening the file.
        let ready = self
            .next
            .as_ref()
            .is_some_and(|next| !next.is_empty() && !next.frames_done());
        if !ready {
            return;
        }
        let total = current.total_frames.load(Ordering::SeqCst);
        if total == 0 {
            return;
        }
        let fade_from = total.saturating_sub(self.crossfade_frames);
        if current.played >= fade_from {
            self.fading = true;
            self.fade_frames = 0;
        }
    }

    #[inline]
    fn fade_gains(&self) -> (f32, f32) {
        let progress = if self.crossfade_frames == 0 {
            1.0
        } else {
            (self.fade_frames as f32 / self.crossfade_frames as f32).clamp(0.0, 1.0)
        };
        // Equal power, so the sum never dips in the middle of the fade.
        let angle = progress * std::f32::consts::FRAC_PI_2;
        (angle.cos(), angle.sin())
    }

    #[inline]
    fn take_fade_in(&mut self) -> f32 {
        if self.fade_in_left == 0 {
            return 1.0;
        }
        let remaining = self.fade_in_left as f32 / self.fade_in_total.max(1) as f32;
        let progress = 1.0 - remaining;
        // Smoothstep, so the ramp has no corners at either end.
        let gain = progress * progress * (3.0 - 2.0 * progress);
        self.fade_in_left -= 1;
        gain
    }

    fn arm_fade_in(&mut self) {
        self.fade_in_total = FADE_IN_FRAMES;
        self.fade_in_left = FADE_IN_FRAMES;
    }

    /// Swap decks. `announce` tells the app that the change came from the
    /// engine rather than from a `Play` it issued itself.
    fn promote_next(&mut self, announce: bool) {
        let Some(next) = self.next.take() else {
            return;
        };
        if let Some(old) = self.current.take() {
            old.stop();
        }
        self.fading = false;
        self.fade_frames = 0;
        let path = next.path.clone();
        self.current = Some(next);
        if announce {
            self.emit(AudioEvent::Spliced { path });
        }
    }
}

// ── Audio thread ─────────────────────────────────────────────────────────────

fn audio_thread(
    mut cmd_rx: mpsc::UnboundedReceiver<AudioCommand>,
    event_tx: mpsc::UnboundedSender<AudioEvent>,
    sample_buffer: Arc<Mutex<VecDeque<f32>>>,
    eq_handle: EqHandle,
) {
    let host = cpal::default_host();
    let device = match host.default_output_device() {
        Some(d) => d,
        None => {
            let _ = event_tx.send(AudioEvent::Error("No audio device".into()));
            return;
        }
    };

    let sample_format = device
        .default_output_config()
        .map(|c| c.sample_format())
        .unwrap_or(SampleFormat::F32);

    let stream_config = StreamConfig {
        channels: OUTPUT_CHANNELS,
        sample_rate: SampleRate(OUTPUT_RATE),
        buffer_size: cpal::BufferSize::Default,
    };

    let mixer = Arc::new(Mutex::new(Mixer {
        current: None,
        next: None,
        crossfade_frames: 0,
        fade_frames: 0,
        fading: false,
        paused: false,
        volume: 0.8,
        fade_in_left: 0,
        fade_in_total: 0,
        tx: event_tx.clone(),
        pending: Vec::new(),
    }));

    let err_fn = |e| eprintln!("Erro stream: {e}");
    let eq = EqProcessor::new(eq_handle, OUTPUT_RATE as f32);

    let stream = match sample_format {
        SampleFormat::I16 => {
            let mixer_cb = mixer.clone();
            let vis = sample_buffer.clone();
            let mut eq = eq;
            let mut scratch: Vec<f32> = Vec::new();
            device.build_output_stream(
                &stream_config,
                move |data: &mut [i16], _| {
                    scratch.resize(data.len(), 0.0);
                    fill_output(&mut scratch, &mixer_cb, &vis, &mut eq);
                    for (d, s) in data.iter_mut().zip(scratch.iter()) {
                        *d = cpal::Sample::from_sample(*s);
                    }
                },
                err_fn,
                None,
            )
        }
        _ => {
            let mixer_cb = mixer.clone();
            let vis = sample_buffer.clone();
            let mut eq = eq;
            device.build_output_stream(
                &stream_config,
                move |data: &mut [f32], _| fill_output(data, &mixer_cb, &vis, &mut eq),
                err_fn,
                None,
            )
        }
    };

    let stream = match stream {
        Ok(s) => s,
        Err(e) => {
            let _ = event_tx.send(AudioEvent::Error(format!("Build stream: {e}")));
            return;
        }
    };

    if let Err(e) = stream.play() {
        let _ = event_tx.send(AudioEvent::Error(format!("Stream play: {e}")));
        return;
    }

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    let mut replaygain = ReplayGainMode::Off;

    rt.block_on(async {
        while let Some(cmd) = cmd_rx.recv().await {
            match cmd {
                AudioCommand::Play(path) => {
                    let mut state = mixer.lock().unwrap();
                    if let Some(source) = state.current.take() {
                        source.stop();
                    }
                    if let Some(source) = state.next.take() {
                        source.stop();
                    }
                    state.fading = false;
                    state.fade_frames = 0;
                    state.paused = false;
                    drop(state);

                    let gain = loudness::gain_for(&path, replaygain);
                    let source = Source::new(path.clone(), gain, 0);
                    spawn_decoder(&source, None);

                    let mut state = mixer.lock().unwrap();
                    state.current = Some(source);
                    state.arm_fade_in();
                    let _ = state.tx.send(AudioEvent::Playing { path });
                }

                AudioCommand::Preload(next) => {
                    let mut state = mixer.lock().unwrap();
                    let Some(path) = next else {
                        if let Some(source) = state.next.take() {
                            source.stop();
                        }
                        state.fading = false;
                        state.fade_frames = 0;
                        continue;
                    };
                    // Progress is reported ten times a second; the app
                    // re-issues the same preload until it changes. Ignoring a
                    // repeat keeps a running crossfade intact.
                    if state.next.as_ref().is_some_and(|n| n.path == path) {
                        continue;
                    }
                    if let Some(source) = state.next.take() {
                        source.stop();
                    }
                    state.fading = false;
                    state.fade_frames = 0;

                    let gain = loudness::gain_for(&path, replaygain);
                    let source = Source::new(path, gain, 0);
                    spawn_decoder(&source, None);
                    state.next = Some(source);
                }

                AudioCommand::Pause => {
                    let mut state = mixer.lock().unwrap();
                    state.paused = true;
                    let _ = state.tx.send(AudioEvent::Paused);
                }

                AudioCommand::Resume => {
                    let mut state = mixer.lock().unwrap();
                    state.paused = false;
                    state.arm_fade_in();
                    if let Some(source) = state.current.as_ref() {
                        let _ = state.tx.send(AudioEvent::Playing {
                            path: source.path.clone(),
                        });
                    }
                }

                AudioCommand::SetVolume(v) => {
                    let clamped = v.clamp(0.0, 1.0);
                    mixer.lock().unwrap().volume = clamped;
                    sync_system_volume(clamped);
                }

                AudioCommand::SetCrossfade(secs) => {
                    let frames = (secs.clamp(0.0, 12.0) as f64 * OUTPUT_RATE as f64) as u64;
                    mixer.lock().unwrap().crossfade_frames = frames;
                }

                AudioCommand::SetReplayGain(mode) => {
                    replaygain = mode;
                }

                AudioCommand::Stop => {
                    let mut state = mixer.lock().unwrap();
                    if let Some(source) = state.current.take() {
                        source.stop();
                    }
                    if let Some(source) = state.next.take() {
                        source.stop();
                    }
                    state.fading = false;
                    state.fade_frames = 0;
                    state.paused = false;
                    state.fade_in_left = 0;
                    state.fade_in_total = 0;
                    let _ = state.tx.send(AudioEvent::Stopped);
                }

                AudioCommand::Seek(pos) => {
                    let path = {
                        let mut state = mixer.lock().unwrap();
                        let path = state.current.as_ref().map(|s| s.path.clone());
                        if let Some(source) = state.current.take() {
                            source.stop();
                        }
                        if let Some(source) = state.next.take() {
                            source.stop();
                        }
                        state.fading = false;
                        state.fade_frames = 0;
                        path
                    };

                    let Some(path) = path else {
                        continue;
                    };

                    let start = (pos.as_secs_f64() * OUTPUT_RATE as f64) as u64;
                    let gain = loudness::gain_for(&path, replaygain);
                    let source = Source::new(path, gain, start);
                    spawn_decoder(&source, Some(pos));
                    let mut state = mixer.lock().unwrap();
                    state.current = Some(source);
                    state.arm_fade_in();
                }
            }
        }
    });
}

fn spawn_decoder(source: &Source, seek_to: Option<Duration>) {
    let source = source.clone();
    tokio::task::spawn_blocking(move || {
        if let Err(e) = decode_source(&source, seek_to) {
            let _ = e; // reported by the mixer when the buffer runs dry
            source.done.store(true, Ordering::SeqCst);
        }
        source.done.store(true, Ordering::SeqCst);
    });
}

fn fill_output(
    output: &mut [f32],
    mixer: &Arc<Mutex<Mixer>>,
    vis_buffer: &Arc<Mutex<VecDeque<f32>>>,
    eq: &mut EqProcessor,
) {
    let mut state = mixer.lock().unwrap();

    if state.paused {
        for sample in output.iter_mut() {
            *sample = 0.0;
        }
        return;
    }

    let volume = state.volume;
    let frames = output.len() / 2;

    // A deck that was queued but never claimed (the app preloaded while
    // nothing was playing) still needs to be promoted, silently.
    if state.current.is_none() && state.next.is_some() {
        state.promote_next(false);
    }

    for frame in 0..frames {
        let (mut left, mut right) = match state.current.as_ref() {
            Some(current) => current.pop_frame(),
            None => (0.0, 0.0),
        };

        if state.next.is_some() {
            state.start_fade_if_due();
        }

        if state.fading {
            let faded = {
                match state.next.as_ref() {
                    Some(next) if !next.is_empty() => {
                        let (out_gain, in_gain) = state.fade_gains();
                        let (nl, nr) = next.pop_frame();
                        Some((nl * in_gain, nr * in_gain, out_gain))
                    }
                    _ => None,
                }
            };

            if let Some((nl, nr, out_gain)) = faded {
                left = left * out_gain + nl;
                right = right * out_gain + nr;
                state.fade_frames += 1;
                if state.fade_frames >= state.crossfade_frames {
                    state.promote_next(true);
                }
            }
        } else if state
            .current
            .as_ref()
            .is_some_and(|current| current.frames_done())
        {
            if state.next.is_some() {
                // Gapless: swap at the sample boundary, mid output buffer.
                state.promote_next(true);
                if let Some(current) = state.current.as_ref() {
                    let (l, r) = current.pop_frame();
                    left += l;
                    right += r;
                }
            } else if let Some(current) = state.current.as_mut() {
                if !current.ended {
                    current.ended = true;
                    state.emit(AudioEvent::TrackEnded);
                }
            }
        }

        let fade = state.take_fade_in();
        left *= fade;
        right *= fade;

        if let Some(current) = state.current.as_mut() {
            current.played += 1;
            if current.played.saturating_sub(current.reported) >= PROGRESS_FRAMES {
                current.reported = current.played;
                let position = current.position();
                let duration = current.duration();
                state.emit(AudioEvent::Progress { position, duration });
            }
        }

        output[frame * 2] = left.clamp(-1.0, 1.0);
        output[frame * 2 + 1] = right.clamp(-1.0, 1.0);
    }

    let current_finished = state
        .current
        .as_ref()
        .is_some_and(|current| current.frames_done() && current.ended);
    if current_finished {
        state.current = None;
    }

    state.flush_events();
    drop(state);

    // Filter before the volume stage: EQ gain stays independent of the volume
    // slider, and the visualizer taps what is actually being heard.
    eq.process_in_place(output);

    let mut vis = vis_buffer.lock().unwrap();
    for sample in output.iter_mut() {
        *sample *= volume;
        vis.push_back(*sample);
        if vis.len() > 8192 {
            vis.pop_front();
        }
    }
}

// ── Decode ───────────────────────────────────────────────────────────────────

fn decode_source(source: &Source, seek_to: Option<Duration>) -> Result<()> {
    let path = &source.path;
    let file = std::fs::File::open(path)?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let fmt_opts = FormatOptions {
        enable_gapless: true,
        ..Default::default()
    };

    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &fmt_opts, &MetadataOptions::default())
        .map_err(|e| anyhow!("Unsupported format: {e}"))?;

    let mut format = probed.format;

    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or_else(|| anyhow!("No audio track"))?;

    let track_id = track.id;
    let time_base = track.codec_params.time_base;
    let n_frames = track.codec_params.n_frames;
    let file_rate = track.codec_params.sample_rate.unwrap_or(44100);

    let is_opus = track.codec_params.codec == symphonia::core::codecs::CODEC_TYPE_OPUS;
    let n_channels_param = track.codec_params.channels.map(|c| c.count()).unwrap_or(2);

    if let (Some(tb), Some(nf)) = (time_base, n_frames) {
        let frames =
            (nf as f64 * tb.numer as f64 / tb.denom as f64 * OUTPUT_RATE as f64).round() as u64;
        source.total_frames.store(frames, Ordering::SeqCst);
    }

    let mut symphonia_decoder = if !is_opus {
        Some(
            symphonia::default::get_codecs()
                .make(&track.codec_params, &DecoderOptions::default())
                .map_err(|e| anyhow!("Decoder: {e}"))?,
        )
    } else {
        None
    };

    let channels_mode = if n_channels_param == 1 {
        opus::Channels::Mono
    } else {
        opus::Channels::Stereo
    };
    let mut opus_decoder = if is_opus {
        opus::Decoder::new(file_rate, channels_mode).ok()
    } else {
        None
    };

    if let Some(pos) = seek_to {
        let seek_time = SymphoniaTime {
            seconds: pos.as_secs(),
            frac: pos.subsec_nanos() as f64 / 1_000_000_000.0,
        };
        format
            .seek(
                SeekMode::Accurate,
                SeekTo::Time {
                    time: seek_time,
                    track_id: None,
                },
            )
            .ok();
        if let Some(ref mut dec) = symphonia_decoder {
            dec.reset();
        }
        if is_opus {
            opus_decoder = opus::Decoder::new(file_rate, channels_mode).ok();
        }
    }

    loop {
        if source.cancel.load(Ordering::SeqCst) {
            return Ok(());
        }

        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymphoniaError::IoError(_)) => break,
            Err(SymphoniaError::ResetRequired) => {
                if let Some(ref mut dec) = symphonia_decoder {
                    dec.reset();
                }
                if is_opus {
                    opus_decoder = opus::Decoder::new(file_rate, channels_mode).ok();
                }
                continue;
            }
            Err(e) => return Err(anyhow!("Packet: {e}")),
        };

        if packet.track_id() != track_id {
            continue;
        }

        let stereo: Vec<f32> = if is_opus {
            if let Some(ref mut dec) = opus_decoder {
                let mut pcm_buf = vec![0.0f32; 5760 * n_channels_param];
                match dec.decode_float(packet.buf(), &mut pcm_buf, false) {
                    Ok(samples_per_ch) => {
                        let total = samples_per_ch * n_channels_param;
                        pcm_buf.truncate(total);
                        match n_channels_param {
                            1 => pcm_buf.iter().flat_map(|&s| [s, s]).collect(),
                            2 => pcm_buf,
                            n => pcm_buf
                                .chunks(n)
                                .flat_map(|ch| [ch[0], ch.get(1).copied().unwrap_or(ch[0])])
                                .collect(),
                        }
                    }
                    Err(_) => continue,
                }
            } else {
                continue;
            }
        } else {
            let dec = symphonia_decoder.as_mut().unwrap();
            let decoded = match dec.decode(&packet) {
                Ok(d) => d,
                Err(SymphoniaError::DecodeError(_)) => continue,
                Err(e) => return Err(anyhow!("Decode: {e}")),
            };

            let spec = *decoded.spec();
            let n_channels = spec.channels.count();

            let mut conv = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
            conv.copy_interleaved_ref(decoded);
            let raw = conv.samples();

            match n_channels {
                1 => raw.iter().flat_map(|&s| [s, s]).collect(),
                2 => raw.to_vec(),
                n => raw
                    .chunks(n)
                    .flat_map(|ch| {
                        let l = ch.first().copied().unwrap_or(0.0);
                        let r = ch.get(1).copied().unwrap_or(0.0);
                        [l, r]
                    })
                    .collect(),
            }
        };

        let samples = if file_rate != OUTPUT_RATE {
            resample_stereo(&stereo, file_rate, OUTPUT_RATE)
        } else {
            stereo
        };

        loop {
            if source.cancel.load(Ordering::SeqCst) {
                return Ok(());
            }
            if source.buf.lock().unwrap().len() < BUFFER_SAMPLES {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }

        source.buf.lock().unwrap().extend(samples);
    }

    Ok(())
}

fn resample_stereo(input: &[f32], in_rate: u32, out_rate: u32) -> Vec<f32> {
    let ratio = in_rate as f64 / out_rate as f64;
    let in_frames = input.len() / 2;
    let out_frames = (in_frames as f64 / ratio).ceil() as usize;
    let mut out = Vec::with_capacity(out_frames * 2);

    for i in 0..out_frames {
        let src = i as f64 * ratio;
        let idx = src as usize;
        let frac = (src - idx as f64) as f32;

        let l0 = input.get(idx * 2).copied().unwrap_or(0.0);
        let l1 = input.get(idx * 2 + 2).copied().unwrap_or(l0);
        let r0 = input.get(idx * 2 + 1).copied().unwrap_or(0.0);
        let r1 = input.get(idx * 2 + 3).copied().unwrap_or(r0);

        out.push(l0 + (l1 - l0) * frac);
        out.push(r0 + (r1 - r0) * frac);
    }

    out
}
