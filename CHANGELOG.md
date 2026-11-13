# Changelog

All notable changes to this build of Omatunes. This fork is maintained by
[prof.dog](https://github.com/habdelrahman510-tech); the projects it descends from are
credited in [README.md](README.md).

## [2.0.0] — 2026-11-13

The playback engine, the library and the editing tools were rebuilt. Nothing
about how your files are stored changes: the same JSON state, the same folders,
the same tags. What changes is how they are read, played and written.

### Added

- **Gapless playback.** The engine now runs two decks. When a track is within a
  few seconds of ending, the next one is preloaded into an idle deck and
  spliced in at the sample the previous one stops — no silence, no click, no
  second `Play`. Applies to normal advance, shuffle and repeat alike.
- **Crossfade.** Settings > Playback gains a 0–12 s slider. Above zero, the
  engine equal-power crossfades the outgoing and incoming decks instead of
  splicing. Zero keeps playback bit-exact gapless.
- **ReplayGain.** Off / Track / Album, read from the file's ReplayGain tags when
  they exist. Files without tags are measured once with a hand-rolled EBU R128
  (BS.1770-4) analyser, cached in `~/.config/omatunes/loudness.json`, and
  played at unity gain until the measurement lands.
- **Repeat this song only.** The repeat button is now three-state:
  off → repeat queue → repeat track. Repeat-track feeds the same file into the
  idle deck, so it is as gapless as everything else. MPRIS reports
  `None`/`Playlist`/`Track` accordingly.
- **Several library folders.** Settings > Library edits a list of roots instead
  of one path: add, browse, remove. Existing installs migrate the old single
  `music_dir` into the list automatically, and nested roots are de-duplicated.
- **Incremental rescans.** A rescan now reuses cached tags whenever a file's
  mtime and size are unchanged, so a 233-track library costs a stat per track
  instead of an open+parse. The cache is versioned and written atomically.
- **Folder watching.** With watching on, adding, removing or renaming files
  refreshes the library by itself. Bursts are debounced for 750 ms and turn
  into one incremental rescan.
- **Duplicates playlist.** A new auto-playlist groups tracks with the same
  normalised artist and title when their durations are within two seconds, so
  the same recording in FLAC and MP3 lands in one place.
- **Drag and drop.** Drop audio files on the window and they replace the queue
  and start playing; drop a folder and it is walked off the UI thread first;
  drop an image while the tag editor is open and it becomes the cover. A
  full-window scrim explains what the drop will do before you let go.
- **Playlist import.** Dropping an `.m3u`/`.m3u8` file creates a user playlist.
  Relative entries resolve against the playlist's own folder, `file://` URIs and
  percent-encoding are understood, and a duplicate name is suffixed instead of
  overwriting the playlist you already had.
- **Cover art tools.** "Save cover art to folder" writes the embedded picture
  next to the track (never clobbering an existing file), and "Embed folder
  artwork" pushes a `cover.jpg` back into the tags. The tag editor gained a
  button that pulls the folder's artwork into the cover field.

### Changed

- **Saving tags only rewrites what changed.** The tag editor diffs the values
  you are saving against the track on disk and skips files that already match,
  so saving an unchanged selection no longer bumps mtimes and wakes the watcher.
  A toast reports how many files were written and how many were skipped.
- **The app is split into modules.** The 8,000-line `app.rs` became
  `src/app/` with playback, browse, overlays, achievements, tag editor, smart
  playlists and drag-and-drop in their own files. Behaviour is unchanged; the
  diff is much easier to read from here on.
- **CI exists.** GitHub Actions now runs `cargo fmt --check`, an advisory
  clippy pass and the unit tests on every push and pull request, so a broken
  build is caught before it becomes a release.

### Fixed

- Seeking no longer restores the old position on the next track.
- Writing tags while a track plays pauses the decoder for the write and seeks
  back to the same sample afterwards, instead of racing the truncate.
- Sidecar `cover.jpg` files are no longer overwritten by a single-track tag
  save; the sidecar is only created when the folder has no artwork yet.

## [1.2.2] — 2026-11-07

### Added

- **Open with omaTUNES.** Double-clicking a song in a file manager now plays
  *that* song instead of resuming whatever was last playing.
  - New `play <absolute path>` verb on the local control socket
    (UDP `127.0.0.1:18888`).
  - The `omatunes-open` helper is installed as the `audio/*` handler: it hands
    the file to a running player, or starts the player with the file as its
    argument.
  - Files outside `music_dir` play too — their tags are read on demand, so the
    title, artist and album still show.
  - `omatunes /path/to/song.mp3` does the same from a terminal.

### Fixed

- **No more launching on a stray artist.** The saved browsing state is restored
  as it was saved. When nothing was saved the track list opens on everything
  rather than auto-selecting the first artist in the library.
- **The browsing view survives a restart.** The selected playlist (including
  smart playlists), the playlist tab and the folder are persisted when you
  change them, not only when a track happens to start.
- **Keyboard shortcuts no longer jam.** A focused text field consumed `Escape`
  while reporting nothing to the app, so the song search and the tag editor
  could not be dismissed from the keyboard and every shortcut afterwards
  appeared dead until you pressed `Escape` a second time (or restarted).
  `Escape` is now handled at the window level, so it always closes the thing you
  are looking at. Clicking a track, artist, album, genre or playlist also hands
  the keyboard back, so a search box can no longer keep it hostage.
- **`/` actually opens the song search.** It used to focus a widget that was
  not on screen until the search row was open, so the key did nothing at all.
  The same was true of the song-search step in the `Tab` cycle, and the first
  `Tab` on a fresh launch now starts at the sidebar search instead of doing
  nothing.
- **Character shortcuts respect modifiers.** `Ctrl+C` no longer opens the
  new-playlist dialog, `Ctrl+E` the tag editor or `Ctrl+F` "liked"; chords are
  left to the focused widget or the system.
- **Clicking an album switches to the album view**, like clicking an artist or
  a genre already did.
- Queue "move down" no longer risks an out-of-bounds panic when the queue is
  empty.

## [1.3.1] — 2026-10-29

### Fixed

- `library::scanner::tests::test_scan_media_dir` no longer fails a release
  build. It is a smoke test that scans `$HOME/Music`; in a clean build
  directory, chroot or CI there is no library to scan, so it now skips with a
  message instead of panicking. Reported by our own AUR `check()`.

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
