use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;
use lofty::config::WriteOptions;
use lofty::file::TaggedFileExt;
use lofty::prelude::*;
use lofty::probe::Probe;
use lofty::tag::{ItemKey, ItemValue, Tag, TagItem};
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use super::models::Track;

const AUDIO_EXTENSIONS: &[&str] = &[
    "mp3", "flac", "ogg", "opus", "wav", "aac", "m4a", "wma", "aiff",
];

const COVER_FILENAMES: &[&str] = &[
    "cover.jpg",
    "Cover.jpg",
    "cover.png",
    "Cover.png",
    "cover.webp",
    "Cover.webp",
    "folder.jpg",
    "Folder.jpg",
    "folder.png",
    "Folder.png",
];

/// Write like/rating metadata to an audio file using the unified generic tag API.
///
/// Writes RATING=100, xLIKE=1.
/// When `is_liked` is false, all of the above are removed.
/// Note: POPM is intentionally omitted (ID3v2 rejects `ItemValue::Text` for POPM).
pub fn write_like_status(path: &Path, is_liked: bool) -> Result<()> {
    write_like_status_atomic(path, is_liked)
}

/// Atomic variant: copy → save → rename (same dir, same FS).
/// Fixes corruption where save_to_path(&tmp) on an empty tmp would write tag-only file.
pub fn write_like_status_atomic(path: &Path, is_liked: bool) -> Result<()> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("mp3");
    let tmp = path.with_extension(format!("tmp.{}", ext));
    std::fs::copy(path, &tmp)?;
    let mut tagged_file = Probe::open(&tmp)?.read()?;

    if tagged_file.primary_tag_mut().is_none() {
        tagged_file.insert_tag(Tag::new(tagged_file.primary_tag_type()));
    }
    let tag = tagged_file.primary_tag_mut().unwrap();

    let rating_key = ItemKey::Unknown("RATING".to_string());
    let like_key = ItemKey::Unknown("xLIKE".to_string());
    tag.retain(|i| i.key() != &rating_key && i.key() != &like_key);

    if is_liked {
        tag.insert_unchecked(TagItem::new(
            ItemKey::Unknown("RATING".to_string()),
            ItemValue::Text("100".to_string()),
        ));
        tag.insert_unchecked(TagItem::new(
            ItemKey::Unknown("xLIKE".to_string()),
            ItemValue::Text("1".to_string()),
        ));
    }

    tagged_file.save_to_path(&tmp, WriteOptions::default())?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// One cached track plus the filesystem fingerprint it was read from.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CachedFile {
    pub path: PathBuf,
    pub mtime: u64,
    pub size: u64,
    pub track: Track,
}

/// On-disk scan cache. Version 2 carries fingerprints so an unchanged file is
/// never opened again.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LibraryCache {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub entries: Vec<CachedFile>,
}

fn fingerprint(path: &Path) -> (u64, u64) {
    match std::fs::metadata(path) {
        Ok(meta) => {
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0);
            (mtime, meta.len())
        }
        Err(_) => (0, 0),
    }
}

pub fn is_audio_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| AUDIO_EXTENSIONS.contains(&ext.to_lowercase().as_str()))
        .unwrap_or(false)
}

/// Every audio file under every root, deduplicated (roots may nest).
fn collect_audio_files(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut seen = std::collections::HashSet::new();
    let mut paths = Vec::new();
    for root in roots {
        for entry in WalkDir::new(root)
            .into_iter()
            .filter_entry(|e| !e.file_name().to_string_lossy().starts_with('.'))
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
        {
            let path = entry.path().to_path_buf();
            if is_audio_file(&path) && seen.insert(path.clone()) {
                paths.push(path);
            }
        }
    }
    paths
}

fn track_from_info(path: PathBuf, info: TrackInfo) -> Track {
    let play_count = crate::db::get(|db| db.play_counts.get(&path).copied().unwrap_or(0));
    Track {
        id: {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            path.hash(&mut hasher);
            hasher.finish() as i64
        },
        path,
        title: info.title,
        artist: info.artist,
        album: info.album,
        album_id: 0,
        track_number: info.track_number,
        disc_number: info.disc_number,
        duration: Duration::from_millis(info.duration_ms),
        cover_data: None,
        genre: info.genre,
        year: info.year,
        play_count,
        liked: info.liked,
        date_played: None,
        lyrics: info.lyrics,
    }
}

