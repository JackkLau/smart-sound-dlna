//! Ephemeral provider login state. No credentials are included in UI snapshots.
use crate::login_webview::LoginPage;
use serde::Serialize;
use std::{
    future::Future,
    sync::{Arc, Mutex},
};
use tauri::webview::Cookie;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Default, Serialize)]
pub struct LoginStatus {
    pub phase: Phase,
    pub error: Option<String>,
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    #[default]
    Idle,
    Waiting,
    Checking,
    Connected,
    Failed,
    Cancelled,
}

#[derive(Clone)]
pub struct Attempt {
    id: u64,
    pub cancel: CancellationToken,
    pub page: Arc<LoginPage>,
}

#[derive(Default)]
struct Inner {
    id: u64,
    attempt: Option<Attempt>,
    status: LoginStatus,
}

#[derive(Default)]
pub struct LoginFlow {
    // All provider mutations (automatic/manual completion, logout, re-login)
    // share this gate. A late completion cannot undo a completed logout.
    pub gate: tokio::sync::Mutex<()>,
    inner: Mutex<Inner>,
}

pub enum Completion {
    Connected,
    Cancelled { clear_session: bool },
}

impl LoginFlow {
    pub fn status(&self) -> LoginStatus {
        self.inner.lock().unwrap().status.clone()
    }

    pub fn begin(&self) -> Attempt {
        self.begin_with_page(Arc::default())
    }

    // A retry in an existing window must retain the state captured by that
    // window's navigation callbacks. A newly opened window gets fresh state.
    pub fn retry_in_window(&self) -> Attempt {
        let page = self
            .current()
            .map(|attempt| attempt.page)
            .unwrap_or_default();
        self.begin_with_page(page)
    }

    fn begin_with_page(&self, page: Arc<LoginPage>) -> Attempt {
        let mut inner = self.inner.lock().unwrap();
        if let Some(previous) = inner.attempt.take() {
            previous.cancel.cancel();
        }
        inner.id += 1;
        let attempt = Attempt {
            id: inner.id,
            cancel: CancellationToken::new(),
            page,
        };
        inner.attempt = Some(attempt.clone());
        inner.status = LoginStatus {
            phase: Phase::Waiting,
            error: None,
        };
        attempt
    }

    pub fn current(&self) -> Option<Attempt> {
        self.inner.lock().unwrap().attempt.clone()
    }

    pub fn update(&self, attempt: &Attempt, phase: Phase, error: Option<String>) -> bool {
        let mut inner = self.inner.lock().unwrap();
        if inner.id != attempt.id || attempt.cancel.is_cancelled() {
            return false;
        }
        inner.status = LoginStatus { phase, error };
        true
    }

    pub fn window_closed(&self, attempt: &Attempt) {
        let mut inner = self.inner.lock().unwrap();
        if inner.id == attempt.id && inner.status.phase != Phase::Connected {
            attempt.cancel.cancel();
            // Preserve a useful timeout/validation error when the window closes.
            inner.status.phase = Phase::Cancelled;
        }
    }

    pub fn reset(&self) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(attempt) = inner.attempt.take() {
            attempt.cancel.cancel();
        }
        inner.id += 1;
        inner.status = LoginStatus::default();
    }

    // Called with gate held, after all required cookies have arrived. Keeping
    // validation separate from the UI callback makes the close-on-success
    // contract testable without real credentials or a provider network request.
    pub async fn complete(
        &self,
        attempt: &Attempt,
        validate: impl Future<Output = Result<(), String>>,
        close_window: impl FnOnce() -> Result<(), String>,
    ) -> Result<Completion, String> {
        if !self.update(attempt, Phase::Checking, None) {
            return Ok(Completion::Cancelled {
                clear_session: false,
            });
        }
        let result = validate.await;
        if attempt.cancel.is_cancelled() {
            return Ok(Completion::Cancelled {
                clear_session: result.is_ok(),
            });
        }
        if let Err(error) = result {
            self.update(attempt, Phase::Failed, Some(error.clone()));
            return Err(error);
        }
        if !self.update(attempt, Phase::Connected, None) {
            return Ok(Completion::Cancelled {
                clear_session: true,
            });
        }
        if let Err(error) = close_window() {
            self.update(attempt, Phase::Connected, Some(error));
        }
        Ok(Completion::Connected)
    }
}

