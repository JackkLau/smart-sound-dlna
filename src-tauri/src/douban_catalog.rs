//! Incremental refresh from the public movie charts. Each successful slice is replay-safe.
use crate::movie_library::{Movie, Store};
use chrono::{Datelike, Local, Utc};
use dom_query::Document;
use reqwest::{header::RETRY_AFTER, Client, StatusCode};
use serde::{Deserialize, Serialize};
use std::{io::Write, path::PathBuf, sync::Arc, time::Duration};
use tokio::sync::Mutex;

const GENRES: &[u8] = &[
    11, 24, 5, 13, 17, 25, 10, 19, 20, 1, 23, 6, 14, 7, 28, 8, 2, 4, 22, 3, 27, 16, 15, 12, 29, 30,
    18, 31,
];
const CHUNK: usize = 5;
const MAX_RESPONSE: usize = 3 * 1024 * 1024;
const SITE: &str = "https://movie.douban.com";

#[derive(Default, Serialize, Deserialize, Clone)]
struct Cursor {
    top_page: usize,
    genre: usize,
    band: u8,
    offset: usize,
    #[serde(default)]
    not_before: i64,
}

#[derive(Serialize)]
pub struct SyncReport {
    changed: usize,
    pages: usize,
    complete: bool,
    message: String,
}

#[derive(Serialize)]
pub struct RefreshReport {
    eligible: bool,
    message: String,
}

pub struct CatalogSync {
    cursor_file: PathBuf,
    lock: Mutex<()>,
}

struct FetchError {
    message: String,
    delay: i64,
}

impl From<String> for FetchError {
    fn from(message: String) -> Self {
        Self { message, delay: 3 }
    }
}

async fn fetch(client: &Client, url: &str) -> Result<Option<Vec<u8>>, FetchError> {
    let mut response = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("无法连接豆瓣，请检查网络后重试：{e}"))?;
    if response.status() == StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if response.status() == StatusCode::TOO_MANY_REQUESTS
        || response.status() == StatusCode::FORBIDDEN
    {
        let delay = response
            .headers()
            .get(RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| {
                v.parse::<i64>().ok().or_else(|| {
                    let date = httpdate::parse_http_date(v).ok()?;
                    Some(
                        date.duration_since(std::time::SystemTime::now())
                            .unwrap_or_default()
                            .as_secs()
                            .min(86400) as i64,
                    )
                })
            })
            .unwrap_or(60)
            .clamp(1, 86400);
        return Err(FetchError {
            message: format!("豆瓣暂时限制访问，请在 {delay} 秒后继续。"),
            delay,
        });
    }
    if response.status().is_redirection() {
        return Err(FetchError {
            message: "豆瓣页面要求跳转或验证，已保留本地评分。稍后可以继续更新。".into(),
            delay: 60,
        });
    }
    if !response.status().is_success() {
        return Err(format!("豆瓣返回 HTTP {}，保留本次进度。", response.status()).into());
    }
    let mut bytes = Vec::new();
    while let Some(part) = response.chunk().await.map_err(|e| e.to_string())? {
        if part.len() > MAX_RESPONSE - bytes.len() {
            return Err("豆瓣响应超过大小限制。".to_string().into());
        }
        bytes.extend_from_slice(&part);
    }
    Ok(Some(bytes))
}

fn subject_id(href: &str) -> Option<&str> {
    href.strip_prefix("https://movie.douban.com/subject/")?
        .strip_suffix('/')
        .filter(|s| !s.is_empty() && s.len() <= 16 && s.bytes().all(|c| c.is_ascii_digit()))
}

fn score(text: &str) -> Option<i32> {
    let (a, b) = text.trim().split_once('.').unwrap_or((text.trim(), "0"));
    if b.len() != 1 || !b.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let n = a
        .parse::<i32>()
        .ok()?
        .checked_mul(10)?
        .checked_add(b.parse::<i32>().ok()?)?;
    (0..=100).contains(&n).then_some(n)
}

fn first_year(text: &str) -> Option<i32> {
    text.split(|c: char| !c.is_ascii_digit())
        .filter(|s| s.len() == 4)
        .find_map(|s| s.parse::<i32>().ok().filter(|v| (1870..=2200).contains(v)))
}

