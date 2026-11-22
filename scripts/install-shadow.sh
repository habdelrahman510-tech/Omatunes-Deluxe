#!/usr/bin/env bash
# Install this checkout's release binary where it outranks the distro one.
#
# The Omarchy plugin starts the player with `omatunes --background`, resolved
# through PATH. On this machine /usr/local/bin comes before /usr/bin, so a copy
# there keeps our MPRIS fixes alive when `pacman -Syu` replaces the packaged
# /usr/bin/omatunes with an older build. Run this again after rebuilding from
# a newer checkout.
set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

cargo build --release --manifest-path "$repo/Cargo.toml"

if (( EUID == 0 )); then
  install -Dm755 "$repo/target/release/omatunes" /usr/local/bin/omatunes
else
  pkexec install -Dm755 "$repo/target/release/omatunes" /usr/local/bin/omatunes
fi

printf 'Installed %s to /usr/local/bin/omatunes\n' "$repo/target/release/omatunes"