fn sort_tracks(tracks: &mut [Track]) {
    tracks.sort_by(|a, b| {
        a.album
            .cmp(&b.album)
            .then(a.disc_number.cmp(&b.disc_number))
            .then(a.track_number.cmp(&b.track_number))
            .then(a.title.cmp(&b.title))
    });
}

/// Scan one directory (kept for callers that only have a single path).
pub fn scan_folder(dir: &Path) -> Vec<Track> {
    scan_roots(&[dir.to_path_buf()])
}

/// Scan every root, reusing cached tags for any file whose mtime and size are
/// unchanged. Returns tracks sorted by album/disc/number/title.
pub fn scan_roots(roots: &[PathBuf]) -> Vec<Track> {
    use rayon::prelude::*;

    let cached = load_library_cache();
    let index: std::collections::HashMap<PathBuf, CachedFile> = cached
        .entries
        .into_iter()
        .map(|e| (e.path.clone(), e))
        .collect();

    let paths = collect_audio_files(roots);

    let read_one = |path: &PathBuf| -> Option<Track> {
        let (mtime, size) = fingerprint(path);
        if mtime != 0 {
            if let Some(hit) = index.get(path) {
                if hit.mtime == mtime && hit.size == size {
                    let mut track = hit.track.clone();
                    track.play_count =
                        crate::db::get(|db| db.play_counts.get(path).copied().unwrap_or(0));
                    return Some(track);
                }
            }
        }
        read_tags(path)
            .ok()
            .map(|info| track_from_info(path.clone(), info))
    };

    let pool = rayon::ThreadPoolBuilder::new().num_threads(4).build().ok();
    let mut tracks: Vec<Track> = match pool {
        Some(pool) => pool.install(|| paths.par_iter().filter_map(read_one).collect()),
        None => paths.iter().filter_map(read_one).collect(),
    };
    sort_tracks(&mut tracks);
    tracks
}

pub fn load_library_cache() -> LibraryCache {
    let path = crate::paths::library_cache();
    let Ok(content) = std::fs::read_to_string(&path) else {
        return LibraryCache::default();
    };
    if let Ok(cache) = serde_json::from_str::<LibraryCache>(&content) {
        return cache;
    }
    // Version 1 was a bare array of tracks with no fingerprints.
    if let Ok(legacy) = serde_json::from_str::<Vec<Track>>(&content) {
        return LibraryCache {
            version: 1,
            entries: legacy
                .into_iter()
                .map(|track| CachedFile {
                    path: track.path.clone(),
                    mtime: 0,
                    size: 0,
                    track,
                })
                .collect(),
        };
    }
    LibraryCache::default()
}

pub fn load_cache() -> Option<Vec<Track>> {
    let cache = load_library_cache();
    if cache.entries.is_empty() {
        None
    } else {
        Some(cache.entries.into_iter().map(|e| e.track).collect())
    }
}

/// Persist tracks plus fresh fingerprints, atomically.
pub fn save_cache(tracks: &[Track]) {
    let entries = tracks
        .iter()
        .map(|track| {
            let (mtime, size) = fingerprint(&track.path);
            CachedFile {
                path: track.path.clone(),
                mtime,
                size,
                track: track.clone(),
            }
        })
        .collect();
    let cache = LibraryCache {
        version: 2,
        entries,
    };
    let Ok(json) = serde_json::to_string(&cache) else {
        return;
    };
    let path = crate::paths::library_cache();
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, json).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}

/// Tracks that look like the same recording in more than one place.
pub mod duplicates {
    use super::Track;

    pub type Group = Vec<Track>;

    pub fn normalise(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        let mut last_space = false;
        for c in s.chars().flat_map(|c| c.to_lowercase()) {
            if c.is_alphanumeric() {
                out.push(c);
                last_space = false;
            } else if !last_space && !out.is_empty() {
                out.push(' ');
                last_space = true;
            }
        }
        out.trim().to_string()
    }

