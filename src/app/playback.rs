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
        self.send_mpris(MprisUpdate::Loop(if self.repeat {
            mpris_server::LoopStatus::Track
        } else {
            mpris_server::LoopStatus::None
        }));
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
            status, self.shuffle, self.repeat
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
        if let Some(t) = self.all_tracks.iter().find(|t| t.path == track.path) {
            track.liked = t.liked;
        }
        let cover_data = load_cover(&track.path);
        let track = Track {
            cover_data,
            ..track
        };
        self.audio.send(AudioCommand::Play(track.path.clone()));
        self.audio.send(AudioCommand::SetVolume(self.volume));
        self.current_track = Some(track.clone());
        self.selected_track = Some(track.clone());
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
