<p align="center">
  <img src="assets/OmaTUNES Logo Transparent.png" alt="OmaTUNES Logo" width="400">
</p>

# omaTUNES

omaTUNES is a native Wayland music player and library manager built in Rust for Hyprland and Omarchy systems. omaTUNES is built to be as lightweight as possible while matching Omarchy's opinionated UI style — it picks up your system theme automatically, so your player always matches the rest of your desktop. omaTUNES is 100% offline, has wide support for most popular music codecs, and is designed to manage very large libraries with robust playlist and library management tools.

Built and maintained by [prof.dog](https://github.com/habdelrahman510-tech). `omatunes` is a fork of
[sheep-farm/lavanda](https://github.com/sheep-farm/lavanda) by Flavio de Vasconcellos, carried forward by
[Balthazzahr](https://github.com/Balthazzahr) — this build adds the equalizer, Arabic/RTL shaping and the
lyrics fixes described below. The original MIT notices are retained in [LICENSE](LICENSE).

## Why "Deluxe"

This build carries upstream omaTUNES 1.2.1 and adds three things:

- **A real DSP equalizer** — ten ISO bands of RBJ peaking biquads running inside the audio callback, sixteen presets, automatic headroom, and click-free bit-exact bypass.
- **Proper Arabic and right-to-left support** — Noto Sans Arabic bundled, advanced shaping so letters join correctly, and a per-line direction guard, so Arabic metadata and lyrics read properly instead of scrambling.
- **Lyrics fixes** — unsynchronized lyrics wrap at the panel width, and every RTL line keeps the correct margin, not just the first.


<p align="center">
  <img src="assets/Main UI.png" alt="OmaTUNES Main UI Screenshot" width="800">
</p>

---

## Key Features

- **Wide Format Support.** MP3, FLAC, OGG, Opus, WAV, AAC, M4A, AIFF — all decoded natively through [Symphonia](https://github.com/pdeljanov/Symphonia), no plugins or codecs to hunt down.
- **Fully Offline & Private.** No telemetry, no accounts, no internet requirement to play a song. Your play counts, likes, and playlists live in one plain JSON file on disk, fully yours.
- **Smart Playlists.** Build iTunes-style rule-based playlists — mix criteria like artist, genre, play count, liked status, or "last played within 2 weeks" — and let them keep themselves up to date as your library changes. No manual curation required.
- **Customizable Library View.** Sort, filter, and browse by Artist, Album, or Genre. Drag columns into whatever order makes sense to you, group tracks dynamically by Album, Artist, Genre, or Year via a sleek fold-out capsule control, and let the table gracefully shrink its own columns as you resize the window instead of turning into a wall of wrapped text.
- **Playlist Management.** Drag songs into the order you want, drag whole playlists around in the sidebar to prioritize your favorites, and add a song to any playlist right from the player controls — no need to dig through menus mid-listen.
- **Synced Lyrics.** Synced LRC lyrics scroll and highlight in time with the track, and you can click any line to jump straight to that moment in the song.
- **Listening Statistics & Leaderboards.** Complete rewrite of listening stats — a toolbar button opens a dashboard modal with Day, Week, Month, and All-Time tabs. View three-column leaderboards (Artists, Albums, Genres) ranking your top 10 by minutes played, complete with gold, silver, and bronze highlights and custom tier icons. Drill down into any leaderboard item to open a sub-window displaying the top 100 songs by play count, and receive persistent milestone toasts for daily listening counts (25/50/100 tracks), artist plays (50/100/500/1000), and genre milestones.
- **Audio Visualizer 2.0 & Motion Trails.** Real-time FFT spectrum analysis with 9 visually distinct canvas rendering modes (Mirrored Spectrograph with gravity-bouncing peak limiters, Radial Pulse, Liquid Ribbon, Particle Constellation web, Hyperdrive Depth Tunnel flight, Rolling Plains, Kaleidoscope Mirror, Cosmic Aurora plasma curtains, and Retro Synthwave Horizon). Features customizable visualizer background modes (Theme Default, Custom RGB with interactive sliders, Muted Reactive Pulse), mode-specific customization settings, random mode dice selector button (`\ue270`), decaying ghost motion trails, audio sensitivity, and spectrum color shifting via Settings.
- **10-Band Parametric Equalizer.** A real DSP engine inside the playback callback — ten ISO bands (31.5 Hz – 16 kHz, one octave each) built from RBJ peaking biquads with independent per-channel state. Gains ramp over 20 ms and bypass crossfades dry/wet, so nothing clicks; bypass is bit-exact when the EQ is off or flat. Includes 16 built-in presets (Flat, Loudness, Bass boost, Vocal, Rock, Jazz, Night and more), a −12…0 dB preamp with automatic headroom, a soft-clip knee so boosts stay clean, and a live frequency-response curve. Runs *before* the volume slider, and the visualizer taps what you actually hear.
- **Arabic & Full RTL Support.** Arabic, Hebrew, Syriac and other right-to-left scripts are shaped correctly everywhere in the UI — artist, album, title, genre and playlist names, not just lyrics. Ships Noto Sans Arabic (regular + bold), shapes with `Shaping::Advanced` so letters join properly, and applies a per-line direction guard so mixed Arabic/English text keeps the correct margin and reading order.
- **Theming System.** Follow your Omarchy system theme live, pick from built-in presets (Nord, Catppuccin, Dracula, Gruvbox, Everforest, Monokai), or build your own — omaTUNES derives the supporting shades automatically using proper WCAG contrast math, so your custom theme never ends up with unreadable text.
- **Bulk Metadata Editing.** Select a stack of tracks, check only the fields you want to change, and apply edits across an entire album in one go — with autocomplete pulled straight from your existing library tags.
- **Desktop & Quickshell Integration.** Full MPRIS2 D-Bus support, a ready-to-go Waybar module, and a native **Quickshell Player Module** featuring an interactive mini-player pop-up window with live timeline scrubbing, volume controls, liked status tracking, and active shuffle/repeat highlights.
- **Non-Destructive Library Handling.** No forced re-organization, no renaming your folders, no importing into some walled-off library format. omaTUNES reads your music exactly where it already lives.

---

## User Manual

The [USER_MANUAL.md](USER_MANUAL.md) covers everything in detail — every keybinding, every menu, how the database is structured, and how to get the Waybar integration looking sharp.

---

## System Requirements

| Requirement | Notes |
|---|---|
| Rust ≥ 1.75 | `rustup` is the easiest way to get it |
| A Nerd Font | Ships configured for `JetBrainsMono Nerd Font Mono`, but any Nerd Font works |
| PipeWire or PulseAudio | Audio output runs through cpal |
| D-Bus session bus | Needed for MPRIS2 — make sure `DBUS_SESSION_BUS_ADDRESS` is set |
| Wayland or X11 | Built and tested on Hyprland, but plays nicely on GNOME, KDE, and standard X11 window managers too |

---

## Install Instructions

### Arch Linux (PKGBUILD)

Two packages ship in [`packaging/`](packaging/):

```bash
git clone https://github.com/habdelrahman510-tech/Omatunes-Deluxe
cd Omatunes-Deluxe/packaging/omatunes-deluxe-bin   # prebuilt, fast
# or: cd Omatunes-Deluxe/packaging/omatunes-deluxe  # builds from source, runs tests
makepkg -si
```

`omatunes-deluxe-bin` installs the release binary; `omatunes-deluxe` compiles it locally and runs the test suite. Both install `/usr/bin/omatunes`, the desktop entry, the icon and the licence, and both declare `provides=omatunes`.

An AUR submission is being prepared but not yet published — new AUR account registration is currently paused, so the packages live here until that reopens.

### 1. Install the binary

**Easiest: grab a pre-built release.**
```bash
mkdir -p ~/.local/bin
curl -L -o ~/.local/bin/omatunes https://github.com/habdelrahman510-tech/Omatunes-Deluxe/releases/latest/download/omatunes
chmod +x ~/.local/bin/omatunes
```

**Prefer to build it yourself?**
```bash
git clone https://github.com/habdelrahman510-tech/Omatunes-Deluxe
cd omatunes
cargo build --release
mkdir -p ~/.local/bin
cp target/release/omatunes ~/.local/bin/omatunes
```

### 2. Install the Waybar module (optional)

If you want the Waybar integration — playback controls, track info, and listening stats right in your bar — copy the scripts over and make them executable:

```bash
mkdir -p ~/.local/bin/omatunes_scripts
cp scripts/omatunes_text.py ~/.local/bin/omatunes_scripts/omatunes_text.py
cp scripts/omatunes_volume.sh ~/.local/bin/omatunes_scripts/omatunes_volume.sh
chmod +x ~/.local/bin/omatunes_scripts/omatunes_text.py
chmod +x ~/.local/bin/omatunes_scripts/omatunes_volume.sh
```

Full Waybar config below, and CSS styling details in the [User Manual](USER_MANUAL.md#waybar-integration).

### 3. Install the Quickshell widget (optional)

On **Omarchy**, the bar widget ships as its own plugin repository, so the shell manages it:

```bash
omarchy plugin add https://github.com/habdelrahman510-tech/omarchy-omatunes --enable --yes
```

It installs as plugin id `omatunes` and keeps updating with `omarchy plugin update omatunes`.

If you use **standalone Quickshell**, run the bundled installer script to copy the widget and pop-up card files into place:

```bash
git clone https://github.com/habdelrahman510-tech/Omatunes-Deluxe.git
cd Omatunes-Deluxe
bash scripts/quickshell/install.sh
```

For full details on enabling the plugin in Omarchy's `shell.json` or standalone Quickshell's `shell.qml`, see the [Quickshell Player Module section](#quickshell-player-module--omarchy-v4-bar-plugin) below.

### 4. Add omaTUNES to your app launcher (optional)

Drop in a desktop entry and its icon, and omaTUNES shows up in your launcher — `Super + Space` on Omarchy, or the app grid on GNOME/KDE:

```bash
mkdir -p ~/.local/share/applications ~/.local/share/icons/hicolor/256x256/apps

# From a clone
cp scripts/omatunes.desktop ~/.local/share/applications/omatunes.desktop
cp assets/omatunes.png ~/.local/share/icons/hicolor/256x256/apps/omatunes.png

# Or straight from GitHub, if you installed the pre-built binary
curl -L -o ~/.local/share/applications/omatunes.desktop https://raw.githubusercontent.com/prof.dog/omatunes/master/scripts/omatunes.desktop
curl -L -o ~/.local/share/icons/hicolor/256x256/apps/omatunes.png https://raw.githubusercontent.com/prof.dog/omatunes/master/assets/omatunes.png
```

If the entry doesn't appear right away, log out and back in — some launchers only rescan `applications/` at session start.

The entry assumes `omatunes` is on your `PATH`. If you put the binary somewhere unusual, change `Exec=` to the full path.

---


## Waybar Integration

The Waybar setup uses a single `custom/omatunes` module that shows a music note icon, artist, and
track name in the bar, with a tooltip summarising the track and your available mouse controls.

Add this to `~/.config/waybar/config.jsonc`:

```jsonc
  "modules-left": [
    ...
    "custom/omatunes"
  ],

  "custom/omatunes": {
    "exec": "/home/yourname/.local/bin/omatunes_scripts/omatunes_text.py",
    "interval": 1,
    "return-type": "json",
    "format": "{}",
    "markup": "pango",
    "tooltip": true,
    "on-click": "/home/yourname/.local/bin/omatunes_scripts/omatunes_text.py --click play",
    "on-click-middle": "/home/yourname/.local/bin/omatunes_scripts/omatunes_text.py --click like",
    "on-click-right": "/home/yourname/.local/bin/omatunes_scripts/omatunes_text.py --click next",
    "on-scroll-up": "/home/yourname/.local/bin/omatunes_scripts/omatunes_volume.sh up",
    "on-scroll-down": "/home/yourname/.local/bin/omatunes_scripts/omatunes_volume.sh down"
  }
```

> Replace `/home/yourname/` with your actual home path — Waybar requires fully expanded paths.

For CSS styling, see the [Waybar Integration section](USER_MANUAL.md#waybar-integration) in the User Manual.

---

## Quickshell Player Module & Omarchy v4 Bar Plugin

omaTUNES includes a native **Quickshell Player Module** and an **Omarchy v4 Bar Plugin** with an interactive pop-up control card.

> **FYI for Omarchy v4 Users**: `install.sh` automatically copies the required `BarWidget.qml` & `manifest.json` files directly into `~/.config/omarchy/plugins/omatunes/`.

### Quick Install

```bash
# Clone or navigate to the repository
git clone https://github.com/habdelrahman510-tech/Omatunes-Deluxe.git
cd omatunes

# Run the automated installer for Quickshell & Omarchy v4
bash scripts/quickshell/install.sh
```
Then add "omatunes" to your bar layout configuration (~/.config/omarchy/shell.json or ~/.config/quickshell/shell.qml).

### Enabling the Module

#### For Omarchy v4 / Omarchy Shell Users (`~/.config/omarchy/shell.json`)
Open your `~/.config/omarchy/shell.json` configuration file and add `"omatunes"` to your desired panel bar section (e.g. `modules`, `modules-left`, `modules-center`, or `modules-right` depending on your layout style, such as Pure or Default):

```json
{
  "panel": {
    "modules-center": [
      "omatunes"
    ]
  }
}
```

> **Note**: Omarchy v4 plugin manifests auto-register the plugin named `omatunes` when installed via `install.sh`. Adding `"omatunes"` to any module array in your `shell.json` renders the bar widget.

#### For Standalone Quickshell Users (`~/.config/quickshell/shell.qml`)
If you use standard Quickshell without Omarchy, import the module directory and place `OmatunesWidget {}` into your bar layout:

```qml
import "./modules/omatunes"

// Inside your Bar / Panel component:
OmatunesWidget {}
```

### Module Features & Mouse Controls

- **Bar Widget Display**:
  - **Offline (OmaTUNES closed)**: Automatically hides from your bar (`visible: false`) to keep your panel clean.
  - **Playing / Open**: Appears on your bar with play state (`󰐊`/`󰏤`), `Artist - Song Title`, and a red heart icon (``) if liked.
  - **Left Click**: Play / Pause track.
  - **Right Click**: Toggle interactive mini-player pop-up window.
  - **Middle Click**: Toggle Like / Unlike for the current track.
  - **Scroll Wheel Up / Down**: Adjust volume smoothly.

- **Pop-up Mini-Player**:
  - **Album Cover**: High-res album art (75% width). Click artwork or title to bring the main omaTUNES window to the foreground.
  - **Transport Controls**: Shuffle (``), Previous (``), Play/Pause (``/``), Next (``), and Repeat (``). Active mode buttons illuminate in your system accent color.
  - **Volume Slider**: Interactive volume control bar.

---

## Keybindings

These fire whenever the omaTUNES window has focus:

| Key | Action |
|---|---|
| `Space` | Play / Pause |
| `→` / `←` | Seek +5s / −5s |
| `n` / `p` | Next / Previous track |
| `s` | Toggle Shuffle |
| `r` | Toggle Repeat |
| `+` or `=` | Volume +5% |
| `-` | Volume −5% |
| `E` | Edit metadata for the selected track(s) |

The full list — including focus navigation and dialog shortcuts — is in the [Keybinding Reference](USER_MANUAL.md#keybinding-reference) in the User Manual.

---

## License

MIT — see [LICENSE](LICENSE).

Copyright (c) 2026 prof.dog
Copyright (c) 2026 Balthazzahr and omaTUNES contributors
Copyright (c) 2026 Flavio de Vasconcellos
