//! Internet clients are deliberately separate from the LAN-only DLNA client.
use reqwest::{Client, Response};
use serde_json::Value;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub fn client(agent: &str) -> Result<Client, String> {
    Client::builder()
        .no_proxy()
        .user_agent(agent)
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .read_timeout(Duration::from_secs(20))
        .build()
        .map_err(|_| "无法创建网络连接".into())
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub async fn json(mut response: Response) -> Result<Value, String> {
    if !response.status().is_success() {
        return Err(match response.status().as_u16() {
            401 | 403 => "访问被拒绝，请重新登录或确认该内容的访问权限。".into(),
            412 | 429 => "请求暂时受限，请稍后重试。".into(),
            code => format!("服务暂时无法响应（HTTP {code}），请稍后重试。"),
        });
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "读取服务响应失败，请检查网络。")?
    {
        if bytes.len() + chunk.len() > 2 * 1024 * 1024 {
            return Err("服务响应过大，已停止读取。".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| "服务返回了无法识别的数据，请稍后重试或重新登录。".into())
}

/// Only explicitly allowlisted cookies may leave this process. Never include a
/// cookie/header or a signed URL in an error, snapshot, log, or persisted index.
pub fn cookies(raw: &str, allowed: &[&str]) -> Result<String, String> {
    if raw.len() > 16_384 || raw.contains(['\r', '\n']) {
        return Err("登录会话格式无效。".into());
    }
    let mut result = Vec::new();
    for name in allowed {
        if let Some(value) = raw
            .split(';')
            .filter_map(|p| p.trim().split_once('='))
            .find_map(|(k, v)| (k == *name).then_some(v.trim()))
        {
            if value.is_empty() || !value.bytes().all(|b| b.is_ascii_graphic() && b != b';') {
                return Err("登录会话格式无效。".into());
            }
            result.push(format!("{name}={value}"));
        }
    }
    Ok(result.join("; "))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cookie_scope_and_header_injection() {
        assert_eq!(
            cookies(
                "secret=x; userId=12; serviceToken=a=b",
                &["userId", "serviceToken"]
            )
            .unwrap(),
            "userId=12; serviceToken=a=b"
        );
        assert!(cookies("userId=1\r\nHost: bad", &["userId"]).is_err());
    }
}
