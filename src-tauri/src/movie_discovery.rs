//! Discover public film releases; saving a candidate is a separate user action.
use futures_util::{stream, StreamExt};
use reqwest::{redirect::Policy, Client};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::HashSet,
    sync::Mutex,
    time::{Duration, Instant, SystemTime},
};
use url::Url;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Candidate {
    pub title: String,
    pub uri: String,
    pub source: String,
    pub reason: String,
    #[serde(default)]
    pub license_url: String,
}

#[derive(Deserialize)]
struct SearchResult {
    response: SearchPage,
}
#[derive(Deserialize)]
struct SearchPage {
    docs: Vec<SearchDoc>,
}
#[derive(Deserialize)]
struct SearchDoc {
    identifier: String,
}
#[derive(Deserialize)]
struct Item {
    metadata: Value,
    #[serde(default)]
    files: Vec<ArchiveFile>,
    #[serde(default)]
    is_dark: bool,
}
#[derive(Deserialize)]
struct ArchiveFile {
    name: String,
    #[serde(default)]
    private: Value,
}

fn archive_url(segments: &[&str]) -> Url {
    let mut url = Url::parse("https://archive.org").expect("static Internet Archive origin");
    url.path_segments_mut().unwrap().extend(segments);
    url
}

fn archive_host(url: &Url) -> bool {
    url.scheme() == "https"
        && url.port_or_known_default() == Some(443)
        && url.username().is_empty()
        && url.password().is_none()
        && url
            .host_str()
            .is_some_and(|h| h == "archive.org" || h.ends_with(".archive.org"))
}

static RETRY_AT: Mutex<Option<Instant>> = Mutex::new(None);

pub fn retry_after_seconds() -> i64 {
    RETRY_AT
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .and_then(|t| t.checked_duration_since(Instant::now()))
        .map(|left| left.as_secs() as i64 + i64::from(left.subsec_nanos() > 0))
        .unwrap_or(0)
}

fn backoff(status: u16, value: Option<&str>, now: SystemTime) -> Option<Duration> {
    if status != 429 && !(status == 503 && value.is_some()) {
        return None;
    }
    let delay = value
        .and_then(|s| {
            s.trim()
                .parse::<u64>()
                .ok()
                .map(Duration::from_secs)
                .or_else(|| {
                    httpdate::parse_http_date(s)
                        .ok()
                        .map(|at| at.duration_since(now).unwrap_or_default())
                })
        })
        .unwrap_or(Duration::from_secs(60));
    Some(delay.clamp(Duration::from_secs(1), Duration::from_secs(86400)))
}

