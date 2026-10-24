//! Observe login navigation without querying the native webview's current URL.
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use tauri::{webview::Cookie, WebviewWindow};

pub const LOAD_TIMEOUT: Duration = Duration::from_secs(30);
pub const LOAD_ERROR: &str = "登录网页未能加载，请检查网络或代理设置，再点击“登录”重试。";

#[derive(Default)]
pub struct LoginPage {
    loaded: AtomicBool,
    service_callback: AtomicBool,
}

pub struct Observation {
    pub cookies: Vec<Cookie<'static>>,
    pub service_callback: bool,
}

impl LoginPage {
    pub fn loaded(&self, service_callback: bool) {
        self.service_callback
            .store(service_callback, Ordering::Release);
        self.loaded.store(true, Ordering::Release);
    }

    pub fn has_loaded(&self) -> bool {
        self.loaded.load(Ordering::Acquire)
    }

    pub fn load_error(&self, elapsed: Duration) -> Option<&'static str> {
        (!self.has_loaded() && elapsed >= LOAD_TIMEOUT).then_some(LOAD_ERROR)
    }

    // Call off the UI thread. Wry 0.57's macOS URL getter unwraps WKWebView.URL,
    // which is nil after an initial TLS failure: the UI thread panics and exits
    // with 101. Even `.url().ok()` cannot catch it. Only native callbacks may
    // supply navigation state; cookie polling must never query window.url().
    pub fn observe(&self, window: &WebviewWindow) -> Result<Observation, String> {
        let cookies = window.cookies().map_err(|_| "无法读取登录会话，请重试。")?;
        Ok(Observation {
            cookies,
            service_callback: self.service_callback.load(Ordering::Acquire),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_initial_load_has_a_bounded_actionable_error() {
        let page = LoginPage::default();
        assert!(page
            .load_error(LOAD_TIMEOUT - Duration::from_millis(1))
            .is_none());
        assert_eq!(page.load_error(LOAD_TIMEOUT), Some(LOAD_ERROR));
        // A late successful navigation or user retry can recover the same window.
        page.loaded(false);
        assert!(page.load_error(Duration::from_secs(600)).is_none());
    }
}
