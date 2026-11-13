//! Recursive library watcher.
//!
//! Wraps `notify` and turns a burst of filesystem events into one "library
//! changed" signal, so a batch copy of 40 files triggers one incremental
//! rescan instead of forty. The signal channel is Tokio-side; the app drains
//! it from its existing timer tick.
//!
//! The watcher value must stay alive for as long as watching is wanted.

use std::path::PathBuf;
use std::time::Duration;

use notify::{RecursiveMode, Watcher};

/// How long the filesystem must be quiet before we declare the batch done.
const QUIET_MS: u64 = 750;

pub struct LibraryWatcher {
    _watcher: notify::RecommendedWatcher,
}

impl LibraryWatcher {
    /// Watch every root recursively. Returns `None` when no backend is
    /// available or no root exists yet.
    pub fn spawn(roots: Vec<PathBuf>, tx: tokio::sync::mpsc::UnboundedSender<()>) -> Option<Self> {
        let (raw_tx, raw_rx) = std::sync::mpsc::channel::<()>();

        let mut watcher =
            notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
                let Ok(event) = result else {
                    return;
                };
                let interesting = matches!(
                    event.kind,
                    notify::EventKind::Create(_)
                        | notify::EventKind::Remove(_)
                        | notify::EventKind::Modify(_)
                );
                if interesting && !event.paths.is_empty() {
                    let _ = raw_tx.send(());
                }
            })
            .ok()?;

        let mut watched = 0;
        for root in &roots {
            if root.is_dir() && watcher.watch(root, RecursiveMode::Recursive).is_ok() {
                watched += 1;
            }
        }
        if watched == 0 {
            return None;
        }

        std::thread::spawn(move || {
            while raw_rx.recv().is_ok() {
                while raw_rx.recv_timeout(Duration::from_millis(QUIET_MS)).is_ok() {}
                if tx.send(()).is_err() {
                    break;
                }
            }
        });

        Some(LibraryWatcher { _watcher: watcher })
    }
}
