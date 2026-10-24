//! Provider-scoped credentials in the OS store; never in the music index or UI.
use crate::online;
use std::sync::Arc;

const SERVICE: &str = "com.localmusic.xiaomi-dlna.sessions.v1";
const STORE_ERROR: &str = "无法访问系统登录缓存，请解锁系统凭据库或允许访问后重试。";

pub trait CredentialStore: Send + Sync {
    fn read(&self, provider: &str) -> Result<Option<String>, String>;
    fn write(&self, provider: &str, secret: &str) -> Result<(), String>;
    fn delete(&self, provider: &str) -> Result<(), String>;
}

struct NativeStore {
    service: String,
}
impl NativeStore {
    fn entry(&self, provider: &str) -> Result<keyring::Entry, String> {
        // Never silently use keyring's in-memory fallback on other platforms.
        if !cfg!(any(target_os = "macos", target_os = "windows")) {
            return Err(STORE_ERROR.into());
        }
        keyring::Entry::new(&self.service, provider).map_err(|_| STORE_ERROR.into())
    }
}
impl CredentialStore for NativeStore {
    fn read(&self, provider: &str) -> Result<Option<String>, String> {
        match self.entry(provider)?.get_secret() {
            Ok(secret) => String::from_utf8(secret)
                .map(Some)
                .map_err(|_| "登录缓存格式无效，请重新登录。".into()),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err(STORE_ERROR.into()),
        }
    }
    fn write(&self, provider: &str, secret: &str) -> Result<(), String> {
        self.entry(provider)?
            .set_secret(secret.as_bytes())
            .map_err(|_| STORE_ERROR.into())
    }
    fn delete(&self, provider: &str) -> Result<(), String> {
        match self.entry(provider)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(STORE_ERROR.into()),
        }
    }
}

