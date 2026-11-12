// Split out of app.rs; methods only, no logic changes.
use super::*;

impl AppState {
    pub fn artists(&self) -> Vec<String> {
        let query = self.sidebar_search.to_lowercase();
        let mut artists: Vec<String> = self
            .all_tracks
            .iter()
            .map(|t| {
                if t.artist.trim().is_empty() {
                    "Unknown Artist".to_string()
                } else {
                    t.artist.clone()
                }
            })
            .collect();
        artists.sort_by(|a, b| {
            let normalize = |s: &str| {
                let lower = s.to_lowercase();
                if lower.starts_with("the ") {
                    lower[4..].to_string()
                } else {
                    lower
                }
            };
            normalize(a).cmp(&normalize(b))
        });
        artists.dedup();
        if !query.is_empty() {
            artists.retain(|a| a.to_lowercase().contains(&query));
        }
        artists.retain(|a| !self.hidden_artists_albums.contains(&(a.clone(), true)));
        artists
    }

    pub fn albums(&self) -> Vec<String> {
        let query = self.sidebar_search.to_lowercase();
        let mut albums: Vec<String> = self
            .all_tracks
            .iter()
            .map(|t| {
                if t.album.trim().is_empty() {
                    "Unknown Album".to_string()
                } else {
                    t.album.clone()
                }
            })
            .collect();
        albums.sort();
        albums.dedup();
        if !query.is_empty() {
            albums.retain(|a| a.to_lowercase().contains(&query));
        }
        albums.retain(|a| !self.hidden_artists_albums.contains(&(a.clone(), false)));
        albums
    }

    pub fn genres(&self) -> Vec<String> {
        let query = self.sidebar_search.to_lowercase();
        let mut genres: Vec<String> = self
            .all_tracks
            .iter()
            .flat_map(|t| {
                if t.genre.trim().is_empty() {
                    vec!["Unknown Genre".to_string()]
                } else if t.genre.contains("; ") {
                    t.genre
                        .split("; ")
                        .map(|g| {
                            let trimmed = g.trim();
                            if trimmed.is_empty() {
                                "Unknown Genre".to_string()
                            } else {
                                trimmed.to_string()
                            }
                        })
                        .collect()
                } else {
                    vec![t.genre.clone()]
                }
            })
            .collect();
        genres.sort();
        genres.dedup();
        if !query.is_empty() {
            genres.retain(|g| g.to_lowercase().contains(&query));
        }
        genres
    }

    /// Drops a restored selection that no longer exists (a deleted playlist, an
    /// artist/album/genre that left with its files) so the track list falls back
    /// to everything instead of restoring as an empty view.
    pub(super) fn sanitize_browse_selection(&mut self) {
        if let Some(name) = self.selected_playlist.clone() {
            let known = matches!(
                name.as_str(),
                "Liked Songs" | "Most Played" | "Recently Played" | "New Music"
            ) || crate::db::get(|db| {
                db.playlists.contains_key(&name) || db.smart_playlists.contains_key(&name)
            });
            if !known {
                self.selected_playlist = None;
            }
        }

        if let Some(artist) = self.selected_artist.clone() {
            let exists = self.all_tracks.iter().any(|t| {
                let a = if t.artist.trim().is_empty() {
                    "Unknown Artist"
                } else {
                    t.artist.as_str()
                };
                a == artist
            });
            if !exists {
                self.selected_artist = None;
            }
        }

        if let Some(album) = self.selected_album.clone() {
            let exists = self.all_tracks.iter().any(|t| {
                let al = if t.album.trim().is_empty() {
                    "Unknown Album"
                } else {
                    t.album.as_str()
                };
                al == album
            });
            if !exists {
                self.selected_album = None;
            }
        }

        if let Some(genre) = self.selected_genre.clone() {
            let exists = self.all_tracks.iter().any(|t| {
                t.genres().iter().any(|g| {
                    let clean = if g.trim().is_empty() {
                        "Unknown Genre"
                    } else {
                        g.trim()
                    };
                    clean == genre.as_str()
                })
            });
            if !exists {
                self.selected_genre = None;
            }
        }
    }