async fn fetch(client: &Client, url: Url, limit: usize) -> Result<Vec<u8>, String> {
    let delay = retry_after_seconds();
    if delay > 0 {
        return Err(format!(
            "Internet Archive 正在限流，请在 {delay} 秒后重试。"
        ));
    }
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|_| "无法连接 Internet Archive。")?;
    if let Some(delay) = backoff(
        response.status().as_u16(),
        response
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok()),
        SystemTime::now(),
    ) {
        let until = Instant::now() + delay;
        let mut gate = RETRY_AT.lock().unwrap_or_else(|e| e.into_inner());
        *gate = Some(gate.map_or(until, |old| old.max(until)));
        return Err(format!(
            "Internet Archive 暂缓请求，{} 秒后可重试。",
            delay.as_secs()
        ));
    }
    if !response.status().is_success() {
        return Err(format!(
            "Internet Archive 返回 HTTP {}。",
            response.status().as_u16()
        ));
    }
    if response.content_length().is_some_and(|n| n > limit as u64) {
        return Err("资源来源的响应超过大小限制。".into());
    }
    let mut chunks = response.bytes_stream();
    let mut body = Vec::new();
    while let Some(chunk) = chunks.next().await {
        let chunk = chunk.map_err(|_| "读取资源来源时连接中断。")?;
        if chunk.len() > limit - body.len() {
            return Err("资源来源的响应超过大小限制。".into());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn title_key(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn release_year(date: &str) -> Option<i32> {
    let y = date.get(..4)?;
    if date.len() > 4 && date.as_bytes()[4] != b'-' {
        return None;
    }
    y.parse().ok().filter(|year| (1800..=2200).contains(year))
}

fn license(value: &Value) -> Option<&str> {
    let text = value.as_str()?;
    let u = Url::parse(text).ok()?;
    if text.len() > 512
        || !matches!(u.scheme(), "http" | "https")
        || u.host_str() != Some("creativecommons.org")
        || !u.username().is_empty()
        || u.password().is_some()
        || u.port().is_some()
    {
        return None;
    }
    let parts = u.path().trim_matches('/').split('/').collect::<Vec<_>>();
    match parts.as_slice() {
        ["publicdomain", "zero" | "mark", "1.0", ..] => Some(text),
        ["licenses", "by" | "by-sa" | "by-nd" | "by-nc" | "by-nc-sa" | "by-nc-nd", "1.0" | "2.0" | "2.5" | "3.0" | "4.0", ..] => {
            Some(text)
        }
        _ => None,
    }
}

fn flag(value: &Value) -> bool {
    value == true
        || value
            .as_str()
            .is_some_and(|s| s == "1" || s.eq_ignore_ascii_case("true"))
}

fn title_matches(title: &str, names: &[&str], year: i32) -> bool {
    let bare = title.trim();
    let qualifier = format!("({year})");
    let bare = bare.strip_suffix(&qualifier).unwrap_or(bare);
    let key = title_key(bare);
    !key.is_empty() && names.iter().any(|name| title_key(name) == key)
}

struct Release<'a> {
    title: &'a str,
    torrent: &'a str,
    reason: String,
    license: &'a str,
}

fn inspect<'a>(item: &'a Item, names: &[&str], year: i32) -> Option<Release<'a>> {
    let meta = &item.metadata;
    if item.is_dark || flag(&meta["access-restricted-item"]) || meta["mediatype"] != "movies" {
        return None;
    }
    let title = meta["title"].as_str()?;
    if title.len() > 512 || !title_matches(title, names, year) {
        return None;
    }
    let date_year = meta["year"]
        .as_i64()
        .and_then(|n| i32::try_from(n).ok())
        .or_else(|| meta["year"].as_str().and_then(release_year))
        .or_else(|| meta["date"].as_str().and_then(release_year));
    let evidence = match date_year {
        Some(y) if y == year => format!("来源片名与年份 {year} 一致"),
        Some(_) => return None,
        None => "来源片名一致，缺少作品年份，需核对".into(),
    };
    let rights = &meta["licenseurl"];
    let rights = license(rights).or_else(|| rights.as_array()?.iter().find_map(license))?;
    // The autogenerated archive torrent is public and corresponds to this particular item.
    let id = meta["identifier"].as_str()?;
    let expected = format!("{id}_archive.torrent");
    let mut public = item.files.iter().filter(|f| !flag(&f.private));
    let file = public.clone().find(|f| f.name == expected).or_else(|| {
        public.find(|f| {
            !f.name.contains(['/', '\\'])
                && !f.name.chars().any(char::is_control)
                && f.name
                    .strip_suffix(".torrent")
                    .is_some_and(|name| title_matches(name, names, year))
        })
    })?;
    Some(Release {
        title,
        torrent: &file.name,
        reason: format!("{evidence}；来源许可 {rights}；节点状态未知。"),
        license: rights,
    })
}

