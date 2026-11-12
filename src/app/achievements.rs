// Split out of app.rs; methods only, no logic changes.
use super::*;

impl AppState {
    pub fn mark_all_time_changes_viewed(&self) {
        if let Some(ref breakdown) = self.show_period_breakdown {
            if breakdown.period_label == "All-Time" {
                crate::stats::write(|db| {
                    for (name, _, _) in &breakdown.artist_minutes {
                        db.viewed_rank_changes.insert(name.clone());
                    }
                    for (name, _, _) in &breakdown.album_minutes {
                        db.viewed_rank_changes.insert(name.clone());
                    }
                    for (name, _, _) in &breakdown.genre_minutes {
                        db.viewed_rank_changes.insert(name.clone());
                    }
                });
                crate::stats::flush();
            }
        }
    }

    pub fn show_achievements_in_ui(&self) -> bool {
        crate::config::get().show_achievements_in_ui
    }

    pub fn recalculate_achievements_items(&mut self) {
        let achievements = crate::stats::get(|db| db.earned_achievements.clone());
        let mut items = Vec::new();
        let entity_type_str = match self.achievements_sub_tab {
            AchievementsSubTab::Artists => "Artist",
            AchievementsSubTab::Albums => "Album",
            AchievementsSubTab::Genres => "Genre",
        };

        let mut unique_names = std::collections::HashSet::new();
        for track in self.all_tracks.iter() {
            match self.achievements_sub_tab {
                AchievementsSubTab::Artists => {
                    if !track.artist.trim().is_empty() {
                        unique_names.insert(track.artist.clone());
                    }
                }
                AchievementsSubTab::Albums => {
                    if !track.album.trim().is_empty() {
                        unique_names.insert(track.album.clone());
                    }
                }
                AchievementsSubTab::Genres => {
                    if !track.genre.trim().is_empty() {
                        let parts = if track.genre.contains("; ") {
                            track
                                .genre
                                .split("; ")
                                .map(|g| g.trim().to_string())
                                .collect::<Vec<_>>()
                        } else {
                            vec![track.genre.trim().to_string()]
                        };
                        for p in parts {
                            if !p.is_empty() && p != "Unknown" {
                                unique_names.insert(p);
                            }
                        }
                    }
                }
            }
        }

        let mut play_map = std::collections::HashMap::new();
        for track in self.all_tracks.iter() {
            match self.achievements_sub_tab {
                AchievementsSubTab::Artists => {
                    *play_map.entry(track.artist.clone()).or_insert(0) += track.play_count;
                }
                AchievementsSubTab::Albums => {
                    *play_map.entry(track.album.clone()).or_insert(0) += track.play_count;
                }
                AchievementsSubTab::Genres => {
                    let parts = if track.genre.contains("; ") {
                        track
                            .genre
                            .split("; ")
                            .map(|g| g.trim().to_string())
                            .collect::<Vec<_>>()
                    } else {
                        vec![track.genre.trim().to_string()]
                    };
                    for p in parts {
                        if !p.is_empty() && p != "Unknown" {
                            *play_map.entry(p).or_insert(0) += track.play_count;
                        }
                    }
                }
            }
        }

        for name in unique_names {
            let plays = play_map.get(&name).copied().unwrap_or(0);
            if plays == 0 {
                continue;
            }

            let entity_awards: Vec<_> = achievements
                .iter()
                .filter(|a| a.entity_type == entity_type_str && a.entity_name == name)
                .collect();
            let num_awards = entity_awards.len();
            let highest_tier_score = entity_awards
                .iter()
                .map(|a| crate::stats::get_achievement_score(&a.period, &a.tier))
                .max()
                .unwrap_or(0);

            items.push(AchievementItem {
                name,
                plays,
                highest_tier_score,
                num_awards,
            });
        }

        match self.achievements_sort {
            AchievementsSort::Alphabetical => {
                items.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
            }
            AchievementsSort::AchievementLevel => {
                items.sort_by(|a, b| {
                    b.highest_tier_score
                        .cmp(&a.highest_tier_score)
                        .then(b.num_awards.cmp(&a.num_awards))
                        .then(b.plays.cmp(&a.plays))
                        .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
                });
            }
        }

        self.achievements_items = items;
    }
}
