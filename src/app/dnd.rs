// Split out of app.rs; drag-and-drop plumbing and the drop overlay.
use super::*;
use iced::widget::{column, container};

const IMAGE_EXTS: &[&str] = &["jpg", "jpeg", "png", "webp", "bmp", "gif", "avif"];

fn extension_in(path: &std::path::Path, exts: &[&str]) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| exts.iter().any(|known| e.eq_ignore_ascii_case(known)))
        .unwrap_or(false)
}

pub fn is_audio(path: &std::path::Path) -> bool {
    crate::library::scanner::is_audio_file(path)
}

pub fn is_playlist(path: &std::path::Path) -> bool {
    extension_in(path, &["m3u", "m3u8"])
}

pub fn is_image(path: &std::path::Path) -> bool {
    extension_in(path, IMAGE_EXTS)
}

impl AppState {
    /// Called once per timer tick with every file dropped since the last one.
    /// iced emits `FileDropped` once per file, so batching here keeps a
    /// ten-file drop from starting ten separate queues.
    pub(super) fn handle_drops(&mut self, paths: Vec<PathBuf>) -> Task<Message> {
        let mut audio = Vec::new();
        let mut folders = Vec::new();
        let mut images = Vec::new();
        let mut playlists = Vec::new();

        for path in paths {
            if path.is_dir() {
                folders.push(path);
            } else if is_playlist(&path) {
                playlists.push(path);
            } else if is_audio(&path) {
                audio.push(path);
            } else if is_image(&path) {
                images.push(path);
            }
        }

        // Artwork dropped while the tag editor is open becomes the new cover.
        if !images.is_empty() {
            if let Some(state) = self.show_tag_editor.as_mut() {
                state.cover_path = Some(images[0].to_string_lossy().to_string());
                state.apply_cover = true;
                state.is_saved = false;
                state.just_saved.remove("cover");
            }
        }

        if !playlists.is_empty() {
            return self.import_dropped_playlists(playlists);
        }

        // A folder wins over loose files: walking it can add hundreds of tracks,
        // and mixing both into one queue would be a surprise either way.
        if let Some(folder) = folders.into_iter().next() {
            return Task::perform(
                async move {
                    tokio::task::spawn_blocking(move || scan_folder(&folder))
                        .await
                        .unwrap_or_default()
                },
                Message::DroppedFolderScanned,
            );
        }

        let tracks = self.resolve_dropped(&audio);
        self.play_dropped_queue(tracks)
    }

    /// Map dropped file paths onto library tracks. Anything outside the library
    /// roots gets its own tag read, so dropping a file from a USB stick still
    /// plays it with a sensible title.
    fn resolve_dropped(&self, paths: &[PathBuf]) -> Vec<Track> {
        let mut resolved: Vec<Track> = Vec::with_capacity(paths.len());
        let mut scanned_dirs: Vec<PathBuf> = Vec::new();
        let mut scanned: Vec<Track> = Vec::new();

        for path in paths {
            if resolved.iter().any(|t| &t.path == path) {
                continue;
            }
            if let Some(track) = self.all_tracks.iter().find(|t| &t.path == path) {
                resolved.push(track.clone());
                continue;
            }

            if let Some(dir) = path.parent().map(|p| p.to_path_buf()) {
                if !scanned_dirs.contains(&dir) {
                    // One walk per directory, not per file: dropping an album
                    // would otherwise rescan the same folder a dozen times.
                    scanned.extend(scan_folder(&dir));
                    scanned_dirs.push(dir);
                }
                if let Some(track) = scanned.iter().find(|t| &t.path == path) {
                    resolved.push(track.clone());
                }
            }
        }

        resolved
    }

    /// Replace the queue with `tracks` and start the first one.
    pub(super) fn play_dropped_queue(&mut self, tracks: Vec<Track>) -> Task<Message> {
        if tracks.is_empty() {
            return Task::none();
        }
        self.queue = tracks;
        self.maybe_shuffle_queue(None);
        // A drop is an ad-hoc queue, not a view: clear the browsing context so
        // play-count bookkeeping does not credit whatever album happened to be
        // on screen.
        self.playing_context = None;
        let first = self.queue[0].clone();
        self.play_track_internal(first)
    }

