//! Read-only companion contract for desktop widgets.
//!
//! The Omarchy/Quickshell companion needs three things that MPRIS cannot carry:
//! the next entries in Omatunes' real playback order, the equalizer state, and
//! which queue/album/playlist is playing. Re-deriving any of them in the plugin
//! would mean duplicating this application's queue and audio logic, and guessing
//! a shuffle order it never chose — exactly what the companion contract forbids.
//!
//! So the app publishes a small, versioned, read-only snapshot to
//! `~/.cache/omatunes_companion.json`, replaced atomically. It is written only
//! when the snapshot actually changes: `publish_companion` builds the payload,
//! compares it with the last one it wrote, and touches the disk only on a real
//! difference. `revision` is incremented on each such change and is the change
//! signal clients can rely on (they otherwise watch the file).
//!
//! Upcoming order is defined by the same rule the player itself follows
//! (`advance_track(1)` plus the end-of-track handling): walk the queue from the
//! current entry forward; wrap only when Repeat is "All". With shuffle enabled
//! the queue is already the shuffled sequence the app chose, so the snapshot
//! inherits that order — the plugin never shuffles anything.
//!
//! Nothing here reads or writes the library database, and the file is a cache:
//! deleting it costs nothing but a re-publish on the next change.

use super::*;
use crate::audio::eq;

const COMPANION_SCHEMA: u32 = 1;
const UPCOMING_LIMIT: usize = 3;

impl AppState {
    /// Build the JSON body (without `revision`) for the current state.
    fn companion_body(&self) -> String {
        let mut upcoming = String::from("[");
        for (index, entry) in self.companion_upcoming().iter().enumerate() {
            if index > 0 {
                upcoming.push(',');
            }
            upcoming.push_str(&format!(
                "{{\"id\":{},\"title\":\"{}\",\"artist\":\"{}\",\"duration_ms\":{}}}",
                entry.id,
                json_escape(&entry.title),
                json_escape(&entry.artist),
                entry.duration.as_millis() as u64,
            ));
        }
        upcoming.push(']');

        let (kind, name) = match &self.playing_context {
            Some(PlayingContext::Playlist(name)) => ("Playlist", name.clone()),
            Some(PlayingContext::SmartPlaylist(name)) => ("Smart playlist", name.clone()),
            Some(PlayingContext::Autoplaylist(name)) => ("Playlist", name.clone()),
            Some(PlayingContext::Artist(name)) => ("Artist", name.clone()),
            Some(PlayingContext::Album(name)) => ("Album", name.clone()),
            Some(PlayingContext::Genre(name)) => ("Genre", name.clone()),
            None => ("", String::new()),
        };

        let repeat = match self.repeat {
            RepeatMode::Off => "off",
            RepeatMode::All => "queue",
            RepeatMode::One => "track",
        };

        let mut presets = String::from("[");
        for (index, preset) in eq::BUILTIN_PRESETS.iter().enumerate() {
            if index > 0 {
                presets.push(',');
            }
            presets.push_str(&format!("\"{}\"", json_escape(preset.name)));
        }
        presets.push(']');

        format!(
            concat!(
                "\"product\":\"omatunes\",\"repeat\":\"{}\",",
                "\"context\":{{\"kind\":\"{}\",\"name\":\"{}\",\"queue_length\":{}}},",
                "\"eq\":{{\"enabled\":{},\"preset\":\"{}\",\"presets\":{}}},",
                "\"upcoming\":{}"
            ),
            repeat,
            json_escape(kind),
            json_escape(&name),
            self.queue.len(),
            self.equalizer.enabled,
            json_escape(eq::preset_name(&self.equalizer)),
            presets,
            upcoming,
        )
    }

    /// The entries the player would reach by advancing, in the app's real order.
    fn companion_upcoming(&self) -> Vec<&Track> {
        let len = self.queue.len();
        if len == 0 {
            return Vec::new();
        }
        let start = match self
            .current_track
            .as_ref()
            .and_then(|current| self.queue.iter().position(|track| track.id == current.id))
        {
            Some(index) => index + 1,
            None => 0,
        };
        let wrap = matches!(self.repeat, RepeatMode::All);

        let mut upcoming = Vec::new();
        let mut index = start;
        let mut walked = 0;
        while upcoming.len() < UPCOMING_LIMIT && walked < len {
            if index >= len {
                if !wrap {
                    break;
                }
                index = 0;
            }
            upcoming.push(&self.queue[index]);
            index += 1;
            walked += 1;
        }
        upcoming
    }

    /// Publish the companion snapshot when it has changed.
    ///
    /// Called from the handful of places where playback order, repeat, the
    /// equalizer or the playing context can change. A no-op costs one string
    /// comparison and no I/O.
    pub(super) fn publish_companion(&mut self) {
        let body = self.companion_body();
        if self.companion_last_body.as_deref() == Some(body.as_str()) {
            return;
        }
        self.companion_last_body = Some(body.clone());
        self.companion_revision = self.companion_revision.wrapping_add(1);

        let home = match std::env::var("HOME") {
            Ok(home) if !home.is_empty() => home,
            _ => return,
        };
        let path = PathBuf::from(&home).join(".cache/omatunes_companion.json");
        if let Some(parent) = path.parent() {
            if std::fs::create_dir_all(parent).is_err() {
                return;
            }
        }

        let payload = format!(
            "{{\"schema\":{},\"revision\":{},\"updated\":{},{}}}",
            COMPANION_SCHEMA,
            self.companion_revision,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            body
        );

        let tmp = path.with_extension("tmp.json");
        if std::fs::write(&tmp, &payload).is_ok() && std::fs::rename(&tmp, &path).is_ok() {
            return;
        }
        let _ = std::fs::write(&path, payload);
    }
}

fn json_escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}