// Wry 0.57 on macOS implements cookies_for_url with exact domain equality.
// That drops normal Domain=.mina.mi.com / .xiaomi.com cookies. Read only the
// isolated login webview's store, then apply explicit provider/domain scoping.
pub fn session_cookie(provider: &str, cookies: &[Cookie<'_>]) -> Option<String> {
    let pick = |name: &str, hosts: &[&str], domains: &[&str]| -> Option<String> {
        for host in hosts {
            let mut candidates: Vec<_> = cookies
                .iter()
                .filter(|cookie| {
                    let domain = cookie
                        .domain()
                        .unwrap_or_default()
                        .trim_start_matches('.')
                        .to_ascii_lowercase();
                    cookie.name() == name
                        && !cookie.value().is_empty()
                        && domains.contains(&domain.as_str())
                        && (*host == domain || host.ends_with(&format!(".{domain}")))
                        && !cookie.expires_datetime().is_some_and(|at| {
                            at.unix_timestamp() <= (crate::online::now_ms() / 1000) as i64
                        })
                })
                .collect();
            // Prefer the most specific domain. Never depend on WebKit's order.
            candidates.sort_by_key(|cookie| {
                std::cmp::Reverse(
                    cookie
                        .domain()
                        .unwrap_or_default()
                        .trim_start_matches('.')
                        .len(),
                )
            });
            if let Some(cookie) = candidates.first() {
                return Some(format!("{name}={}", cookie.value()));
            }
        }
        None
    };
    let raw = match provider {
        "xiaomi" => {
            let user = pick(
                "userId",
                &["api2.mina.mi.com", "account.xiaomi.com"],
                &[
                    "api2.mina.mi.com",
                    "mina.mi.com",
                    "mi.com",
                    "account.xiaomi.com",
                    "xiaomi.com",
                ],
            )?;
            // Only micoapi's service host. Account/Mi Home service tokens must
            // never be mistaken for a XiaoAI token, even if their name matches.
            let token = pick(
                "serviceToken",
                &["api2.mina.mi.com"],
                &["api2.mina.mi.com", "mina.mi.com"],
            )?;
            format!("{user}; {token}")
        }
        "bilibili" => {
            let domains = &["bilibili.com", "www.bilibili.com", "api.bilibili.com"];
            let hosts = &["api.bilibili.com", "www.bilibili.com"];
            let mut selected = vec![pick("SESSDATA", hosts, domains)?];
            for name in ["bili_jct", "buvid3", "buvid4", "DedeUserID"] {
                if let Some(cookie) = pick(name, hosts, domains) {
                    selected.push(cookie);
                }
            }
            selected.join("; ")
        }
        _ => return None,
    };
    let names: &[&str] = if provider == "xiaomi" {
        &["userId", "serviceToken"]
    } else {
        &["SESSDATA", "bili_jct", "buvid3", "buvid4", "DedeUserID"]
    };
    crate::online::cookies(&raw, names).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn cookie(name: &'static str, domain: &'static str, value: &'static str) -> Cookie<'static> {
        Cookie::build((name, value))
            .domain(domain)
            .path("/")
            .http_only(true)
            .build()
    }

    #[test]
    fn macos_parent_domain_http_only_cookies_complete_xiaomi_session() {
        let cookies = vec![
            cookie("userId", ".xiaomi.com", "test-user"),
            cookie("serviceToken", ".mina.mi.com", "test-token"),
        ];
        // Demonstrate the old macOS lookup loses both cookies.
        assert!(!cookies
            .iter()
            .any(|c| c.domain() == Some("api2.mina.mi.com")));
        assert!(!cookies
            .iter()
            .any(|c| c.domain() == Some("account.xiaomi.com")));
        assert_eq!(
            session_cookie("xiaomi", &cookies).as_deref(),
            Some("userId=test-user; serviceToken=test-token")
        );
    }

    #[test]
    fn windows_exact_host_cookies_and_duplicate_precedence() {
        let cookies = vec![
            cookie("userId", "api2.mina.mi.com", "test-user"),
            cookie("serviceToken", ".mina.mi.com", "parent"),
            cookie("serviceToken", "api2.mina.mi.com", "specific"),
        ];
        assert_eq!(
            session_cookie("xiaomi", &cookies).as_deref(),
            Some("userId=test-user; serviceToken=specific")
        );
    }

    #[test]
    fn unrelated_tokens_and_incomplete_logins_are_not_accepted() {
        for domain in [
            "xiaomi.com",
            "mi.com",
            "api.io.mi.com",
            "mina.mi.com.evil.test",
            "evilmina.mi.com",
        ] {
            assert!(session_cookie(
                "xiaomi",
                &[
                    cookie("userId", ".xiaomi.com", "test-user"),
                    cookie("serviceToken", domain, "wrong-service")
                ]
            )
            .is_none());
        }
        assert!(
            session_cookie("xiaomi", &[cookie("userId", ".xiaomi.com", "test-user")]).is_none()
        );
        assert!(session_cookie(
            "xiaomi",
            &[cookie("serviceToken", ".mina.mi.com", "test-token")]
        )
        .is_none());
    }

    #[test]
    fn sibling_provider_uses_only_its_cookies() {
        let cookies = vec![
            cookie("SESSDATA", ".bilibili.com", "test-session"),
            cookie("passToken", ".xiaomi.com", "private"),
            cookie("serviceToken", ".mina.mi.com", "private"),
        ];
        assert_eq!(
            session_cookie("bilibili", &cookies).as_deref(),
            Some("SESSDATA=test-session")
        );
        assert!(session_cookie("unknown", &cookies).is_none());
    }

    #[test]
    fn closed_or_replaced_attempt_cannot_publish_success() {
        let flow = LoginFlow::default();
        let first = flow.begin();
        flow.window_closed(&first);
        assert!(!flow.update(&first, Phase::Connected, None));
        let second = flow.begin();
        assert!(!flow.update(&first, Phase::Failed, Some("old failure".into())));
        assert!(flow.update(&second, Phase::Checking, None));
        flow.reset();
        assert!(!flow.update(&second, Phase::Connected, None));
        assert_eq!(flow.status().phase, Phase::Idle);
    }

    #[test]
    fn automatic_window_close_preserves_confirmed_success() {
        let flow = LoginFlow::default();
        let attempt = flow.begin();
        assert!(flow.update(&attempt, Phase::Connected, None));
        flow.window_closed(&attempt);
        assert_eq!(flow.status().phase, Phase::Connected);
        assert!(flow.status().error.is_none());
    }

    #[test]
    fn retry_retains_window_callbacks_but_reopening_is_isolated() {
        let flow = LoginFlow::default();
        let first = flow.begin();
        flow.update(&first, Phase::Failed, Some("load failed".into()));
        let retry = flow.retry_in_window();
        assert!(first.cancel.is_cancelled());
        assert!(Arc::ptr_eq(&first.page, &retry.page));
        first.page.loaded(false); // The existing webview still owns this callback.
        assert!(retry.page.has_loaded());
        assert_eq!(flow.status().phase, Phase::Waiting);
        assert!(flow.status().error.is_none());

        flow.window_closed(&retry);
        let reopened = flow.begin();
        first.page.loaded(false); // A late event from the old window cannot affect it.
        assert!(!reopened.page.has_loaded());
        assert!(!Arc::ptr_eq(&retry.page, &reopened.page));
        assert!(retry.cancel.is_cancelled());
    }

    #[tokio::test]
    async fn server_confirmation_closes_window_and_publishes_connection() {
        let flow = LoginFlow::default();
        let attempt = flow.begin();
        let mut closed = false;
        let result = flow
            .complete(&attempt, async { Ok(()) }, || {
                closed = true;
                flow.window_closed(&attempt);
                Ok(())
            })
            .await
            .unwrap();
        assert!(matches!(result, Completion::Connected));
        assert!(closed);
        assert_eq!(flow.status().phase, Phase::Connected);
    }

    #[tokio::test]
    async fn failed_validation_keeps_window_for_retry_and_reports_error() {
        let flow = LoginFlow::default();
        let attempt = flow.begin();
        let result = flow
            .complete(
                &attempt,
                async { Err("network unavailable".into()) },
                || {
                    panic!("failed login must not close the window");
                },
            )
            .await;
        assert!(result.is_err());
        assert_eq!(flow.status().phase, Phase::Failed);
        assert_eq!(flow.status().error.as_deref(), Some("network unavailable"));
        assert!(matches!(
            flow.complete(&attempt, async { Ok(()) }, || Ok(()))
                .await
                .unwrap(),
            Completion::Connected
        ));
        assert!(flow.status().error.is_none());
    }

    #[tokio::test]
    async fn close_during_validation_cleans_late_session_without_success() {
        let flow = LoginFlow::default();
        let attempt = flow.begin();
        let result = flow
            .complete(
                &attempt,
                async {
                    flow.window_closed(&attempt);
                    Ok(())
                },
                || panic!("cancelled attempt must not publish success"),
            )
            .await
            .unwrap();
        assert!(matches!(
            result,
            Completion::Cancelled {
                clear_session: true
            }
        ));
        assert_eq!(flow.status().phase, Phase::Cancelled);
    }

    #[tokio::test]
    async fn logout_during_validation_cannot_be_undone_by_old_attempt() {
        let flow = LoginFlow::default();
        let attempt = flow.begin();
        flow.reset();
        let result = flow
            .complete(
                &attempt,
                async { panic!("old attempt cannot validate") },
                || Ok(()),
            )
            .await
            .unwrap();
        assert!(matches!(
            result,
            Completion::Cancelled {
                clear_session: false
            }
        ));
        assert_eq!(flow.status().phase, Phase::Idle);
    }
}