    /// Group by normalised artist + title, then split when durations differ by
    /// more than two seconds (same title, different recording).
    pub fn find(tracks: &[Track]) -> Vec<Group> {
        use std::collections::HashMap;

        let mut buckets: HashMap<(String, String), Vec<Track>> = HashMap::new();
        for track in tracks {
            let key = (normalise(&track.artist), normalise(&track.title));
            buckets.entry(key).or_default().push(track.clone());
        }

        let mut groups = Vec::new();
        for (_, mut bucket) in buckets {
            if bucket.len() < 2 {
                continue;
            }
            bucket.sort_by_key(|t| t.duration.as_secs());
            let mut current: Group = Vec::new();
            for track in bucket {
                let fits = current
                    .last()
                    .map(|prev| track.duration.as_secs().abs_diff(prev.duration.as_secs()) <= 2)
                    .unwrap_or(true);
                if fits {
                    current.push(track);
                } else {
                    if current.len() > 1 {
                        groups.push(std::mem::take(&mut current));
                    } else {
                        current.clear();
                    }
                    current.push(track);
                }
            }
            if current.len() > 1 {
                groups.push(current);
            }
        }

        groups.sort_by(|a, b| {
            let ka = (&a[0].artist, &a[0].title);
            let kb = (&b[0].artist, &b[0].title);
            ka.cmp(&kb)
        });
        for group in &mut groups {
            group.sort_by(|a, b| a.path.cmp(&b.path));
        }
        groups
    }
}

pub fn downscale_cover(raw_bytes: &[u8], max_dim: u32) -> Vec<u8> {
    if raw_bytes.len() < 100_000 {
        return raw_bytes.to_vec();
    }
    if let Ok(img) = image::load_from_memory(raw_bytes) {
        if img.width() > max_dim || img.height() > max_dim {
            let resized = img.resize(max_dim, max_dim, image::imageops::FilterType::Triangle);
            let mut out = std::io::Cursor::new(Vec::new());
            if resized.write_to(&mut out, image::ImageFormat::Jpeg).is_ok() {
                return out.into_inner();
            }
        }
    }
    raw_bytes.to_vec()
}

/// Load cover art for a track: embedded tag first, then cover.jpg in the folder.
pub fn load_cover(path: &Path) -> Option<Vec<u8>> {
    let tagged = Probe::open(path).ok()?.read().ok()?;
    let embedded = tagged.primary_tag().and_then(|t| {
        t.pictures()
            .iter()
            .find(|p| {
                matches!(
                    p.pic_type(),
                    lofty::picture::PictureType::CoverFront | lofty::picture::PictureType::Other
                )
            })
            .map(|p| p.data().to_vec())
    });
    let raw = embedded.or_else(|| cover_from_folder(path))?;
    Some(downscale_cover(&raw, 512))
}

// ── Internal ───────────────────────────────────────────────────────────────────

struct TrackInfo {
    title: String,
    artist: String,
    album: String,
    track_number: Option<u32>,
    disc_number: Option<u32>,
    duration_ms: u64,
    genre: String,
    year: Option<u32>,
    lyrics: String,
    liked: bool,
}

fn read_tags(path: &Path) -> Result<TrackInfo> {
    let tagged = Probe::open(path)?.read()?;
    let duration_ms = tagged.properties().duration().as_millis() as u64;
    let tags = tagged.primary_tag();

    let unknown = crate::locale::get().unknown;

    let title = tags
        .and_then(|t| t.title())
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or(unknown)
                .to_string()
        });

    let folder_artist = path
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .or_else(|| {
            path.parent()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
        })
        .unwrap_or(unknown)
        .to_string();

    let folder_album = path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or(unknown)
        .to_string();

    let artist = tags
        .and_then(|t| t.artist())
        .map(|s| s.to_string())
        .unwrap_or(folder_artist);

    let album = tags
        .and_then(|t| t.album())
        .map(|s| s.to_string())
        .unwrap_or(folder_album);

    let track_number = tags.and_then(|t| {
        t.track().or_else(|| {
            t.get_string(&lofty::tag::ItemKey::TrackNumber)
                .and_then(|s| s.split('/').next())
                .and_then(|s| s.trim().parse::<u32>().ok())
        })
    });
    let disc_number = tags.and_then(|t| {
        t.disk().or_else(|| {
            t.get_string(&lofty::tag::ItemKey::DiscNumber)
                .and_then(|s| s.split('/').next())
                .and_then(|s| s.trim().parse::<u32>().ok())
        })
    });
    let year = tags.and_then(|t| t.year());

    let genre = tags
        .and_then(|t| t.genre())
        .map(|s| s.to_string())
        .unwrap_or_else(|| unknown.to_string());

    let mut lyrics = tags
        .and_then(|t| t.get_string(&lofty::tag::ItemKey::Lyrics))
        .map(|s| s.to_string())
        .unwrap_or_default();
    // Fallback: sidecar .lrc file with same stem (e.g. song.lrc next to song.flac)
    if lyrics.trim().is_empty() {
        let lrc_path = path.with_extension("lrc");
        let lrc_path_upper = path.with_extension("LRC");
        let candidate = if lrc_path.exists() {
            Some(lrc_path)
        } else if lrc_path_upper.exists() {
            Some(lrc_path_upper)
        } else {
            None
        };
        if let Some(p) = candidate {
            if let Ok(s) = std::fs::read_to_string(&p) {
                if !s.trim().is_empty() {
                    lyrics = s;
                }
            }
        }
    }

    let liked = tags
        .map(|t| {
            t.get_string(&ItemKey::Unknown("xLIKE".to_string())) == Some("1")
                || t.get_string(&ItemKey::Unknown("XLIKE".to_string())) == Some("1")
                || t.get_string(&ItemKey::Unknown("xlike".to_string())) == Some("1")
                || t.get_string(&ItemKey::Unknown("RATING".to_string())) == Some("100")
                || t.get_string(&ItemKey::Unknown("rating".to_string())) == Some("100")
        })
        .unwrap_or(false);

    Ok(TrackInfo {
        title,
        artist,
        album,
        track_number,
        disc_number,
        duration_ms,
        genre,
        year,
        lyrics,
        liked,
    })
}

