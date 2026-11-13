#!/usr/bin/env bash
# Publish a built package to the GitHub Pages pacman repository.
#
#   ./scripts/publish-repo.sh                       # build packaging/omatunes-bin, then publish
#   ./scripts/publish-repo.sh path/to/pkg.tar.zst   # publish an already built package
#
# The repository is https://github.com/habdelrahman510-tech/omatunes-repo and is
# served at https://habdelrahman510-tech.github.io/omatunes-repo.
#
# GitHub Pages does not follow symlinks, and repo-add renders <repo>.db as a
# symlink to <repo>.db.tar.gz. Both names are therefore stored as real files:
# a symlinked database would 404 for every pacman client.
set -euo pipefail

REPO_ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
WORKDIR="${OMATUNES_REPO_WORKDIR:-$HOME/.cache/omatunes-repo}"
REMOTE="https://github.com/habdelrahman510-tech/omatunes-repo.git"
PKGBASE="omatunes-bin"
SITE="https://habdelrahman510-tech.github.io/omatunes-repo"

pkg_path="${1:-}"

if [ -z "$pkg_path" ]; then
  build_dir=$(mktemp -d "${TMPDIR:-/tmp}/omatunes-pkg.XXXXXX")
  trap 'rm -rf "$build_dir"' EXIT
  cp "$REPO_ROOT/packaging/$PKGBASE/PKGBUILD" "$build_dir/PKGBUILD"
  echo "==> building $PKGBASE"
  ( cd "$build_dir" && makepkg -f >/dev/null )
  pkg_path=$(find "$build_dir" -maxdepth 1 -name '*.pkg.tar.zst' -print -quit)
fi

if [ ! -f "$pkg_path" ]; then
  echo "no package found at: $pkg_path" >&2
  exit 1
fi

pkg_file=$(basename "$pkg_path")
case "$pkg_file" in
  "$PKGBASE"-*.pkg.tar.zst) ;;
  *)
    echo "unexpected package name: $pkg_file (expected $PKGBASE-*.pkg.tar.zst)" >&2
    exit 1
    ;;
esac

if [ -d "$WORKDIR/.git" ]; then
  git -C "$WORKDIR" pull -q --ff-only
else
  echo "==> cloning $REMOTE"
  git clone -q "$REMOTE" "$WORKDIR"
fi

git -C "$WORKDIR" config user.name "$(git -C "$REPO_ROOT" config user.name || echo 'prof. dog')"
git -C "$WORKDIR" config user.email "$(git -C "$REPO_ROOT" config user.email || echo 'prof.dog@users.noreply.github.com')"

dest="$WORKDIR/x86_64"
mkdir -p "$dest"

if [ -f "$dest/$pkg_file" ] && cmp -s "$pkg_path" "$dest/$pkg_file"; then
  echo "==> $pkg_file is already published"
  exit 0
fi

cp "$pkg_path" "$dest/"

# Pages has a 1 GB soft limit and every release is ~12 MB. Keep the newest
# three packages so an upgrade can still roll back a version.
mapfile -t old < <(ls -1t "$dest"/"$PKGBASE"-*.pkg.tar.zst | tail -n +4)
if [ ${#old[@]} -gt 0 ]; then
  echo "==> pruning ${#old[@]} old package(s)"
  printf '    %s\n' "${old[@]##*/}"
  rm -f "${old[@]}"
fi

# Rebuild the database from the packages on disk rather than patching it
# incrementally: that keeps the index and the directory in step, and avoids
# relying on repo-remove's per-version argument parsing.
echo "==> rebuilding the database"
rm -f "$dest/$PKGBASE.db.tar.gz" "$dest/$PKGBASE.files.tar.gz" \
  "$dest/$PKGBASE.db" "$dest/$PKGBASE.files"
( cd "$dest" && repo-add -q "$PKGBASE.db.tar.gz" "$PKGBASE"-*.pkg.tar.zst )

# Real copies, not the symlinks repo-add leaves behind: Pages will not serve them.
rm -f "$dest/$PKGBASE.db" "$dest/$PKGBASE.files"
cp "$dest/$PKGBASE.db.tar.gz" "$dest/$PKGBASE.db"
cp "$dest/$PKGBASE.files.tar.gz" "$dest/$PKGBASE.files"

ver=$(sed -n 's/^pkgver=//p' "$REPO_ROOT/packaging/$PKGBASE/PKGBUILD")
rel=$(sed -n 's/^pkgrel=//p' "$REPO_ROOT/packaging/$PKGBASE/PKGBUILD")

if [ -z "$(git -C "$WORKDIR" status --porcelain)" ]; then
  echo "==> $PKGBASE $ver-$rel already published"
  exit 0
fi

git -C "$WORKDIR" add -A
git -C "$WORKDIR" commit -q -m "repo: $PKGBASE $ver-$rel"
git -C "$WORKDIR" push -q origin main
echo "==> published $PKGBASE $ver-$rel"
echo "    $SITE/x86_64"
echo "    update with: sudo pacman -Syu"
