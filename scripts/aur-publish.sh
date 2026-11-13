#!/usr/bin/env bash
# Push packaging/<pkgbase>/ to the AUR.
#
# The AUR repository for a package holds exactly two tracked files: PKGBUILD
# and .SRCINFO. This copies them out of packaging/, regenerates .SRCINFO so it
# can never drift from the PKGBUILD, and pushes. Run it after a version bump:
#
#   ./scripts/aur-publish.sh            # both packages
#   ./scripts/aur-publish.sh -bin       # only omatunes-deluxe-bin
#
# Requires an AUR account with an SSH key registered:
#   https://aur.archlinux.org -> My Account -> SSH Public Key
set -euo pipefail

REPO_ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
WORKDIR="${AUR_WORKDIR:-$HOME/.cache/omatunes-aur}"

ALL_PACKAGES=("omatunes-deluxe-bin" "omatunes-deluxe")
PACKAGES=()

for arg in "$@"; do
  case "$arg" in
    -bin) PACKAGES+=("omatunes-deluxe-bin") ;;
    src | source) PACKAGES+=("omatunes-deluxe") ;;
    -h | --help)
      sed -n '2,12p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *)
      echo "unknown argument: $arg" >&2
      exit 2
      ;;
  esac
done

[ ${#PACKAGES[@]} -gt 0 ] || PACKAGES=("${ALL_PACKAGES[@]}")

if ! ssh -o BatchMode=yes -o StrictHostKeyChecking=accept-new \
  aur@aur.archlinux.org help >/dev/null 2>&1; then
  cat >&2 <<'EOF'
Cannot authenticate to aur.archlinux.org.

Register a key first:
  1. ssh-keygen -t ed25519 -f ~/.ssh/id_ed25519      (if you have none)
  2. cat ~/.ssh/id_ed25519.pub
  3. Paste it at https://aur.archlinux.org -> My Account -> SSH Public Key
  4. Wait a minute, then run this script again.
EOF
  exit 1
fi

mkdir -p "$WORKDIR"

for pkg in "${PACKAGES[@]}"; do
  src="$REPO_ROOT/packaging/$pkg"
  if [ ! -d "$src" ]; then
    echo "missing packaging/$pkg" >&2
    exit 1
  fi

  dir="$WORKDIR/$pkg"
  if [ ! -d "$dir/.git" ]; then
    echo "==> cloning ssh://aur@aur.archlinux.org/$pkg.git"
    git clone -q "ssh://aur@aur.archlinux.org/$pkg.git" "$dir"
  else
    git -C "$dir" pull -q --ff-only || true
  fi

  cp "$src/PKGBUILD" "$dir/PKGBUILD"
  if [ -f "$src/omatunes.install" ]; then
    cp "$src/omatunes.install" "$dir/omatunes.install"
  fi

  # Regenerate inside the AUR checkout: the AUR compares .SRCINFO against the
  # PKGBUILD and refuses a push where they disagree.
  ( cd "$dir" && makepkg --printsrcinfo >.SRCINFO )

  ver=$(sed -n 's/^pkgver=//p' "$dir/PKGBUILD")
  rel=$(sed -n 's/^pkgrel=//p' "$dir/PKGBUILD")

  if [ -z "$(git -C "$dir" status --porcelain)" ]; then
    echo "==> $pkg already at $ver-$rel"
    continue
  fi

  git -C "$dir" add PKGBUILD .SRCINFO
  [ -f "$dir/omatunes.install" ] && git -C "$dir" add omatunes.install
  git -C "$dir" commit -q -m "upgpkg: $pkg $ver-$rel"
  git -C "$dir" push -q origin master
  echo "==> published $pkg $ver-$rel"
done

echo
echo "Install with:  yay -S omatunes-deluxe-bin"
echo "Build locally: yay -S omatunes-deluxe"