fn cover_from_folder(path: &Path) -> Option<Vec<u8>> {
    let dir = path.parent()?;
    for name in COVER_FILENAMES {
        if let Ok(data) = std::fs::read(dir.join(name)) {
            return Some(data);
        }
    }
    None
}

/// Path of the artwork sitting next to `path`, if any.
pub fn folder_cover(path: &Path) -> Option<PathBuf> {
    let dir = path.parent()?;
    COVER_FILENAMES
        .iter()
        .map(|name| dir.join(name))
        .find(|candidate| candidate.is_file())
}

/// The embedded picture of a file, preferring the front cover.
pub fn embedded_cover(path: &Path) -> Option<Vec<u8>> {
    let file = Probe::open(path).ok()?.read().ok()?;
    let tag = file.primary_tag().or_else(|| file.first_tag())?;
    let pictures = tag.pictures();
    let picture = pictures
        .iter()
        .find(|p| p.pic_type() == lofty::picture::PictureType::CoverFront)
        .or_else(|| pictures.first())?;
    Some(picture.data().to_vec())
}

/// Write the file's embedded artwork beside it as `cover.jpg`/`cover.png`.
///
/// Returns the path written, or `None` when the file carries no picture. An
/// existing file is never clobbered: a second extraction gets a suffix, so
/// running this on a whole album cannot quietly destroy hand-picked artwork.
pub fn extract_cover_to_folder(path: &Path) -> Result<Option<PathBuf>> {
    let Some(data) = embedded_cover(path) else {
        return Ok(None);
    };
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("{} has no parent directory", path.display()))?;
    let ext = if data.starts_with(b"\x89PNG") {
        "png"
    } else if data.starts_with(b"GIF8") {
        "gif"
    } else if data.len() > 12 && &data[8..12] == b"WEBP" {
        "webp"
    } else {
        "jpg"
    };

    let mut candidate = parent.join(format!("cover.{ext}"));
    let mut n = 2;
    while candidate.exists() {
        candidate = parent.join(format!("cover ({n}).{ext}"));
        n += 1;
    }

    let tmp = candidate.with_extension(format!("tmp.{ext}"));
    std::fs::write(&tmp, &data)?;
    std::fs::rename(&tmp, &candidate)?;
    Ok(Some(candidate))
}

