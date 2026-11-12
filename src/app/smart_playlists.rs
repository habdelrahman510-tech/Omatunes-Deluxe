// Split out of app.rs; methods only, no logic changes.
use super::*;

impl AppState {
    pub fn evaluate_smart_playlist(
        &self,
        sp: &crate::library::smart_playlist::SmartPlaylist,
    ) -> Vec<Track> {
        let rp = crate::db::get(|db| db.recently_played.clone());
        let mut matched_tracks: Vec<Track> = self
            .all_tracks
            .iter()
            .filter(|t| crate::library::smart_playlist::evaluate_rules(t, &sp.rules, &rp))
            .cloned()
            .collect();

        // Hydrate date_played if available in recently_played
        for t in &mut matched_tracks {
            if let Some((_, date_str)) = rp.iter().find(|(p, _)| p == &t.path) {
                t.date_played = Some(date_str.clone());
            }
        }

        crate::library::smart_playlist::sort_and_limit_tracks(
            &mut matched_tracks,
            sp.order_by,
            sp.limit,
            &rp,
        );
        matched_tracks
    }

    pub fn update_live_smart_playlists(&mut self) {
        let smart_playlists = crate::db::get(|db| db.smart_playlists.clone());
        for (name, mut sp) in smart_playlists {
            if sp.live_updating {
                let evaluated = self.evaluate_smart_playlist(&sp);
                sp.tracks = evaluated.iter().map(|t| t.path.clone()).collect();
                crate::db::save_smart_playlist(name, sp);
            }
        }
    }
}