fn torrent_magnet(raw: &[u8], title: &str) -> Option<String> {
    let t = librqbit::torrent_from_bytes(raw).ok()?;
    if t.info.data.private {
        return None;
    }
    let mut magnet = Url::parse("magnet:?").ok()?;
    magnet
        .query_pairs_mut()
        .append_pair("xt", &format!("urn:btih:{}", t.info_hash.as_string()))
        .append_pair("dn", title);
    for tracker in t.iter_announce().take(32) {
        let Some(url) = std::str::from_utf8(tracker.as_ref())
            .ok()
            .and_then(|s| Url::parse(s).ok())
        else {
            continue;
        };
        if matches!(url.scheme(), "http" | "https" | "udp")
            && url.host().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.as_str().len() <= 1024
        {
            // Keep discovered URIs acceptable to both existing downloaders.
            let mut next = magnet.clone();
            next.query_pairs_mut().append_pair("tr", url.as_str());
            if next.as_str().len() <= 8192 {
                magnet = next;
            }
        }
    }
    let info = t.info.data.validate().ok()?;
    let supports_video = info.iter_file_details().any(|f| {
        f.len > 0
            && !f.attrs().padding
            && !f.attrs().symlink
            && f.symlink_path.is_none()
            && f.filename
                .to_pathbuf()
                .extension()
                .and_then(|s| s.to_str())
                .is_some_and(|ext| {
                    ["mp4", "mkv", "m2ts"]
                        .iter()
                        .any(|v| ext.eq_ignore_ascii_case(v))
                })
    });
    supports_video.then_some(magnet.to_string())
}
fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && !id.starts_with('.')
        && id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.'))
}
async fn candidate(
    client: &Client,
    id: &str,
    names: &[&str],
    year: i32,
) -> Result<Option<Candidate>, String> {
    let raw = fetch(client, archive_url(&["metadata", id]), 2 * 1024 * 1024).await?;
    let item: Item = serde_json::from_slice(&raw).map_err(|_| "Archive 条目数据格式无效。")?;
    if item.metadata["identifier"].as_str() != Some(id) {
        return Ok(None);
    }
    let Some(release) = inspect(&item, names, year) else {
        return Ok(None);
    };
    let raw = fetch(
        client,
        archive_url(&["download", id, release.torrent]),
        4 * 1024 * 1024,
    )
    .await?;
    Ok(torrent_magnet(&raw, release.title).map(|uri| Candidate {
        uri,
        title: release.title.to_owned(),
        source: archive_url(&["details", id]).into(),
        reason: release.reason,
        license_url: release.license.to_owned(),
    }))
}

/// Dropping the future cancels pending HTTP operations. Work is bounded per source and per search.
pub async fn search(title: &str, original: &str, year: i32) -> Result<Vec<Candidate>, String> {
    let names = [original.trim(), title.trim()];
    let mut phrases = Vec::new();
    for name in names {
        if name.is_empty() {
            continue;
        }
        if name.len() > 512 || name.chars().any(char::is_control) {
            return Err("片名无效。".into());
        }
        let escaped = name.replace('\\', "\\\\").replace('"', "\\\"");
        let phrase = format!("title:\"{escaped}\"");
        if !phrases.contains(&phrase) {
            phrases.push(phrase);
        }
    }
    if phrases.is_empty() {
        return Err("请提供片名。".into());
    }
    let query = format!(
        "mediatype:movies AND licenseurl:* AND ({})",
        phrases.join(" OR ")
    );
    let mut url = archive_url(&["advancedsearch.php"]);
    url.query_pairs_mut()
        .append_pair("q", &query)
        .append_pair("fl[]", "identifier")
        .append_pair("rows", "12")
        .append_pair("output", "json");
    let client = Client::builder()
        .user_agent("SmartSoundDLNA movie catalog/0.1")
        .connect_timeout(Duration::from_secs(6))
        .timeout(Duration::from_secs(12))
        .redirect(Policy::custom(|attempt| {
            if archive_host(attempt.url()) && attempt.previous().len() < 5 {
                attempt.follow()
            } else {
                attempt.error("unsupported archive redirect")
            }
        }))
        .build()
        .map_err(|_| "无法初始化资源检索。")?;
    tokio::time::timeout(Duration::from_secs(45), async {
        let raw = fetch(&client, url, 256 * 1024).await?;
        let page: SearchResult =
            serde_json::from_slice(&raw).map_err(|_| "Archive 搜索结果格式无效。")?;
        let mut seen_ids = HashSet::new();
        let mut requests = Vec::new();
        for doc in page.response.docs.iter().take(12) {
            if valid_id(&doc.identifier) && seen_ids.insert(doc.identifier.as_str()) {
                requests.push(candidate(&client, &doc.identifier, &names, year));
            }
        }
        let mut work = stream::iter(requests).buffered(2);
        let mut found = Vec::new();
        let mut error = None;
        while let Some(result) = work.next().await {
            match result {
                Ok(Some(c)) => found.push(c),
                Err(e) => error = Some(e),
                Ok(None) => (),
            }
        }
        if let Some(error) = error {
            if found.is_empty() {
                return Err(error);
            }
            for c in &mut found {
                c.reason.push_str("另有条目读取失败，可稍后重试。");
            }
        }
        Ok(found)
    })
    .await
    .map_err(|_| "资源检索超时，请稍后重试。".to_string())?
}

#[cfg(test)]
mod tests;
