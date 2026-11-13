pub mod models;
pub mod pending;
pub mod playlist;
pub mod scanner;
pub mod smart_playlist;
pub mod watcher;

pub use models::Track;
pub use scanner::{
    embedded_cover, extract_cover_to_folder, folder_cover, is_audio_file, load_cache, load_cover,
    save_cache, scan_folder, scan_roots, write_tags,
};
