//! Exchange an already authenticated Xiaomi web account for the micoapi session.
//! The web STS redirect can return an empty 401: waiting for cookies cannot fix
//! that. Fetch the service challenge with the account session and sign its nonce.
use crate::online;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use reqwest::{header, Client};
use ring::digest::{digest, SHA1_FOR_LEGACY_USE_ONLY};
use serde_json::Value;
use std::time::Duration;
use tauri::webview::Cookie;
use url::Url;

const ACCOUNT: &str = "https://account.xiaomi.com/pass/serviceLogin";
const UA: &str = "APP/com.xiaomi.mihome APPV/11.3.203 iosPassportSDK/4.2.50 iOS/26.3.1 MK/aVBob25lMTcsMg== DEVT/aVBob25l DEVS/aU9T BRA/QXBwbGU= L/zh_CN";

pub fn account_cookie(cookies: &[Cookie<'_>]) -> Option<String> {
    let mut parts = Vec::new();
    for name in ["userId", "passToken", "deviceId"] {
        let value = ["account.xiaomi.com", "xiaomi.com"]
            .into_iter()
            .find_map(|domain| {
                cookies.iter().find(|cookie| {
                    cookie.name() == name
                        && cookie
                            .domain()
                            .is_some_and(|d| d.trim_start_matches('.').eq_ignore_ascii_case(domain))
                        && !cookie.value().is_empty()
                })
            });
        match value {
            Some(cookie) => parts.push(format!("{name}={}", cookie.value())),
            None if name == "deviceId" => {
                parts.push(format!("deviceId={}", uuid::Uuid::new_v4().simple()))
            }
            None => return None,
        }
    }
    parts.push("sdkVersion=3.9".into());
    online::cookies(
        &parts.join("; "),
        &["userId", "passToken", "deviceId", "sdkVersion"],
    )
    .ok()
}

fn signed_location(data: &Value) -> Result<(String, Url), String> {
    if data["code"].as_i64() != Some(0) {
        return Err("小米账号会话尚未授权，请在官方窗口完成验证后重试。".into());
    }
    let user = match &data["userId"] {
        Value::String(user) => user.clone(),
        Value::Number(user) => user.to_string(),
        _ => String::new(),
    };
    if user.is_empty() || !user.bytes().all(|b| b.is_ascii_digit()) {
        return Err("小米登录响应缺少账号信息，请重新登录。".into());
    }
    let nonce = match &data["nonce"] {
        Value::String(nonce) => nonce.clone(),
        Value::Number(nonce) => nonce.to_string(),
        _ => String::new(),
    };
    let security = data["ssecurity"].as_str().unwrap_or_default();
    if nonce.is_empty() || nonce.len() > 80 || security.is_empty() || security.len() > 2048 {
        return Err("小米登录响应缺少服务签名信息，请重新登录。".into());
    }
    let mut location = data["location"]
        .as_str()
        .and_then(|v| Url::parse(v).ok())
        .filter(is_sts)
        .ok_or("小米返回的服务授权地址无效。")?;
    let sign = STANDARD.encode(digest(
        &SHA1_FOR_LEGACY_USE_ONLY,
        format!("nonce={nonce}&{security}").as_bytes(),
    ));
    // Never use float conversion for Xiaomi's 64-bit nonce.
    let pairs: Vec<_> = location
        .query_pairs()
        .filter(|(key, _)| key != "clientSign")
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    location
        .query_pairs_mut()
        .clear()
        .extend_pairs(pairs)
        .append_pair("clientSign", &sign);
    Ok((user, location))
}

pub fn is_sts(url: &Url) -> bool {
    url.scheme() == "https"
        && url.host_str() == Some("api2.mina.mi.com")
        && url.port_or_known_default() == Some(443)
        && url.path() == "/sts"
        && url.username().is_empty()
        && url.password().is_none()
        && url.fragment().is_none()
}

pub async fn exchange(account: &str) -> Result<String, String> {
    exchange_with(&online::client(UA)?, account, None).await
}

// Local endpoint substitution is only available to this module's tests. In
// production both requests use fixed/validated Xiaomi origins, no redirects.
async fn exchange_with(
    client: &Client,
    account: &str,
    test_origin: Option<&str>,
) -> Result<String, String> {
    #[cfg(not(test))]
    assert!(test_origin.is_none());
    let account_url = test_origin
        .map(|origin| format!("{origin}/pass/serviceLogin"))
        .unwrap_or_else(|| ACCOUNT.into());
    let response = client
        .get(account_url)
        .query(&[("sid", "micoapi"), ("_json", "true"), ("_locale", "zh_CN")])
        .header(header::COOKIE, account)
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .map_err(|_| "无法向小米账号服务申请授权，请检查网络后重试。")?;
    let data = online::json_with_prefix(response, b"&&&START&&&").await?;
    let (user, location) = signed_location(&data)?;
    let target = test_origin
        .map(|origin| format!("{origin}/sts?{}", location.query().unwrap_or_default()))
        .unwrap_or_else(|| location.into());
    // Deliberately no account Cookie header on the service request: passToken
    // goes only to account.xiaomi.com, never to MiNA, a redirect or a CDN.
    let response = client
        .get(target)
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .map_err(|_| "小爱服务授权连接失败，请检查网络后重试。")?;
    if !response.status().is_success() {
        return Err(format!(
            "小米账号已登录，但小爱服务未签发授权（HTTP {}）。请稍后重试连接。",
            response.status().as_u16()
        ));
    }
    let token = response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .filter_map(|value| Cookie::parse(value).ok())
        .find(|cookie| cookie.name() == "serviceToken" && !cookie.value().is_empty())
        .map(|cookie| cookie.value().to_owned())
        .ok_or("小爱授权响应缺少服务凭据，请重试连接。")?;
    online::cookies(
        &format!("userId={user}; serviceToken={token}"),
        &["userId", "serviceToken"],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn challenge() -> Value {
        json!({"code":0,"userId":123,"nonce":6175437449139532800u64,"ssecurity":"test-security", "location":"https://api2.mina.mi.com/sts?auth=fixture"})
    }

    #[test]
    fn web_account_credentials_are_separate_from_mina_tokens() {
        let cookies = vec![
            Cookie::build(("userId", "123"))
                .domain(".xiaomi.com")
                .build(),
            Cookie::build(("passToken", "account-only"))
                .domain(".account.xiaomi.com")
                .http_only(true)
                .build(),
            Cookie::build(("serviceToken", "wrong"))
                .domain(".mi.com")
                .build(),
        ];
        let header = account_cookie(&cookies).unwrap();
        assert!(header.starts_with("userId=123; passToken=account-only; deviceId="));
        assert!(!header.contains("serviceToken"));
        assert!(account_cookie(&cookies[..1]).is_none());
    }

    #[test]
    fn signed_exchange_rejects_untrusted_location_without_exposing_it() {
        for url in [
            "http://api2.mina.mi.com/sts",
            "https://api2.mina.mi.com.evil.test/sts",
            "https://user:secret@api2.mina.mi.com/sts",
            "https://api2.mina.mi.com:8443/sts",
            "https://api2.mina.mi.com/other",
        ] {
            let mut data = challenge();
            data["location"] = url.into();
            let error = signed_location(&data).unwrap_err();
            assert!(!error.contains(url));
        }
    }

    #[tokio::test]
    async fn account_challenge_is_signed_and_account_cookie_never_reaches_sts() {
        use axum::{
            extract::Query,
            http::{HeaderMap, StatusCode},
            routing::get,
            Router,
        };
        use std::collections::HashMap;
        let app = Router::new()
            .route(
                "/pass/serviceLogin",
                get(
                    |headers: HeaderMap, Query(query): Query<HashMap<String, String>>| async move {
                        assert_eq!(query["sid"], "micoapi");
                        assert_eq!(query["_json"], "true");
                        assert!(headers[header::COOKIE]
                            .to_str()
                            .unwrap()
                            .contains("passToken=account-only"));
                        format!("&&&START&&&{}", challenge())
                    },
                ),
            )
            .route(
                "/sts",
                get(
                    |headers: HeaderMap, Query(query): Query<HashMap<String, String>>| async move {
                        assert!(!headers.contains_key(header::COOKIE));
                        assert_eq!(query["clientSign"], "YCqD/oPtWgGyamqGdVwWBVIDz9A=");
                        (
                            StatusCode::OK,
                            [(
                                header::SET_COOKIE,
                                "serviceToken=service-only; Domain=.mina.mi.com; Path=/; HttpOnly",
                            )],
                            "",
                        )
                    },
                ),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async {
            axum::serve(listener, app).await.unwrap();
        });
        let client = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let result =
            exchange_with(&client, "userId=123; passToken=account-only", Some(&origin)).await;
        server.abort();
        assert_eq!(result.unwrap(), "userId=123; serviceToken=service-only");
    }

    #[tokio::test]
    async fn empty_sts_401_is_an_actionable_error_without_credentials() {
        use axum::{http::StatusCode, routing::get, Router};
        let app = Router::new()
            .route(
                "/pass/serviceLogin",
                get(|| async { format!("&&&START&&&{}", challenge()) }),
            )
            .route("/sts", get(|| async { StatusCode::UNAUTHORIZED }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async {
            axum::serve(listener, app).await.unwrap();
        });
        let client = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let error = exchange_with(&client, "userId=123; passToken=account-only", Some(&origin))
            .await
            .unwrap_err();
        server.abort();
        assert!(error.contains("HTTP 401"));
        assert!(error.contains("重试"));
        for secret in ["account-only", "fixture", "clientSign", "test-security"] {
            assert!(!error.contains(secret));
        }
    }

    #[test]
    fn incomplete_account_response_never_becomes_a_service_session() {
        let mut data = challenge();
        data["code"] = 70016.into();
        assert!(signed_location(&data).is_err());
        for key in ["nonce", "ssecurity", "userId", "location"] {
            let mut data = challenge();
            data.as_object_mut().unwrap().remove(key);
            assert!(signed_location(&data).is_err());
        }
    }
}
