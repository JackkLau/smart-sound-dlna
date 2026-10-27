pub mod bilibili;
pub mod dlna;
pub mod douyin;
#[cfg(feature = "desktop")]
mod douyin_webview;
pub mod engine;
pub mod library;
pub mod media;
pub mod mobile;
mod mobile_pairing;
mod online;
mod track_names;
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
