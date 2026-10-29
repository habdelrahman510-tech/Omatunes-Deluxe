# Changelog

All notable changes to this build of Omatunes. This fork is maintained by
[prof.dog](https://github.com/habdelrahman510-tech); the projects it descends from are
credited in [README.md](README.md).

## [1.3.0] — 2026-10-29

First release under this maintainer. Everything below is new relative to
upstream 1.2.1.

### Added

- **10-band parametric equalizer.** New `src/audio/eq.rs` DSP engine plus an
  Equalizer tab in the right-hand panel.
  - Ten ISO bands at 31.5, 63, 125, 250, 500, 1k, 2k, 4k, 8k and 16 kHz, Q 1.414.
  - RBJ peaking biquads in Direct Form II transposed form, independent filter
    state per channel.
  - Filtering happens inside the cpal output callback after decoding and
    before the volume stage; the visualizer taps the post-EQ signal.
  - 20 ms gain ramps in 64-frame blocks, dry/wet crossfade on bypass, and
    bit-exact bypass when the EQ is off or flat.
  - Soft-clip knee above 0.95 to keep boosted output clean.
  - Preamp −12…0 dB with automatic headroom equal to the largest band boost.
  - 16 built-in presets, from Flat and Loudness to Night and Hip-Hop.
  - Live magnitude-response curve, per-band sliders, reset.
  - Settings persist in `db.json` and are applied before the first sample plays.
  - Unit tests cover bypass exactness and effective boost.
- **Arabic and RTL text support.** Bundled Noto Sans Arabic (regular + bold)
  and a shaping layer (`src/ui/txt.rs`) used by every label in the interface.
  - RTL detection across Hebrew, Arabic, Syriac, Thaana, NKo, Samaritan,
    Arabic Presentation Forms, Adlam and Mende Kikakui ranges.
  - A left-to-right mark guard keeps the paragraph layout stable while the RTL
    run still shapes right-to-left.
  - `Shaping::Advanced` throughout, so Arabic letters join correctly.
  - Twelve UI modules switched to the guarded text widget, so metadata
    (artist, album, title, genre, playlists) renders correctly, not just lyrics.
- `guard_lines()` for long-form lyrics: applies the direction guard per line,
  because text shaping resolves base direction per paragraph and a single
  leading mark only fixed the first line.

### Changed

- Unsynchronized lyrics now wrap at the panel width instead of overflowing.
- Long-form lyrics keep a consistent left margin for every RTL line.
- Package metadata: `authors`, `description`, `license` and `repository` set.

### Removed

- `scripts/omatunes-sync.service`. It referenced `~/.local/bin/omatunes_scripts/git_sync.sh`,
  which does not exist, so the unit could never start.

## Upstream notes

Features inherited from upstream 1.2.x — liked queue, atomic database writes,
`.lrc` sidecar support, shuffle, widget art, tag-editor polish, spectrum
polling gated to the active visualizer, cover art downscaled to 512×512, WGPU
backend fallback, relative MPRIS Seek, Opus decoder reset on seek — are not
this maintainer's work and are not claimed as such.