pub struct SessionCache {
    store: Arc<dyn CredentialStore>,
}
impl Default for SessionCache {
    fn default() -> Self {
        Self {
            store: Arc::new(NativeStore {
                service: SERVICE.into(),
            }),
        }
    }
}
impl SessionCache {
    #[cfg(test)]
    pub(crate) fn with_store(store: Arc<dyn CredentialStore>) -> Self {
        Self { store }
    }
    pub async fn read(&self, provider: &str) -> Result<Option<String>, String> {
        check_provider(provider)?;
        let provider = provider.to_owned();
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            store
                .read(&provider)?
                .map(|secret| normalize(&provider, &secret))
                .transpose()
        })
        .await
        .map_err(|_| STORE_ERROR.to_owned())?
    }
    pub async fn save(&self, provider: &str, raw: &str) -> Result<(), String> {
        let secret = normalize(provider, raw)?;
        let provider = provider.to_owned();
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || store.write(&provider, &secret))
            .await
            .map_err(|_| STORE_ERROR.to_owned())?
    }
    pub async fn clear(&self, provider: &str) -> Result<(), String> {
        check_provider(provider)?;
        let provider = provider.to_owned();
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || store.delete(&provider))
            .await
            .map_err(|_| STORE_ERROR.to_owned())?
    }
}
fn check_provider(provider: &str) -> Result<(), String> {
    if matches!(provider, "xiaomi" | "bilibili") {
        Ok(())
    } else {
        Err("不支持的登录缓存。".into())
    }
}
fn normalize(provider: &str, raw: &str) -> Result<String, String> {
    check_provider(provider)?;
    let (keys, required): (&[&str], &[&str]) = if provider == "xiaomi" {
        (&["userId", "serviceToken"], &["userId=", "serviceToken="])
    } else {
        (
            &["SESSDATA", "bili_jct", "buvid3", "buvid4", "DedeUserID"],
            &["SESSDATA="],
        )
    };
    let secret = online::cookies(raw, keys)?;
    if secret.len() > 2400
        || !required
            .iter()
            .all(|key| secret.split("; ").any(|part| part.starts_with(key)))
    {
        return Err("登录缓存格式无效，请重新登录。".into());
    }
    Ok(secret)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::{collections::HashMap, sync::Mutex};
    #[derive(Default)]
    pub struct MemoryStore {
        pub secrets: Mutex<HashMap<String, String>>,
        pub fail: std::sync::atomic::AtomicBool,
    }
    impl CredentialStore for MemoryStore {
        fn read(&self, provider: &str) -> Result<Option<String>, String> {
            if self.fail.load(std::sync::atomic::Ordering::SeqCst) {
                return Err(STORE_ERROR.into());
            }
            Ok(self.secrets.lock().unwrap().get(provider).cloned())
        }
        fn write(&self, provider: &str, secret: &str) -> Result<(), String> {
            if self.fail.load(std::sync::atomic::Ordering::SeqCst) {
                return Err(STORE_ERROR.into());
            }
            self.secrets
                .lock()
                .unwrap()
                .insert(provider.into(), secret.into());
            Ok(())
        }
        fn delete(&self, provider: &str) -> Result<(), String> {
            if self.fail.load(std::sync::atomic::Ordering::SeqCst) {
                return Err(STORE_ERROR.into());
            }
            self.secrets.lock().unwrap().remove(provider);
            Ok(())
        }
    }
    #[tokio::test]
    async fn providers_survive_cache_recreation_and_logout_only_clears_its_provider() {
        let store = Arc::new(MemoryStore::default());
        let first = SessionCache::with_store(store.clone());
        first
            .save(
                "xiaomi",
                "userId=fixture; serviceToken=fixture-token; passToken=never-save",
            )
            .await
            .unwrap();
        first
            .save("bilibili", "SESSDATA=fixture-session; passToken=never-save")
            .await
            .unwrap();
        let restarted = SessionCache::with_store(store.clone());
        assert_eq!(
            restarted.read("xiaomi").await.unwrap().as_deref(),
            Some("userId=fixture; serviceToken=fixture-token")
        );
        assert_eq!(
            restarted.read("bilibili").await.unwrap().as_deref(),
            Some("SESSDATA=fixture-session")
        );
        restarted.clear("xiaomi").await.unwrap();
        assert!(first.read("xiaomi").await.unwrap().is_none());
        assert!(first.read("bilibili").await.unwrap().is_some());
        assert!(first.save("unknown", "SESSDATA=fixture").await.is_err());
        assert!(first.save("xiaomi", "userId=fixture").await.is_err());
        assert!(first
            .save("bilibili", "SESSDATA=fixture\r\nHost: other")
            .await
            .is_err());
        store.fail.store(true, std::sync::atomic::Ordering::SeqCst);
        assert!(first.clear("bilibili").await.is_err());
        store.fail.store(false, std::sync::atomic::Ordering::SeqCst);
        assert!(first.read("bilibili").await.unwrap().is_some());
    }

    #[test]
    #[ignore = "requires the native OS credential store"]
    fn native_process_round_trip() {
        let namespace = format!("{SERVICE}.verification.{}", uuid::Uuid::new_v4());
        struct Cleanup(String);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let store = NativeStore {
                    service: self.0.clone(),
                };
                let _ = store.delete("xiaomi");
                let _ = store.delete("bilibili");
            }
        }
        let _cleanup = Cleanup(namespace.clone());
        for operation in ["write", "read", "delete", "empty"] {
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "session_cache::tests::native_child_process",
                    "--ignored",
                    "--nocapture",
                ])
                .env("XIAOMI_CACHE_TEST_NAMESPACE", &namespace)
                .env("XIAOMI_CACHE_TEST_OPERATION", operation)
                .status()
                .unwrap();
            assert!(status.success(), "native credential {operation} failed");
        }
    }
    #[tokio::test]
    #[ignore = "child process of native_process_round_trip"]
    async fn native_child_process() {
        let Ok(namespace) = std::env::var("XIAOMI_CACHE_TEST_NAMESPACE") else {
            return;
        };
        assert!(namespace.starts_with(&format!("{SERVICE}.verification.")));
        let cache = SessionCache {
            store: Arc::new(NativeStore { service: namespace }),
        };
        match std::env::var("XIAOMI_CACHE_TEST_OPERATION")
            .unwrap()
            .as_str()
        {
            "write" => {
                cache
                    .save("xiaomi", "userId=fixture-only; serviceToken=fixture-token")
                    .await
                    .unwrap();
                cache
                    .save("bilibili", &format!("SESSDATA={}", "f".repeat(1600)))
                    .await
                    .unwrap();
            }
            "read" => {
                assert_eq!(
                    cache.read("xiaomi").await.unwrap().as_deref(),
                    Some("userId=fixture-only; serviceToken=fixture-token")
                );
                assert_eq!(
                    cache.read("bilibili").await.unwrap(),
                    Some(format!("SESSDATA={}", "f".repeat(1600)))
                );
            }
            "delete" => {
                cache.clear("xiaomi").await.unwrap();
                cache.clear("bilibili").await.unwrap();
            }
            "empty" => {
                assert!(cache.read("xiaomi").await.unwrap().is_none());
                assert!(cache.read("bilibili").await.unwrap().is_none());
            }
            _ => panic!("unknown credential test operation"),
        }
    }
}
