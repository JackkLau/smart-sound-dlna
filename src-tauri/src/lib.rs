pub mod bilibili;
#[cfg(feature = "desktop")]
mod catalog_queue;
pub mod dlna;
#[cfg(feature = "desktop")]
mod douban_catalog;
pub mod douyin;
#[cfg(feature = "desktop")]
mod douyin_webview;
pub mod engine;
pub mod library;
pub mod magnet;
#[cfg(feature = "desktop")]
mod magnet_clipboard;
pub mod media;
pub mod mobile;
mod mobile_pairing;
#[cfg(feature = "desktop")]
mod movie_discovery;
pub mod movie_library;
mod online;
mod subtitle_cache;
#[cfg(feature = "desktop")]
pub mod torrent_transfer;
mod track_names;
pub mod video;
mod video_catalog;
mod video_diagnostics;
pub mod video_subtitles;
#[cfg(feature = "desktop")]
mod video_tools;
pub mod voice;

#[cfg(feature = "desktop")]
mod desktop;
#[cfg(feature = "desktop")]
mod login;
#[cfg(feature = "desktop")]
mod login_webview;
#[cfg(feature = "desktop")]
mod session_cache;
#[cfg(feature = "desktop")]
mod xiaomi_login;
#[cfg(feature = "desktop")]
pub use desktop::run;

#[cfg(feature = "desktop")]
#[doc(hidden)]
pub fn desktop_context() -> tauri::Context<tauri::Wry> {
    tauri::generate_context!()
}
