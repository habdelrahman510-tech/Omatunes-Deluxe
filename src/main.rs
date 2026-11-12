mod app;
mod audio;
mod config;
mod db;
mod library;
mod locale;
mod paths;
mod stats;
mod ui;

fn main() -> iced::Result {
    if std::env::var_os("RUST_LOG").is_some() {
        let _ = env_logger::try_init();
    }
    if std::env::var_os("WGPU_BACKEND").is_none() {
        std::env::set_var("WGPU_BACKEND", "gl,vulkan");
    }
    config::load(); // first: define music_dir, language, volume…
    db::init(); // initialize local database
    stats::init(); // initialize listening statistics
    locale::load();
    ui::theme::load_system_theme();

    // `omatunes /some/song.mp3` — play the file that was asked for rather than
    // resuming `db.last_track_path`. File managers launch us through
    // `omatunes-open`, which only takes this path when no instance is running to
    // hand the file to over the control socket (127.0.0.1:18888, `play <path>`).
    if let Some(path) = std::env::args_os()
        .skip(1)
        .map(std::path::PathBuf::from)
        .find(|p| p.is_file())
    {
        app::set_startup_path(path);
    }

    let res = app::run();
    // Best-effort synchronous flush on normal exit (covers close via window X, SIGTERM, etc.).
    // Does not cover SIGKILL (kill -9) — for that, high-value writes now flush() immediately (see db.rs).
    // Liked pending writes: db.json already has liked state (crash-safe), file tag is eventual mirror
    // flush_sync drains any pending liked tag writes for the currently playing track (fd now closed)
    crate::library::pending::flush_sync();
    db::flush_sync();
    stats::flush_sync();
    res
}
