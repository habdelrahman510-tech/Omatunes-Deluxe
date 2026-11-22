mod app;
mod audio;
mod config;
mod db;
mod library;
mod locale;
mod paths;
mod stats;
mod ui;

/// Where the running instance publishes its pid, so a later launch can tell
/// "someone already owns the engine" from "nothing is playing".
fn pid_path() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    std::path::Path::new(&home).join(".cache/omatunes.pid")
}

/// Hand a launch to the instance that is already running. `show` asks it to put
/// its window on screen (the background mode has none); a plain launch only
/// needs to disappear, because the player is already there.
fn hand_off_to_running_instance(show: bool) -> bool {
    let pid: u32 = match std::fs::read_to_string(pid_path())
        .ok()
        .and_then(|text| text.trim().parse().ok())
    {
        Some(pid) => pid,
        None => return false,
    };
    if !std::path::Path::new(&format!("/proc/{pid}")).exists() {
        return false;
    }
    if show {
        if let Ok(socket) = std::net::UdpSocket::bind("127.0.0.1:0") {
            let _ = socket.send_to(b"show", "127.0.0.1:18888");
        }
    }
    true
}

fn write_pid() {
    let _ = std::fs::write(pid_path(), format!("{}", std::process::id()));
}

fn clear_pid() {
    let _ = std::fs::remove_file(pid_path());
}

fn main() -> iced::Result {
    if std::env::var_os("RUST_LOG").is_some() {
        let _ = env_logger::try_init();
    }
    // `omatunes --background` starts hidden so the bar companion can drive it
    // without a window appearing; the window is asked for later (`show`).
    let background = std::env::args_os().any(|arg| arg == "--background" || arg == "-b");
    let wants_file = std::env::args_os()
        .skip(1)
        .map(std::path::PathBuf::from)
        .any(|p| p.is_file());

    // One engine only. A second `omatunes` — from the launcher, a keybinding or
    // the bar — is a request to *see* the player, not another copy of it: the
    // audio device, the MPRIS name and the control socket are already owned.
    if !wants_file && hand_off_to_running_instance(!background) {
        return Ok(());
    }
    if std::env::var_os("WGPU_BACKEND").is_none() {
        std::env::set_var("WGPU_BACKEND", "gl,vulkan");
    }
    config::load(); // first: define music_dir, language, volume…
    db::init(); // initialize local database
    stats::init(); // initialize listening statistics
    locale::load();
    ui::theme::load_system_theme();
    write_pid();

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

    let res = app::run(background);
    // Best-effort synchronous flush on normal exit (covers close via window X, SIGTERM, etc.).
    // Does not cover SIGKILL (kill -9) — for that, high-value writes now flush() immediately (see db.rs).
    // Liked pending writes: db.json already has liked state (crash-safe), file tag is eventual mirror
    // flush_sync drains any pending liked tag writes for the currently playing track (fd now closed)
    crate::library::pending::flush_sync();
    db::flush_sync();
    stats::flush_sync();
    clear_pid();
    res
}