fn top_movies(html: &str, today: &str) -> Result<Vec<Movie>, String> {
    let doc = Document::from(html);
    let mut movies = Vec::new();
    for row in doc.select("ol.grid_view .item").iter() {
        let a = row.select(".hd a");
        let href = a.attr("href").ok_or("榜单缺少电影链接。")?;
        let id = subject_id(&href).ok_or("榜单条目标识无效。")?;
        let mut names = a
            .select(".title")
            .iter()
            .map(|v| v.text().trim().trim_start_matches('/').trim().to_string())
            .collect::<Vec<_>>()
            .into_iter();
        let title = names
            .next()
            .filter(|s| !s.is_empty())
            .ok_or("榜单缺少片名。")?;
        let details = row.select_single(".bd p").inner_html();
        let year = details
            .split_once("<br>")
            .and_then(|(_, v)| first_year(v))
            .ok_or("榜单缺少上映年份。")?;
        let rating_tenths = score(&row.select(".rating_num").text()).ok_or("榜单缺少豆瓣评分。")?;
        movies.push(Movie {
            id: id.into(),
            title,
            original: names.next().unwrap_or_default(),
            year,
            rating_tenths,
            observed: today.into(),
            kind: "movie".into(),
            resource_count: 0,
            rating_evidence: None,
        });
    }
    if movies.len() != 25 {
        return Err("豆瓣榜单格式已变化或需要验证，已保留上次进度。".into());
    }
    Ok(movies)
}

#[derive(Deserialize)]
struct ChartMovie {
    id: String,
    title: String,
    score: String,
}

fn detail_movie(html: &str, record: &ChartMovie, today: &str) -> Result<Option<Movie>, String> {
    let doc = Document::from(html);
    let structured = doc.select("script[type='application/ld+json']").text();
    let data: serde_json::Value = serde_json::from_str(structured.trim())
        .map_err(|_| "豆瓣条目未返回电影结构化数据，请稍后继续。")?;
    match data["@type"].as_str() {
        Some("Movie") => (),
        Some("TVSeries" | "TVEpisode" | "TVSeason") => return Ok(None),
        _ => return Err("豆瓣条目类型无法确认。".into()),
    }
    let year = first_year(&doc.select("#content h1 .year").text())
        .or_else(|| data["datePublished"].as_str().and_then(first_year))
        .ok_or("无法确认电影的上映年份。")?;
    let reviewed = doc.select("h1 [property='v:itemreviewed']").text();
    let original = reviewed
        .trim()
        .strip_prefix(&record.title)
        .unwrap_or("")
        .trim()
        .to_string();
    let rating = doc.select("strong.rating_num").text();
    let rating_tenths = score(&rating)
        .or_else(|| {
            data["aggregateRating"]["ratingValue"]
                .as_f64()
                .and_then(|v| score(&v.to_string()))
        })
        .or_else(|| {
            let state = doc.select("#interest_sectl").text();
            (state.contains("暂无评分") || state.contains("尚未上映")).then_some(0)
        })
        .ok_or("无法确认豆瓣条目评分。")?;
    Ok(Some(Movie {
        id: record.id.clone(),
        title: record.title.clone(),
        original,
        year,
        rating_tenths,
        observed: today.into(),
        kind: "movie".into(),
        resource_count: 0,
        rating_evidence: None,
    }))
}

impl Cursor {
    fn next_band(&mut self) {
        self.offset = 0;
        self.band += 1;
        if self.band >= 10 {
            self.next_genre();
        }
    }
    fn next_genre(&mut self) {
        self.offset = 0;
        self.band = 0;
        self.genre += 1;
    }

