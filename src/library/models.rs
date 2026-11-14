use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct Track {
    pub id: i64,
    pub path: PathBuf,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_id: i64,
    pub track_number: Option<u32>,
    pub disc_number: Option<u32>,
    pub duration: Duration,
    pub cover_data: Option<Vec<u8>>,
    pub genre: String,
    pub year: Option<u32>,
    pub play_count: u32,
    pub liked: bool,
    pub date_played: Option<String>,
    pub lyrics: String,
}

impl Track {
    /// Copy the tag-derived fields from `other`, leaving runtime-only state
    /// (cover bytes, play counts, play dates) alone. Used to pull edits and
    /// rescans into snapshots such as the play queue.
    pub fn refresh_tags_from(&mut self, other: &Track) {
        self.title = other.title.clone();
        self.artist = other.artist.clone();
        self.album = other.album.clone();
        self.album_id = other.album_id;
        self.track_number = other.track_number;
        self.disc_number = other.disc_number;
        self.duration = other.duration;
        self.genre = other.genre.clone();
        self.year = other.year;
        self.liked = other.liked;
        self.lyrics = other.lyrics.clone();
    }

    pub fn duration_str(&self) -> String {
        let secs = self.duration.as_secs();
        let m = secs / 60;
        let s = secs % 60;
        format!("{m}:{s:02}")
    }

    pub fn primary_genre(&self) -> &str {
        self.genre.split("; ").next().unwrap_or(&self.genre)
    }

    pub fn genres(&self) -> Vec<&str> {
        if self.genre.contains("; ") {
            self.genre.split("; ").filter(|g| !g.is_empty()).collect()
        } else {
            vec![self.genre.as_str()]
        }
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Album {
    pub id: i64,
    pub title: String,
    pub artist: String,
    pub year: Option<u32>,
    pub cover_data: Option<Vec<u8>>,
}

#[derive(Debug, Clone)]
pub struct Artist {
    pub id: i64,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct Playlist {
    pub id: i64,
    pub name: String,
    pub track_count: usize,
}
