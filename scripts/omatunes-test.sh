#!/usr/bin/env bash
set -euo pipefail
# omatunes-test — run omaTUNES against throwaway state, never your real library.
#
# The point is to exercise a build without touching ~/.config/omatunes,
# ~/.local/share/omatunes or your real database: HOME, XDG_CONFIG_HOME,
# XDG_DATA_HOME and XDG_CACHE_HOME are all redirected into a scratch tree under
# XDG_RUNTIME_DIR, and the config is generated fresh with your real music
# folder so the scan still finds something to play.
#
#   ./scripts/omatunes-test.sh [file...]
#   OMATUNES_BIN=/path/to/binary ./scripts/omatunes-test.sh
#   OMATUNES_MUSIC_DIR=~/Music/Test ./scripts/omatunes-test.sh
#
# Stop the real player first: the player is single-instance by design (one
# control socket on 127.0.0.1:18888, one org.mpris.MediaPlayer2.omatunes name),
# so a second copy would run with a dead socket and no MPRIS. Set
# OMATUNES_TEST_FORCE=1 to run anyway.

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
REPO_ROOT=$(cd "$SCRIPT_DIR/.." && pwd)

# Prefer an explicit override, then the release build, then whatever is in PATH.
if [ -n "${OMATUNES_BIN:-}" ]; then
  BIN="$OMATUNES_BIN"
elif [ -x "$REPO_ROOT/target/release/omatunes" ]; then
  BIN="$REPO_ROOT/target/release/omatunes"
elif [ -x "$REPO_ROOT/target/debug/omatunes" ]; then
  BIN="$REPO_ROOT/target/debug/omatunes"
else
  BIN=$(command -v omatunes || true)
fi
if [ -z "$BIN" ] || [ ! -x "$BIN" ]; then
  echo "no omatunes binary found — build one (cargo build --release) or set OMATUNES_BIN" >&2
  exit 1
fi

if pgrep -x omatunes >/dev/null 2>&1 && [ -z "${OMATUNES_TEST_FORCE:-}" ]; then
  cat >&2 <<'EOF'
omatunes is already running. The test copy would fail to claim the control
socket and the MPRIS name, so this test would prove nothing.

  pkill -x omatunes      # or close the window

Set OMATUNES_TEST_FORCE=1 to launch anyway.
EOF
  exit 1
fi

TEST_ROOT="${XDG_RUNTIME_DIR:-/tmp}/omatunes-test-$$"
# Preserve real HOME for browser mime lookup (so xdg-open still finds a browser)
export REAL_HOME="$HOME"
export HOME="$TEST_ROOT/home"
export XDG_CONFIG_HOME="$TEST_ROOT/config"
export XDG_DATA_HOME="$TEST_ROOT/data"
export XDG_CACHE_HOME="$TEST_ROOT/cache"
# Copy mimeapps so xdg-mime / gio still resolve the same defaults in the isolated HOME
mkdir -p "$XDG_CONFIG_HOME"
if [ -f "$REAL_HOME/.config/mimeapps.list" ]; then
  cp "$REAL_HOME/.config/mimeapps.list" "$XDG_CONFIG_HOME/mimeapps.list" 2>/dev/null || true
  mkdir -p "$HOME/.config"
  cp "$REAL_HOME/.config/mimeapps.list" "$HOME/.config/mimeapps.list" 2>/dev/null || true
fi
if [ -f "$REAL_HOME/.local/share/applications/mimeapps.list" ]; then
  mkdir -p "$XDG_DATA_HOME/applications"
  cp "$REAL_HOME/.local/share/applications/mimeapps.list" "$XDG_DATA_HOME/applications/mimeapps.list" 2>/dev/null || true
fi
# GPU prime offload — match the live Hyprland env so wgpu picks the dGPU, not the iGPU
export VK_DRIVER_FILES=/usr/share/vulkan/icd.d/nvidia_icd.json
export __NV_PRIME_RENDER_OFFLOAD=1
export __GLX_VENDOR_LIBRARY_NAME=nvidia
# wgpu backend fallback (also set in src/main.rs if unset, but export here for child processes)
if [ -z "${WGPU_BACKEND:-}" ]; then
  export WGPU_BACKEND="gl,vulkan"
fi
mkdir -p "$HOME/.config/omatunes" "$XDG_CONFIG_HOME/omatunes" "$XDG_DATA_HOME" "$XDG_CACHE_HOME"
# Pre-create config with the real music_dir so the scan finds a real library
# (not the empty $HOME/Music of the isolated tree).
MUSIC_DIR="${OMATUNES_MUSIC_DIR:-$REAL_HOME/Music}"
if [ ! -f "$HOME/.config/omatunes/config.toml" ]; then
  cat > "$HOME/.config/omatunes/config.toml" << EOF
music_dir = "$MUSIC_DIR"
music_dirs = ["$MUSIC_DIR"]
volume = 0.8
language = "auto"
seek_step = 5
volume_step = 0.05
theme_source = "System"
theme_preset = "Nord"
EOF
fi
echo "omatunes-test isolated dirs:"
echo "  HOME=$HOME"
echo "  XDG_CONFIG_HOME=$XDG_CONFIG_HOME"
echo "  XDG_DATA_HOME=$XDG_DATA_HOME"
echo "  XDG_CACHE_HOME=$XDG_CACHE_HOME"
echo "  music_dir=$MUSIC_DIR"
echo "  VK_DRIVER_FILES=$VK_DRIVER_FILES __NV_PRIME_RENDER_OFFLOAD=$__NV_PRIME_RENDER_OFFLOAD __GLX_VENDOR_LIBRARY_NAME=$__GLX_VENDOR_LIBRARY_NAME WGPU_BACKEND=$WGPU_BACKEND"
echo "  binary=$BIN"
echo "  real config untouched: ~/.config/omatunes not read (HOME overridden)"
exec "$BIN" "$@"