    /// Saves the browsing position immediately, so a restart lands where the
    /// user left off even if no track was played in between.
    pub(super) fn persist_browse_state(&self) {
        let vm = self.view_mode;
        let tab = self.playlist_tab;
        let playlist = self.selected_playlist.clone();
        let folder = self.selected_folder.clone();
        let artist = self.selected_artist.clone();
        let album = self.selected_album.clone();
        let genre = self.selected_genre.clone();
        crate::db::write(|db| {
            db.last_view_mode = Some(vm);
            db.last_playlist_tab = Some(tab);
            db.last_selected_playlist = playlist;
            db.last_selected_folder = folder;
            db.last_selected_artist = artist;
            db.last_selected_album = album;
            db.last_selected_genre = genre;
        });
    }

    pub fn update_filtered_tracks(&mut self) {
        self.track_list_start = 0;
        self.track_list_end = 500;
        if !self.search_query.is_empty() {
            let query = self.search_query.to_lowercase();
            self.tracks = Arc::new(
                self.all_tracks
                    .iter()
                    .filter(|t| {
                        let match_title =
                            self.filter_title && t.title.to_lowercase().contains(&query);
                        let match_artist =
                            self.filter_artist && t.artist.to_lowercase().contains(&query);
                        let match_album =
                            self.filter_album && t.album.to_lowercase().contains(&query);
                        let match_genre = self.filter_genre
                            && t.genres().iter().any(|g| g.to_lowercase().contains(&query));
                        match_title || match_artist || match_album || match_genre
                    })
                    .cloned()
                    .collect::<Vec<_>>(),
            );
        } else if let Some(playlist_name) = &self.selected_playlist {
            if playlist_name == "Liked Songs" {
                self.tracks = Arc::new(
                    self.all_tracks
                        .iter()
                        .filter(|t| t.liked)
                        .cloned()
                        .collect::<Vec<_>>(),
                );
            } else if playlist_name == "Most Played" {
                let mut temp = (*self.all_tracks).clone();
                temp.sort_by(|a, b| b.play_count.cmp(&a.play_count));
                self.tracks = Arc::new(
                    temp.into_iter()
                        .filter(|t| t.play_count > 0)
                        .collect::<Vec<_>>(),
                );
            } else if playlist_name == "Recently Played" {
                let rp = crate::db::get(|db| db.recently_played.clone());
                let mut temp_tracks = Vec::new();
                for (path, date_str) in rp {
                    if let Some(mut t) = self.all_tracks.iter().find(|t| t.path == path).cloned() {
                        t.date_played = Some(date_str);
                        temp_tracks.push(t);
                    }
                }
                self.tracks = Arc::new(temp_tracks);
            } else if playlist_name == "New Music" {
                use std::time::SystemTime;
                let mut album_times: std::collections::HashMap<String, SystemTime> =
                    std::collections::HashMap::new();
                for t in self.all_tracks.iter() {
                    let mtime = std::fs::metadata(&t.path)
                        .and_then(|meta| meta.modified())
                        .unwrap_or(SystemTime::UNIX_EPOCH);
                    let entry = album_times
                        .entry(t.album.clone())
                        .or_insert(SystemTime::UNIX_EPOCH);
                    if mtime > *entry {
                        *entry = mtime;
                    }
                }

                let mut albums_sorted: Vec<(String, SystemTime)> =
                    album_times.into_iter().collect();
                albums_sorted.sort_by(|a, b| b.1.cmp(&a.1));

                let now = SystemTime::now();
                let forty_eight_hours = std::time::Duration::from_secs(48 * 3600);

                let mut target_albums = std::collections::HashSet::new();
                for (idx, (album_title, added_time)) in albums_sorted.iter().enumerate() {
                    let is_in_last_48h = now
                        .duration_since(*added_time)
                        .map(|d| d < forty_eight_hours)
                        .unwrap_or(false);
                    if idx < 5 || is_in_last_48h {
                        target_albums.insert(album_title.clone());
                    }
                }

                let mut temp_tracks: Vec<Track> = self
                    .all_tracks
                    .iter()
                    .filter(|t| target_albums.contains(&t.album))
                    .cloned()
                    .collect();

                let album_times_ref = &albums_sorted
                    .into_iter()
                    .collect::<std::collections::HashMap<_, _>>();
                temp_tracks.sort_by(|a, b| {
                    let time_a = album_times_ref
                        .get(&a.album)
                        .unwrap_or(&SystemTime::UNIX_EPOCH);
                    let time_b = album_times_ref
                        .get(&b.album)
                        .unwrap_or(&SystemTime::UNIX_EPOCH);
                    let cmp_time = time_b.cmp(time_a);
                    if cmp_time == std::cmp::Ordering::Equal {
                        let cmp_album = a.album.cmp(&b.album);
                        if cmp_album == std::cmp::Ordering::Equal {
                            let cmp_disc =
                                a.disc_number.unwrap_or(0).cmp(&b.disc_number.unwrap_or(0));
                            if cmp_disc == std::cmp::Ordering::Equal {
                                a.track_number
                                    .unwrap_or(0)
                                    .cmp(&b.track_number.unwrap_or(0))
                            } else {
                                cmp_disc
                            }
                        } else {
                            cmp_album
                        }
                    } else {
                        cmp_time
                    }
                });

                self.tracks = Arc::new(temp_tracks);
            } else if let Some(sp) =
                crate::db::get(|db| db.smart_playlists.get(playlist_name).cloned())
            {
                self.tracks = Arc::new(self.evaluate_smart_playlist(&sp));
            } else {
                let paths = crate::db::get(|db| {
                    db.playlists.get(playlist_name).cloned().unwrap_or_default()
                });
                let track_map: std::collections::HashMap<std::path::PathBuf, Track> = self
                    .all_tracks
                    .iter()
                    .map(|t| (t.path.clone(), t.clone()))
                    .collect();
                self.tracks = Arc::new(
                    paths
                        .iter()
                        .filter_map(|p| track_map.get(p).cloned())
                        .collect::<Vec<_>>(),
                );
            }

            if playlist_name == "Liked Songs" || playlist_name == "New Music" {
                let manual =
                    crate::db::get(|db| db.auto_playlist_song_order.get(playlist_name).cloned());
                if let Some(manual_order) = manual {
                    let live_paths: Vec<PathBuf> =
                        self.tracks.iter().map(|t| t.path.clone()).collect();
                    let merged_paths = merge_song_order(&manual_order, &live_paths);
                    let track_map: std::collections::HashMap<PathBuf, Track> = self
                        .tracks
                        .iter()
                        .map(|t| (t.path.clone(), t.clone()))
                        .collect();
                    self.tracks = Arc::new(
                        merged_paths
                            .iter()
                            .filter_map(|p| track_map.get(p).cloned())
                            .collect::<Vec<_>>(),
                    );
                    crate::db::write(|db| {
                        db.auto_playlist_song_order
                            .insert(playlist_name.clone(), merged_paths);
                    });
                }
            } else if crate::db::get(|db| db.smart_playlists.contains_key(playlist_name)) {
                let manual =
                    crate::db::get(|db| db.smart_playlist_song_order.get(playlist_name).cloned());
                if let Some(manual_order) = manual {
                    let live_paths: Vec<PathBuf> =
                        self.tracks.iter().map(|t| t.path.clone()).collect();
                    let merged_paths = merge_song_order(&manual_order, &live_paths);
                    let track_map: std::collections::HashMap<PathBuf, Track> = self
                        .tracks
                        .iter()
                        .map(|t| (t.path.clone(), t.clone()))
                        .collect();
                    self.tracks = Arc::new(
                        merged_paths
                            .iter()
                            .filter_map(|p| track_map.get(p).cloned())
                            .collect::<Vec<_>>(),
                    );
                    crate::db::write(|db| {
                        db.smart_playlist_song_order
                            .insert(playlist_name.clone(), merged_paths);
                    });
                }
            }
        } else {
            match self.view_mode {
                ViewMode::Artists => {
                    if let Some(artist_name) = &self.selected_artist {
                        self.tracks = Arc::new(
                            self.all_tracks
                                .iter()
                                .filter(|t| {
                                    let a = if t.artist.trim().is_empty() {
                                        "Unknown Artist"
                                    } else {
                                        &t.artist
                                    };
                                    a == artist_name
                                })
                                .cloned()
                                .collect::<Vec<_>>(),
                        );
                    } else {
                        self.tracks = self.all_tracks.clone();
                    }
                }
                ViewMode::Albums => {
                    if let Some(album_name) = &self.selected_album {
                        self.tracks = Arc::new(
                            self.all_tracks
                                .iter()
                                .filter(|t| {
                                    let al = if t.album.trim().is_empty() {
                                        "Unknown Album"
                                    } else {
                                        &t.album
                                    };
                                    al == album_name
                                })
                                .cloned()
                                .collect::<Vec<_>>(),
                        );
                    } else {
                        self.tracks = self.all_tracks.clone();
                    }
                }
                ViewMode::Genres => {
                    if let Some(genre_name) = &self.selected_genre {
                        self.tracks = Arc::new(
                            self.all_tracks
                                .iter()
                                .filter(|t| {
                                    t.genres().iter().any(|g| {
                                        let clean = if g.trim().is_empty() {
                                            "Unknown Genre"
                                        } else {
                                            g.trim()
                                        };
                                        clean == genre_name
                                    })
                                })
                                .cloned()
                                .collect::<Vec<_>>(),
                        );
                    } else {
                        self.tracks = self.all_tracks.clone();
                    }
                }
                ViewMode::NowPlaying => {
                    self.tracks = Arc::new(self.queue.clone());
                }
            }
        }

        // Apply sorting
        if let Some(ref playlist_name) = self.selected_playlist {
            if playlist_name == "Recently Played" || playlist_name == "Most Played" {
                return;
            }
        }

        let group_by = self.group_by;
        let sort_column = self.sort_column;
        let sort_ascending = self.sort_ascending;

        if group_by != crate::db::GroupBy::None || sort_column.is_some() {
            Arc::make_mut(&mut self.tracks).sort_by(|a, b| {
                // 1. Sort by group key first if grouping is active
                if group_by != crate::db::GroupBy::None {
                    let key_a = match group_by {
                        crate::db::GroupBy::Album => a.album.to_lowercase(),
                        crate::db::GroupBy::Artist => a.artist.to_lowercase(),
                        crate::db::GroupBy::Genre => a.primary_genre().to_lowercase(),
                        crate::db::GroupBy::Year => {
                            a.year.map(|y| y.to_string()).unwrap_or_default()
                        }
                        crate::db::GroupBy::None => unreachable!(),
                    };
                    let key_b = match group_by {
                        crate::db::GroupBy::Album => b.album.to_lowercase(),
                        crate::db::GroupBy::Artist => b.artist.to_lowercase(),
                        crate::db::GroupBy::Genre => b.primary_genre().to_lowercase(),
                        crate::db::GroupBy::Year => {
                            b.year.map(|y| y.to_string()).unwrap_or_default()
                        }
                        crate::db::GroupBy::None => unreachable!(),
                    };
                    let cmp_group = key_a.cmp(&key_b);
                    if cmp_group != std::cmp::Ordering::Equal {
                        return cmp_group;
                    }
                }

                // 2. Sort by column within group
                if let Some(col) = sort_column {
                    let cmp = match col {
                        SortColumn::TrackNumber => {
                            let a_dc = a.disc_number.unwrap_or(0);
                            let b_dc = b.disc_number.unwrap_or(0);
                            let cmp_dc = a_dc.cmp(&b_dc);
                            if cmp_dc == std::cmp::Ordering::Equal {
                                let a_num = a.track_number.unwrap_or(u32::MAX);
                                let b_num = b.track_number.unwrap_or(u32::MAX);
                                a_num.cmp(&b_num)
                            } else {
                                cmp_dc
                            }
                        }
                        SortColumn::Title => a.title.to_lowercase().cmp(&b.title.to_lowercase()),
                        SortColumn::Artist => a.artist.to_lowercase().cmp(&b.artist.to_lowercase()),
                        SortColumn::Album => a.album.to_lowercase().cmp(&b.album.to_lowercase()),
                        SortColumn::Genre => a.genre.to_lowercase().cmp(&b.genre.to_lowercase()),
                        SortColumn::Year => {
                            let a_yr = a.year.unwrap_or(u32::MAX);
                            let b_yr = b.year.unwrap_or(u32::MAX);
                            a_yr.cmp(&b_yr)
                        }
                        SortColumn::DiscNumber => {
                            let a_dc = a.disc_number.unwrap_or(u32::MAX);
                            let b_dc = b.disc_number.unwrap_or(u32::MAX);
                            let cmp_dc = a_dc.cmp(&b_dc);
                            if cmp_dc == std::cmp::Ordering::Equal {
                                let a_num = a.track_number.unwrap_or(u32::MAX);
                                let b_num = b.track_number.unwrap_or(u32::MAX);
                                a_num.cmp(&b_num)
                            } else {
                                cmp_dc
                            }
                        }
                        SortColumn::Duration => a.duration.cmp(&b.duration),
                        SortColumn::Plays => a.play_count.cmp(&b.play_count),
                        SortColumn::DatePlayed => {
                            let a_dp = a.date_played.as_deref().unwrap_or("");
                            let b_dp = b.date_played.as_deref().unwrap_or("");
                            a_dp.cmp(b_dp)
                        }
                        SortColumn::Liked => a.liked.cmp(&b.liked),
                    };
                    if sort_ascending {
                        cmp
                    } else {
                        cmp.reverse()
                    }
                } else {
                    // Default layout sorting (Disc, then Track number)
                    let a_dc = a.disc_number.unwrap_or(0);
                    let b_dc = b.disc_number.unwrap_or(0);
                    let cmp_dc = a_dc.cmp(&b_dc);
                    if cmp_dc == std::cmp::Ordering::Equal {
                        let a_num = a.track_number.unwrap_or(u32::MAX);
                        let b_num = b.track_number.unwrap_or(u32::MAX);
                        a_num.cmp(&b_num)
                    } else {
                        cmp_dc
                    }
                }
            });
        }
    }