    async fn slice(
        &mut self,
        client: &Client,
        store: &Store,
        today: &str,
    ) -> Result<Vec<Movie>, FetchError> {
        if self.top_page < 10 {
            let url = format!("{SITE}/top250?start={}", self.top_page * 25);
            let bytes = fetch(client, &url)
                .await?
                .ok_or_else(|| "豆瓣榜单页面不存在。".to_string())?;
            let movies = top_movies(&String::from_utf8_lossy(&bytes), today)?;
            self.top_page += 1;
            return Ok(movies);
        }
        let high = 100 - self.band * 10;
        let url=format!("{SITE}/j/chart/top_list?type={}&interval_id={high}%3A{}&action=&start={}&limit={CHUNK}",GENRES[self.genre],high-10,self.offset);
        let bytes = fetch(client, &url)
            .await?
            .ok_or_else(|| "豆瓣分类榜入口暂不可用。".to_string())?;
        let rows: Vec<ChartMovie> = serde_json::from_slice(&bytes)
            .map_err(|_| "豆瓣分类榜未返回有效电影清单。".to_string())?;
        if rows.len() > CHUNK {
            return Err("豆瓣分类榜分页异常。".to_string().into());
        }
        let mut movies = Vec::new();

        for item in &rows {
            if item.id.is_empty()
                || item.id.len() > 16
                || !item.id.bytes().all(|x| x.is_ascii_digit())
            {
                return Err("豆瓣电影编号无效。".to_string().into());
            }
            let rating =
                score(&item.score).ok_or_else(|| "豆瓣分类榜评分格式无效。".to_string())?;
            // Scan every available page; chart rank is not an exact score ordering.
            let previous = store.get(&item.id).ok();
            if let Some(mut m) = previous {
                m.rating_tenths = rating;
                m.observed = today.into();
                movies.push(m);
            } else if rating >= 80 {
                tokio::time::sleep(Duration::from_millis(750)).await;
                let Some(detail) = fetch(client, &format!("{SITE}/subject/{}/", item.id)).await?
                else {
                    continue;
                };
                if let Some(movie) = detail_movie(&String::from_utf8_lossy(&detail), item, today)? {
                    movies.push(movie);
                }
            }
        }
        if rows.len() < CHUNK {
            self.next_band();
        } else {
            self.offset += rows.len();
        }
        Ok(movies)
    }
}

impl CatalogSync {
    pub fn new(cursor_file: PathBuf) -> Self {
        Self {
            cursor_file,
            lock: Mutex::new(()),
        }
    }
    fn save(&self, c: &Cursor) -> Result<(), String> {
        let directory = self.cursor_file.parent().ok_or("片库同步路径无效。")?;
        let mut tmp = tempfile::NamedTempFile::new_in(directory).map_err(|e| e.to_string())?;
        serde_json::to_writer(&mut tmp, c).map_err(|e| e.to_string())?;
        tmp.flush()
            .and_then(|()| tmp.as_file().sync_all())
            .map_err(|e| e.to_string())?;
        tmp.persist(&self.cursor_file).map_err(|e| e.to_string())?;
        Ok(())
    }
    fn ready_cursor(&self) -> Result<Cursor, String> {
        let cursor: Cursor = match std::fs::read(&self.cursor_file) {
            Ok(b) => serde_json::from_slice(&b).map_err(|_| "同步进度文件损坏。")?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Cursor::default(),
            Err(e) => return Err(e.to_string()),
        };
        if cursor.top_page > 10 || cursor.genre > GENRES.len() || cursor.band >= 10 {
            return Err("同步进度无效。".into());
        }
        let now = Utc::now().timestamp();
        if now < cursor.not_before {
            return Err(format!("请在 {} 秒后继续更新。", cursor.not_before - now));
        }
        Ok(cursor)
    }

    pub async fn refresh(&self, store: Arc<Store>, id: String) -> Result<RefreshReport, String> {
        let _guard = self.lock.try_lock().map_err(|_| "豆瓣更新已在进行。")?;
        let mut cursor = self.ready_cursor()?;
        let previous = store.get(&id)?;
        let today = Local::now().date_naive();
        let raw = match fetch(&Self::client()?, &format!("{SITE}/subject/{id}/")).await {
            Ok(raw) => raw,
            Err(error) => {
                cursor.not_before = Utc::now().timestamp() + error.delay;
                self.save(&cursor)?;
                return Err(error.message);
            }
        };
        cursor.not_before = Utc::now().timestamp() + 2;
        self.save(&cursor)?;
        let bytes = raw.ok_or("豆瓣原始条目已找不到，保留上次评分。")?;
        let record = ChartMovie {
            id,
            title: previous.title.clone(),
            score: String::new(),
        };
        let movie = detail_movie(
            &String::from_utf8_lossy(&bytes),
            &record,
            &today.to_string(),
        )?
        .unwrap_or_else(|| Movie {
            kind: "tv".into(),
            observed: today.to_string(),
            ..previous
        });
        let eligible = movie.kind == "movie"
            && (1990..=today.year()).contains(&movie.year)
            && movie.rating_tenths >= 80;
        let message = if eligible {
            format!(
                "已更新《{}》的豆瓣评分：{:.1}。",
                movie.title,
                movie.rating_tenths as f32 / 10.
            )
        } else {
            format!(
                "《{}》的最新条目不符合收录条件，已从片库列表移除。",
                movie.title
            )
        };
        let json = serde_json::to_string(&[movie]).map_err(|e| e.to_string())?;
        tokio::task::spawn_blocking(move || store.import(&json))
            .await
            .map_err(|e| e.to_string())??;
        Ok(RefreshReport { eligible, message })
    }

