// Split out of app.rs; methods only, no logic changes.
use super::*;

impl AppState {
    pub fn load_track_in_tag_editor(&mut self, track: Track) {
        let active_tab = self
            .show_tag_editor
            .as_ref()
            .map(|state| state.active_tab)
            .unwrap_or(TagEditorTab::Main);

        let mut original_tracks = self
            .show_tag_editor
            .as_mut()
            .map(|state| std::mem::take(&mut state.original_tracks))
            .unwrap_or_default();

        if !original_tracks.contains_key(&track.path) {
            original_tracks.insert(track.path.clone(), track.clone());
        }

        let tracks = vec![track.clone()];
        let first = &tracks[0];
        let mut genre_vec: Vec<String> =
            first.genres().into_iter().map(|s| s.to_string()).collect();
        // Always show at least 2 genre boxes
        while genre_vec.len() < 2 {
            genre_vec.push(String::new());
        }
        let genres_original = genre_vec.clone();
        let apply_genres = vec![false; genre_vec.len()];
        self.show_tag_editor = Some(TagEditorState {
            tracks: tracks.clone(),
            original_tracks,
            is_saved: false,
            title: first.title.clone(),
            artist: first.artist.clone(),
            album: first.album.clone(),
            genres: genre_vec.clone(),
            genres_original,
            apply_genres,
            track_number: first
                .track_number
                .map(|n| n.to_string())
                .unwrap_or_default(),
            disc_number: first.disc_number.map(|n| n.to_string()).unwrap_or_default(),
            cover_path: None,
            apply_to_album: false,
            year: first.year.map(|n| n.to_string()).unwrap_or_default(),
            apply_title: false,
            apply_artist: false,
            apply_album: false,
            apply_year: false,
            apply_track_num: false,
            apply_disc_num: false,
            apply_cover: false,
            apply_lyrics: false,
            lyrics: first.lyrics.clone(),
            lyrics_content: iced::widget::text_editor::Content::with_text(&first.lyrics),
            active_tab,
            focused_field: Some(0),
            pending_offset: 0.0,
            just_saved: HashSet::new(),
            removed_genres: Vec::new(),
        });
    }
}
