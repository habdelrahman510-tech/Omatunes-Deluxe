//! M3U/M3U8 playlist import.
//!
//! Deliberately small: a playlist file is a list of paths, one per line, with
//! `#`-prefixed directives in between. We only care about the paths; durations
//! and titles from `#EXTINF` are recomputed from the files themselves when the
//! scanner later reads their tags.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::scanner::is_audio_file;

/// Parse an `.m3u`/`.m3u8` file into absolute, existing audio paths.
///
/// Order is preserved and duplicates are dropped. Relative entries resolve
/// against the playlist's own directory, which is what every player does and
/// what makes a playlist survive being moved together with its music folder.
pub fn parse_m3u(path: &Path) -> Result<Vec<PathBuf>> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let base = path.parent().unwrap_or_else(|| Path::new("."));

    // m3u8 is UTF-8 by definition; classic m3u is whatever the author's locale
    // was. Lossy decoding keeps the readable part instead of failing outright.
    let text = String::from_utf8_lossy(&bytes);
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);

    let mut entries: Vec<PathBuf> = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some(candidate) = resolve_entry(line, base) else {
            continue;
        };
        if candidate.is_file() && is_audio_file(&candidate) && !entries.contains(&candidate) {
            entries.push(candidate);
        }
    }

    Ok(entries)
}

fn resolve_entry(line: &str, base: &Path) -> Option<PathBuf> {
    // Some writers percent-encode or prefix with file://.
    let unquoted = line.trim_matches('"');
    let without_scheme = unquoted.strip_prefix("file://").unwrap_or(unquoted);
    let decoded = percent_decode(without_scheme);

    let candidate = Path::new(&decoded);
    if candidate.is_absolute() {
        Some(candidate.to_path_buf())
    } else {
        // A relative entry that still includes the playlist's own folder name
        // would double up; joining is correct for the common case.
        Some(base.join(candidate))
    }
}

fn percent_decode(input: &str) -> String {
    if !input.contains('%') {
        return input.to_string();
    }
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(value) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(value);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_playlist(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        path
    }

    fn touch(path: &Path) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, b"not really audio").unwrap();
    }

    #[test]
    fn parses_relative_entries_against_the_playlist_dir() {
        let dir = std::env::temp_dir().join(format!("omatunes-m3u-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        touch(&dir.join("album/one.flac"));
        touch(&dir.join("album/two.mp3"));

        let list = write_playlist(
            &dir,
            "mix.m3u8",
            "\u{feff}#EXTM3U\n#EXTINF:1,Artist - One\nalbum/one.flac\n\nalbum/two.mp3\nalbum/one.flac\n",
        );

        let entries = parse_m3u(&list).unwrap();
        assert_eq!(entries.len(), 2, "duplicate entries are dropped");
        assert_eq!(entries[0], dir.join("album/one.flac"));
        assert_eq!(entries[1], dir.join("album/two.mp3"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn skips_missing_files_and_directives() {
        let dir = std::env::temp_dir().join(format!("omatunes-m3u-skip-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        touch(&dir.join("real.ogg"));

        let list = write_playlist(
            &dir,
            "partial.m3u",
            "#EXTM3U\nmissing.flac\nreal.ogg\nnotes.txt\n",
        );

        let entries = parse_m3u(&list).unwrap();
        assert_eq!(entries, vec![dir.join("real.ogg")]);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn decodes_file_uris() {
        let dir = std::env::temp_dir().join(format!("omatunes-m3u-uri-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let song = dir.join("space name.flac");
        touch(&song);

        let uri = format!("file://{}", song.display()).replace(' ', "%20");
        let list = write_playlist(&dir, "uris.m3u", &format!("{uri}\n"));

        let entries = parse_m3u(&list).unwrap();
        assert_eq!(entries, vec![song]);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