    pub fn is_draggable_playlist_view(&self) -> bool {
        match &self.selected_playlist {
            Some(name) => {
                name != "Recently Played"
                    && name != "Most Played"
                    && (crate::db::get(|db| db.playlists.contains_key(name.as_str()))
                        || crate::db::get(|db| db.smart_playlists.contains_key(name.as_str()))
                        || name == "Liked Songs"
                        || name == "New Music")
            }
            None => false,
        }
    }

    pub fn get_display_cover(&self) -> Option<iced::widget::image::Handle> {
        let is_playing_or_paused = !matches!(self.playback_state, PlaybackState::Stopped);
        let display_track = if is_playing_or_paused {
            self.current_track.as_ref()
        } else {
            self.selected_track.as_ref()
        };
        let track_id = display_track.map(|t| t.id);
        let cache_key = (track_id, self.cover_cache_version);

        let mut cache = self.cover_cache.lock().unwrap();
        if cache_key != (cache.id, cache.version) {
            cache.id = track_id;
            cache.version = self.cover_cache_version;
            cache.handle = display_track
                .and_then(|t| t.cover_data.as_ref())
                .map(|data| iced::widget::image::Handle::from_bytes(data.clone()));
        }
        cache.handle.clone()
    }

    pub fn calculate_scroll_offset(&self, track_id: i64) -> Option<f32> {
        let track_height = 34.0;
        let spacing = 1.0;
        if self.group_by != crate::db::GroupBy::None {
            let mut y = 0.0;
            let mut groups: Vec<(String, Vec<&crate::library::models::Track>)> = Vec::new();
            for track in self.tracks.iter() {
                let group_key = match self.group_by {
                    crate::db::GroupBy::Album => track.album.clone(),
                    crate::db::GroupBy::Artist => track.artist.clone(),
                    crate::db::GroupBy::Genre => track.primary_genre().to_string(),
                    crate::db::GroupBy::Year => {
                        track.year.map(|y| y.to_string()).unwrap_or_default()
                    }
                    crate::db::GroupBy::None => unreachable!(),
                };
                if let Some(last) = groups.last_mut() {
                    if last.0 == group_key {
                        last.1.push(track);
                        continue;
                    }
                }
                groups.push((group_key, vec![track]));
            }
            for (_group_name, tracks) in groups {
                let header_height = 28.0;
                if let Some(index_in_album) = tracks.iter().position(|t| t.id == track_id) {
                    y += header_height + spacing;
                    y += index_in_album as f32 * (track_height + spacing);
                    return Some(y);
                } else {
                    y += header_height + spacing;
                    y += tracks.len() as f32 * (track_height + spacing);
                    y += 8.0 + spacing;
                }
            }
        } else {
            if let Some(idx) = self.tracks.iter().position(|t| t.id == track_id) {
                return Some(idx as f32 * (track_height + spacing));
            }
        }
        None
    }
}