    fn client() -> Result<Client, String> {
        Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(12))
            .user_agent("smart-sound-dlna/movie-library")
            .build()
            .map_err(|e| e.to_string())
    }

    pub async fn advance(&self, store: Arc<Store>) -> Result<SyncReport, String> {
        let _guard = self.lock.try_lock().map_err(|_| "豆瓣更新已在进行。")?;
        let mut cursor = self.ready_cursor()?;
        if cursor.genre == GENRES.len() {
            cursor = Cursor::default();
        }
        let client = Self::client()?;
        let today = Local::now().format("%Y-%m-%d").to_string();
        let mut next = cursor.clone();
        let movies = match next.slice(&client, &store, &today).await {
            Ok(m) => m,
            Err(e) => {
                cursor.not_before = Utc::now().timestamp() + e.delay;
                self.save(&cursor)?;
                return Err(e.message);
            }
        };
        let json = serde_json::to_string(&movies).map_err(|e| e.to_string())?;
        let changed = tokio::task::spawn_blocking(move || store.import(&json))
            .await
            .map_err(|e| e.to_string())??;
        next.not_before = Utc::now().timestamp() + 2;
        self.save(&next)?;
        let complete = next.genre == GENRES.len();
        let message = if complete {
            "已遍历豆瓣公开分类榜。".to_string()
        } else if next.top_page < 10 {
            format!("Top 250 已更新 {}/10 页。", next.top_page)
        } else {
            format!(
                "Top 250 已更新；已处理 {}/{} 个分类。",
                next.genre,
                GENRES.len()
            )
        };
        Ok(SyncReport {
            changed,
            pages: 1,
            complete,
            message,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chart_preserves_original_release_year_and_rating_boundary() {
        let row = r#"<li><div class="item"><div class="hd"><a href="https://movie.douban.com/subject/1234/"><span class="title">电影</span><span class="title"> / Film</span></a></div><div class="bd"><p>导演 / 主演<br>1990(首映) / 2025(重映) / 地区</p><div class="rating_num">8.0</div></div></div></li>"#;
        let movies = top_movies(
            &format!("<ol class='grid_view'>{}</ol>", row.repeat(25)),
            "2026-11-27",
        )
        .unwrap();
        assert_eq!(
            (
                movies[0].year,
                movies[0].rating_tenths,
                movies[0].original.as_str()
            ),
            (1990, 80, "Film")
        );
        assert!(top_movies("请先验证", "2026-11-27").is_err());
    }
    #[test]
    fn cursor_keeps_the_previous_page_during_cooldown() {
        let dir = tempfile::tempdir().unwrap();
        let sync = CatalogSync::new(dir.path().join("cursor.json"));
        let mut c = Cursor {
            top_page: 3,
            ..Cursor::default()
        };
        sync.save(&c).unwrap();
        assert_eq!(sync.ready_cursor().unwrap().top_page, 3);
        c.not_before = Utc::now().timestamp() + 1800;
        sync.save(&c).unwrap();
        assert!(sync.ready_cursor().is_err());
    }

    #[test]
    fn genre_entry_uses_subject_year_and_rejects_tv() {
        let item = ChartMovie {
            id: "123".into(),
            title: "测试".into(),
            score: "8.1".into(),
        };
        let html = r#"<div id="content"><h1><span property="v:itemreviewed">测试 Test</span><span class="year">(1990)</span></h1></div><script type="application/ld+json">{"@type":"Movie","datePublished":"2025","aggregateRating":{"ratingValue":8.0}}</script>"#;
        let film = detail_movie(html, &item, "2026-11-27").unwrap().unwrap();
        assert_eq!(
            (film.year, film.original.as_str(), film.rating_tenths),
            (1990, "Test", 80)
        );
        assert!(
            detail_movie(&html.replace("Movie", "TVSeries"), &item, "2026-11-27")
                .unwrap()
                .is_none()
        );
    }
}
