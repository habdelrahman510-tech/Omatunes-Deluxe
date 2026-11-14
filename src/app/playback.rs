// Split out of app.rs; methods only, no logic changes.
use super::*;

impl AppState {
    pub(super) fn send_mpris(&self, update: MprisUpdate) {
        let _ = self.mpris_update_tx.send(update);
    }

    pub(super) fn notify_mpris_track(&self, status: PlaybackStatus) {
        if let Some(track) = &self.current_track {
            let art_url = if let Some(parent) = track.path.parent() {
                let mut found = None;
                for name in &[
                    "cover.jpg",
                    "Cover.jpg",
                    "cover.png",
                    "Cover.png",
                    "cover.webp",
                    "Cover.webp",
                    "folder.jpg",
                    "Folder.jpg",
                ] {
                    let p = parent.join(name);
                    if p.exists() {
                        found = Some(format!("file://{}", p.to_string_lossy()));
                        break;
                    }
                }
                if let Some(url) = found {
                    url
                } else if let Some(cover_bytes) = crate::library::scanner::load_cover(&track.path) {
                    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
                    let cache_art = PathBuf::from(home).join(".cache/omatunes_current_art.jpg");
                    if std::fs::write(&cache_art, cover_bytes).is_ok() {
                        // Cache-bust with track id + timestamp so QML Image (cache:true) reloads per track
                        let ts = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis();
                        format!(
                            "file://{}?t={}_{}",
                            cache_art.to_string_lossy(),
                            track.id,
                            ts
                        )
                    } else {
                        String::new()
                    }
                } else {
                    String::new()
                }
            } else {
                String::new()
            };

            self.send_mpris(MprisUpdate::Metadata {
                title: track.title.clone(),
                artist: track.artist.clone(),
                album: track.album.clone(),
                duration_us: track.duration.as_micros() as i64,
                url: track.path.to_string_lossy().to_string(),
                art_url,
            });
        }
        self.send_mpris(MprisUpdate::Status(status));
        self.send_mpris(MprisUpdate::Shuffle(self.shuffle));
        self.send_mpris(MprisUpdate::Loop(self.repeat.mpris()));
        self.write_current_liked_status();
    }

    pub(super) fn write_current_liked_status(&self) {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        let path = PathBuf::from(&home).join(".cache/omatunes_current_liked");
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let status = self
            .current_track
            .as_ref()
            .map(|t| t.liked)
            .unwrap_or(false);
        let data = if status { "1" } else { "0" };
        let tmp = path.with_extension("tmp");
        if std::fs::write(&tmp, data).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        } else {
            let _ = std::fs::write(&path, data);
        }