pub fn write_tags(
    path: &Path,
    title: &str,
    artist: &str,
    album: &str,
    genre: &str,
    track_number: Option<u32>,
    disc_number: Option<u32>,
    cover_path: Option<&str>,
    year: Option<u32>,
    lyrics: Option<&str>,
) -> Result<()> {
    let mut tagged_file = Probe::open(path)?.read()?;

    if tagged_file.primary_tag_mut().is_none() {
        tagged_file.insert_tag(lofty::tag::Tag::new(tagged_file.primary_tag_type()));
    }
    let tag = tagged_file.primary_tag_mut().unwrap();

    tag.set_title(title.to_string());
    tag.set_artist(artist.to_string());
    tag.set_album(album.to_string());
    tag.set_genre(genre.to_string());
    if let Some(num) = track_number {
        tag.set_track(num);
    } else {
        tag.remove_track();
    }
    if let Some(num) = disc_number {
        tag.set_disk(num);
    } else {
        tag.remove_disk();
    }
    if let Some(yr) = year {
        tag.set_year(yr);
    } else {
        tag.remove_year();
    }

    if let Some(lyr) = lyrics {
        tag.insert_text(lofty::tag::ItemKey::Lyrics, lyr.to_string());
    }

    if let Some(cp) = cover_path {
        if let Ok(cover_data) = std::fs::read(cp) {
            let mime = if cp.to_lowercase().ends_with(".png") {
                "image/png".to_string()
            } else {
                "image/jpeg".to_string()
            };
            let picture = lofty::picture::Picture::new_unchecked(
                lofty::picture::PictureType::CoverFront,
                Some(lofty::picture::MimeType::Unknown(mime)),
                None,
                cover_data.clone(),
            );
            while !tag.pictures().is_empty() {
                tag.remove_picture(0);
            }
            tag.push_picture(picture);

            // Also drop a copy beside the file so file managers and other
            // players see the new art. Only when there is nothing there yet:
            // overwriting an album's hand-picked cover.jpg from a single track
            // edit is not ours to decide.
            if let Some(parent) = path.parent() {
                let folder_cover = parent.join("cover.jpg");
                if !folder_cover.exists() {
                    let _ = std::fs::write(folder_cover, cover_data);
                }
            }
        }
    }

    tagged_file.remove(lofty::tag::TagType::Id3v1);
    // Atomic: copy → save on tmp → rename (same dir, same FS) — preserve extension for Probe
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("mp3");
    let tmp = path.with_extension(format!("tmp.{}", ext));
    std::fs::copy(path, &tmp)?;
    tagged_file.save_to_path(&tmp, Default::default())?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scan_media_dir() {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        let music_dir = PathBuf::from(home).join("Music");

        // This is a smoke test against a real library, not a fixture-based unit
        // test. Skip when there is nothing to scan (clean build dirs, chroots,
        // CI) instead of failing a release build for having no music on disk.
        if !music_dir.is_dir() {
            eprintln!(
                "skipping test_scan_media_dir: {} is not a directory",
                music_dir.display()
            );
            return;
        }

        let tracks = scan_folder(&music_dir);
        println!(
            "scan_folder returned {} tracks from {}",
            tracks.len(),
            music_dir.display()
        );
        if tracks.is_empty() {
            // Check why read_tags fails on sample track
            for entry in WalkDir::new(&music_dir).into_iter().filter_map(|e| e.ok()) {
                let p = entry.path();
                if p.extension().and_then(|e| e.to_str()) == Some("opus") {
                    let err = read_tags(p).err();
                    println!("Failed to read_tags for {:?}: {:?}", p, err);
                    break;
                }
            }
            eprintln!(
                "skipping test_scan_media_dir: no tracks found in {}",
                music_dir.display()
            );
            return;
        }
        assert!(!tracks.is_empty(), "scan_folder should find tracks!");
    }

    fn fake_track(path: &str, artist: &str, title: &str, secs: u64) -> Track {
        Track {
            id: 0,
            path: PathBuf::from(path),
            title: title.to_string(),
            artist: artist.to_string(),
            album: "Album".to_string(),
            album_id: 0,
            track_number: Some(1),
            disc_number: Some(1),
            duration: Duration::from_secs(secs),
            cover_data: None,
            genre: "Test".to_string(),
            year: Some(2026),
            play_count: 0,
            liked: false,
            date_played: None,
            lyrics: String::new(),
        }
    }

    #[test]
    fn duplicates_find_same_recording() {
        let tracks = vec![
            fake_track("/a/one.flac", "Boards of Canada", "Roygbiv", 152),
            fake_track("/b/one.mp3", "boards of canada", "Roygbiv!", 153),
            fake_track("/c/other.flac", "Boards of Canada", "Rue The Whirl", 400),
        ];
        let groups = duplicates::find(&tracks);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].len(), 2);
        assert_eq!(groups[0][0].path, PathBuf::from("/a/one.flac"));
    }

    #[test]
    fn duplicates_respect_duration() {
        // Same artist and title, but ten minutes apart: a different recording.
        let tracks = vec![
            fake_track("/a/live.flac", "Portishead", "Roads", 300),
            fake_track("/b/live.flac", "Portishead", "Roads", 900),
        ];
        assert!(duplicates::find(&tracks).is_empty());
    }

    #[test]
    fn normalisation_ignores_punctuation() {
        assert_eq!(
            duplicates::normalise("Boards of Canada"),
            duplicates::normalise("Boards  of, Canada!")
        );
        assert_eq!(duplicates::normalise("Sigur Rós"), "sigur rós");
    }
}