    /// Read m3u/m3u8 files and turn each one into a user playlist.
    fn import_dropped_playlists(&mut self, paths: Vec<PathBuf>) -> Task<Message> {
        let mut imported: Vec<String> = Vec::new();
        let mut last_error: Option<String> = None;

        for path in paths {
            match crate::library::playlist::parse_m3u(&path) {
                Ok(entries) if !entries.is_empty() => {
                    let stem = path
                        .file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_else(|| "Imported playlist".to_string());
                    let name = unique_playlist_name(&stem);
                    crate::db::write(|db| {
                        db.playlists.insert(name.clone(), entries.clone());
                    });
                    imported.push(format!("{name} ({} tracks)", entries.len()));
                }
                Ok(_) => {
                    last_error = Some(format!(
                        "{} has no playable entries",
                        path.file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_default()
                    ));
                }
                Err(err) => last_error = Some(format!("{err}")),
            }
        }

        if let Some(err) = last_error {
            self.push_toast("Import failed", &err, ToastKind::ThemeError);
        }
        if !imported.is_empty() {
            self.push_toast(
                "Playlist imported",
                &imported.join(", "),
                ToastKind::EnteredTop10,
            );
            self.update_filtered_tracks();
        }
        Task::none()
    }

    /// True when the drop would be artwork for the open tag editor.
    fn drop_wants_artwork(&self, paths: &[PathBuf]) -> bool {
        self.show_tag_editor.is_some() && !paths.is_empty() && paths.iter().all(|p| is_image(p))
    }

    /// The full-window "drop here" scrim, drawn while a drag is in flight.
    pub(super) fn drop_overlay(&self) -> Option<Element<'_, Message>> {
        let hovered = self.drag_hovered.as_ref()?;
        if hovered.is_empty() {
            return None;
        }

        let (icon, headline) = if self.drop_wants_artwork(hovered) {
            (
                crate::ui::icons::ICON_FOLDER,
                "Drop to set cover art".to_string(),
            )
        } else if hovered.iter().any(|p| is_playlist(p)) {
            (
                crate::ui::icons::ICON_LIST,
                "Drop to import playlist".to_string(),
            )
        } else if hovered.len() > 1 {
            (
                crate::ui::icons::ICON_PLAY,
                format!("Drop to play {} files", hovered.len()),
            )
        } else {
            (crate::ui::icons::ICON_PLAY, "Drop to play".to_string())
        };

        let card = container(
            column![
                text(icon)
                    .font(crate::ui::icons::NERD_FONT_MONO)
                    .size(40)
                    .color(theme::accent()),
                Space::with_height(14),
                text(headline)
                    .size(20)
                    .font(crate::ui::icons::UI_FONT_BOLD)
                    .color(theme::text()),
                Space::with_height(6),
                text(first_name(hovered)).size(13).color(theme::subtext()),
            ]
            .align_x(iced::alignment::Horizontal::Center),
        )
        .padding([28, 44])
        .style(|_| iced::widget::container::Style {
            background: Some(iced::Background::Color(theme::mantle())),
            border: iced::Border {
                color: theme::accent(),
                width: 2.0,
                radius: 14.0.into(),
            },
            shadow: iced::Shadow {
                color: iced::Color::from_rgba(0.0, 0.0, 0.0, 0.4),
                offset: [0.0, 6.0].into(),
                blur_radius: 18.0,
            },
            ..Default::default()
        });

        Some(
            container(card)
                .width(Length::Fill)
                .height(Length::Fill)
                .center_x(Length::Fill)
                .center_y(Length::Fill)
                .style(|_| iced::widget::container::Style {
                    background: Some(iced::Background::Color(iced::Color {
                        a: 0.55,
                        ..theme::base()
                    })),
                    ..Default::default()
                })
                .into(),
        )
    }
}

fn first_name(paths: &[PathBuf]) -> String {
    if paths.len() == 1 {
        paths[0]
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| paths[0].to_string_lossy().to_string())
    } else {
        format!("and {} more", paths.len() - 1)
    }
}

/// "Chill Mix.m3u" twice in one session must not silently overwrite the first
/// import, so suffix a counter until the name is free.
fn unique_playlist_name(base: &str) -> String {
    if !crate::db::get(|db| db.playlists.contains_key(base)) {
        return base.to_string();
    }
    for n in 2..1000 {
        let candidate = format!("{base} ({n})");
        if !crate::db::get(|db| db.playlists.contains_key(&candidate)) {
            return candidate;
        }
    }
    base.to_string()
}