        let state_path = PathBuf::from(&home).join(".cache/omatunes_current_state.json");
        if let Some(parent) = state_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let json = format!(
            "{{\"liked\":{},\"shuffle\":{},\"repeat\":{}}}",
            status,
            self.shuffle,
            self.repeat.is_on()
        );
        let state_tmp = state_path.with_extension("tmp.json");
        // Use .tmp.json to keep json extension for atomicity check, fallback to .tmp
        let tmp2 = state_path.with_extension("tmp");
        if std::fs::write(&state_tmp, &json).is_ok() {
            let _ = std::fs::rename(&state_tmp, &state_path);
        } else if std::fs::write(&tmp2, &json).is_ok() {
            let _ = std::fs::rename(&tmp2, &state_path);
        } else {
            let _ = std::fs::write(&state_path, json.clone());
        }
        // Dual-write to REAL_HOME for test isolation so live QuickShell (real HOME) sees TEST's state
        if let Ok(real_home) = std::env::var("REAL_HOME") {
            if real_home != home {
                let real_path = PathBuf::from(&real_home).join(".cache/omatunes_current_liked");
                if let Some(parent) = real_path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::write(&real_path, data);
                let real_state_path =
                    PathBuf::from(&real_home).join(".cache/omatunes_current_state.json");
                if let Some(parent) = real_state_path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::write(&real_state_path, json);
            }
        }
    }

    /// Publish the current curve to the audio thread and persist it.
    pub(super) fn commit_equalizer(&mut self) {
        self.audio.eq.update(self.equalizer);
        crate::db::write(|db| db.equalizer = self.equalizer);
    }

    pub(super) fn maybe_shuffle_queue(&mut self, anchor: Option<i64>) {
        if !self.shuffle || self.queue.is_empty() {
            return;
        }
        use rand::seq::SliceRandom;
        let mut rng = rand::thread_rng();
        if let Some(id) = anchor {
            if let Some(pos) = self.queue.iter().position(|t| t.id == id) {
                let item = self.queue.remove(pos);
                self.queue.shuffle(&mut rng);
                self.queue.insert(0, item);
                let queue_paths: Vec<PathBuf> = self.queue.iter().map(|t| t.path.clone()).collect();
                crate::db::write(|db| db.last_queue_paths = queue_paths);
                return;
            }
        }
        self.queue.shuffle(&mut rng);
        let queue_paths: Vec<PathBuf> = self.queue.iter().map(|t| t.path.clone()).collect();
        crate::db::write(|db| db.last_queue_paths = queue_paths);
    }

    /// The track an automatic advance will land on. Shuffle is already baked
    /// into the queue order, so this is a pure lookup.
    fn next_automatic_path(&self) -> Option<PathBuf> {
        next_automatic_path(&self.queue, self.current_track.as_ref(), self.repeat)
    }

    /// Hand the next track to the engine's idle deck. Cheap and idempotent:
    /// the engine drops repeats, and we skip sending if nothing changed.
    pub(super) fn preload_next(&mut self) {
        let next = self.next_automatic_path();
        if next == self.preloaded_path {
            return;
        }
        self.preloaded_path = next.clone();
        self.audio.send(AudioCommand::Preload(next));
    }

    /// Pull freshly scanned metadata into the snapshots that hold track copies
    /// (queue, now-playing, selection). Without this, a rescan — including the
    /// one our own tag write triggers — leaves the queue showing the old tags.
    pub(super) fn sync_snapshots_with_library(&mut self) {
        let all = self.all_tracks.clone();
        for queued in self.queue.iter_mut() {
            if let Some(fresh) = all.iter().find(|t| t.path == queued.path) {
                queued.refresh_tags_from(fresh);
            }
        }
        if let Some(current) = self.current_track.as_mut() {
            if let Some(fresh) = all.iter().find(|t| t.path == current.path) {
                current.refresh_tags_from(fresh);
            }
        }
        if let Some(selected) = self.selected_track.as_mut() {
            if let Some(fresh) = all.iter().find(|t| t.path == selected.path) {
                selected.refresh_tags_from(fresh);
            }
        }
        for selected in Arc::make_mut(&mut self.selected_tracks).iter_mut() {
            if let Some(fresh) = all.iter().find(|t| t.path == selected.path) {
                selected.refresh_tags_from(fresh);
            }
        }
    }

    /// Move the list's focus onto the track that is now playing, so the
    /// highlight follows the music instead of freezing on whatever was last
    /// clicked. A deliberate multi-selection is left alone: playback must not
    /// wipe a selection the operator is about to act on.
    fn focus_playing_track(&mut self, track: &Track) {
        self.selected_track = Some(track.clone());
        if self.selected_tracks.len() <= 1 {
            self.selected_tracks = std::sync::Arc::new(vec![track.clone()]);
            self.last_clicked_track = Some(track.clone());
        }
    }

    /// The engine spliced into the next deck on its own. Take ownership of the
    /// change: the UI, MPRIS, stats and the preload chain all have to follow.
    pub(super) fn on_spliced(&mut self, path: PathBuf) -> Task<Message> {
        // Repeat-one splices into the same file, so "the track we just left"
        // is the one we are now playing again.
        let repeating = self.current_track.as_ref().is_some_and(|t| t.path == path);

        // Write out any pending like for the track we just left.
        if let Some(prev) = self.current_track.as_ref() {
            if !repeating {
                let p = prev.path.clone();
                if crate::library::pending::flush_one(&p) {
                    eprintln!("Flushed pending liked on splice for {}", p.display());
                }
            }
        }

        let found = self
            .all_tracks
            .iter()
            .find(|t| t.path == path)
            .cloned()
            .or_else(|| self.queue.iter().find(|t| t.path == path).cloned());

        let Some(mut track) = found else {
            // Preloaded file vanished from the library between decks; treat it
            // as a stop rather than scrolling to a track we cannot name.
            self.audio.send(AudioCommand::Stop);
            self.playback_state = PlaybackState::Stopped;
            self.position = Duration::ZERO;
            self.send_mpris(MprisUpdate::Status(PlaybackStatus::Stopped));
            return Task::none();
        };

        // Re-reading the cover on every repeat is wasted work: it is the same
        // file with the same art.
        track.cover_data = if repeating {
            self.current_track
                .as_ref()
                .and_then(|t| t.cover_data.clone())
        } else {
            load_cover(&track.path)
        };
        self.current_track = Some(track.clone());
        self.focus_playing_track(&track);
        self.playback_state = PlaybackState::Playing;
        self.position = Duration::ZERO;
        self.duration = Duration::ZERO;
        self.last_accumulated_position = Duration::ZERO;
        self.current_track_play_counted = false;
        self.preloaded_path = None;
        self.update_live_smart_playlists();
        self.notify_mpris_track(PlaybackStatus::Playing);
        self.preload_next();

        let queue_paths: Vec<PathBuf> = self.queue.iter().map(|t| t.path.clone()).collect();
        crate::db::write(|db| {
            db.last_track_path = Some(track.path.clone());
            db.last_queue_paths = queue_paths;
            db.last_position_secs = 0;
        });
        if !repeating {
            crate::db::add_to_recently_played(track.path.clone());
        }
        if self.selected_playlist.as_deref() == Some("Recently Played") {
            self.update_filtered_tracks();
        }

        // Keep the list in step with the playing track when it is visible.
        if let Some(y) = self.calculate_scroll_offset(track.id) {
            let target_y = (y - 120.0).max(0.0);
            iced::widget::scrollable::scroll_to(
                iced::widget::scrollable::Id::new("tracklist_scroll"),
                iced::widget::scrollable::AbsoluteOffset {
                    x: 0.0,
                    y: target_y,
                },
            )
        } else {
            Task::none()
        }
    }

    pub(super) fn advance_track(&mut self, delta: i32) -> Task<Message> {
        // Flush pending liked tag for the track we're leaving (fd will be cancelled before next Play)
        if let Some(ref ct) = self.current_track {
            let p = ct.path.clone();
            if crate::library::pending::flush_one(&p) {
                eprintln!("Flushed pending liked on advance for {}", p.display());
            }
        }
        if self.queue.is_empty() {
            return Task::none();
        }

        let current_idx = self
            .current_track
            .as_ref()
            .and_then(|ct| self.queue.iter().position(|t| t.id == ct.id));
        let next_idx = match current_idx {
            Some(i) => {
                let new = i as i32 + delta;
                if new < 0 {
                    self.queue.len() - 1
                } else {
                    new as usize % self.queue.len()
                }
            }
            None => 0,
        };

        if let Some(track) = self.queue.get(next_idx).cloned() {
            self.play_track_internal(track)
        } else {
            Task::none()
        }
    }

    pub fn set_playing_context_from_current_view(&mut self) {
        if let Some(ref name) = self.selected_playlist {
            if name == "Liked Songs"
                || name == "Recently Played"
                || name == "Most Played"
                || name == "New Music"
                || name == "Duplicates"
            {
                self.playing_context = Some(PlayingContext::Autoplaylist(name.clone()));
            } else if crate::db::get(|db| db.smart_playlists.contains_key(name)) {
                self.playing_context = Some(PlayingContext::SmartPlaylist(name.clone()));
            } else {
                self.playing_context = Some(PlayingContext::Playlist(name.clone()));
            }
        } else if let Some(ref album) = self.selected_album {
            self.playing_context = Some(PlayingContext::Album(album.clone()));
        } else if let Some(ref artist) = self.selected_artist {
            self.playing_context = Some(PlayingContext::Artist(artist.clone()));
        } else if let Some(ref genre) = self.selected_genre {
            self.playing_context = Some(PlayingContext::Genre(genre.clone()));
        } else {
            self.playing_context = None;
        }
    }

    pub(super) fn play_track_internal(&mut self, mut track: Track) -> Task<Message> {
        // The queue holds snapshots taken when playback started. Tag edits and
        // library rescans update `all_tracks` only, so a stale queue entry
        // would resurrect the old lyrics the moment you switch back to a
        // track. The canonical copy wins; the caller's runtime data (cover
        // bytes) is reloaded below anyway.
        if let Some(canonical) = self.all_tracks.iter().find(|t| t.path == track.path) {
            track = canonical.clone();
        }
        let cover_data = load_cover(&track.path);
        let track = Track {
            cover_data,
            ..track
        };
        self.audio.send(AudioCommand::Play(track.path.clone()));
        self.audio.send(AudioCommand::SetVolume(self.volume));
        self.preloaded_path = None;
        self.current_track = Some(track.clone());
        self.focus_playing_track(&track);
        self.update_live_smart_playlists();
        self.playback_state = PlaybackState::Playing;
        self.position = Duration::ZERO;
        self.duration = Duration::ZERO;
        self.last_accumulated_position = Duration::ZERO;
        self.current_track_play_counted = false;
        self.notify_mpris_track(PlaybackStatus::Playing);

        let queue_paths: Vec<PathBuf> = self.queue.iter().map(|t| t.path.clone()).collect();
        crate::db::write(|db| {
            db.last_track_path = Some(track.path.clone());
            db.last_queue_paths = queue_paths;
            db.last_position_secs = 0;
            db.last_view_mode = Some(self.view_mode);
            db.last_selected_playlist = self.selected_playlist.clone();
            db.last_selected_folder = self.selected_folder.clone();
            db.last_selected_artist = self.selected_artist.clone();
            db.last_selected_album = self.selected_album.clone();
            db.last_selected_genre = self.selected_genre.clone();
            db.last_playlist_tab = Some(self.playlist_tab);
        });

        crate::db::add_to_recently_played(track.path.clone());
        if self.selected_playlist.as_deref() == Some("Recently Played") {
            self.update_filtered_tracks();
        }
        self.preload_next();

        if let Some(y) = self.calculate_scroll_offset(track.id) {
            let target_y = (y - 120.0).max(0.0);
            iced::widget::scrollable::scroll_to(
                iced::widget::scrollable::Id::new("tracklist_scroll"),
                iced::widget::scrollable::AbsoluteOffset {
                    x: 0.0,
                    y: target_y,
                },
            )
        } else {
            Task::none()
        }
    }
}

/// What the engine's idle deck should hold once the current track runs out.
///
/// The engine advances by splicing into whatever was preloaded, so this decides
/// the next track for every automatic transition. Repeat-one must therefore
/// return the *current* file: preloading the queue's next track lets the engine
/// splice into it before `TrackEnded` ever reaches the app, which is exactly
/// how repeat-one used to escape after a few loops.
fn next_automatic_path(
    queue: &[Track],
    current: Option<&Track>,
    repeat: RepeatMode,
) -> Option<PathBuf> {
    if repeat == RepeatMode::One {
        return current.map(|t| t.path.clone());
    }
    if queue.is_empty() {
        return None;
    }
    let index = current.and_then(|t| queue.iter().position(|q| q.id == t.id));
    match index {
        Some(idx) if idx + 1 < queue.len() => Some(queue[idx + 1].path.clone()),
        Some(_) if repeat == RepeatMode::All => Some(queue[0].path.clone()),
        // Not playing from the queue at all: start it from the top.
        None => Some(queue[0].path.clone()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::models::Track;

    fn track(id: i64, name: &str) -> Track {
        Track {
            id,
            path: PathBuf::from(format!("/music/{name}.mp3")),
            title: name.to_string(),
            artist: "artist".to_string(),
            album: "album".to_string(),
            album_id: 0,
            track_number: Some(id as u32),
            disc_number: None,
            duration: Duration::from_secs(60),
            cover_data: None,
            genre: "genre".to_string(),
            year: None,
            play_count: 0,
            liked: false,
            date_played: None,
            lyrics: String::new(),
        }
    }

    #[test]
    fn repeat_one_always_preloads_the_current_track() {
        let queue = vec![track(1, "a"), track(2, "b"), track(3, "c")];
        // First track of three: the old code preloaded "b" and the engine
        // spliced into it, so repeat-one never repeated.
        assert_eq!(
            next_automatic_path(&queue, Some(&queue[0]), RepeatMode::One),
            Some(PathBuf::from("/music/a.mp3"))
        );
        assert_eq!(
            next_automatic_path(&queue, Some(&queue[1]), RepeatMode::One),
            Some(PathBuf::from("/music/b.mp3"))
        );
    }

    #[test]
    fn repeat_one_still_repeats_when_the_track_left_the_queue() {
        let queue = vec![track(1, "a")];
        let current = track(9, "gone");
        assert_eq!(
            next_automatic_path(&queue, Some(&current), RepeatMode::One),
            Some(PathBuf::from("/music/gone.mp3"))
        );
    }

    #[test]
    fn repeat_off_stops_at_the_end() {
        let queue = vec![track(1, "a"), track(2, "b")];
        assert_eq!(
            next_automatic_path(&queue, Some(&queue[0]), RepeatMode::Off),
            Some(PathBuf::from("/music/b.mp3"))
        );
        assert_eq!(
            next_automatic_path(&queue, Some(&queue[1]), RepeatMode::Off),
            None
        );
    }

    #[test]
    fn repeat_all_wraps_to_the_top() {
        let queue = vec![track(1, "a"), track(2, "b")];
        assert_eq!(
            next_automatic_path(&queue, Some(&queue[1]), RepeatMode::All),
            Some(PathBuf::from("/music/a.mp3"))
        );
    }
}
