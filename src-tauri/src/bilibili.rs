//! Native Bilibili extraction. Only separate AAC audio is downloaded, never video.
use crate::{engine::Engine, library::TrackSource, online};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap},
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{Arc, Weak},
    time::{Duration, Instant},
};
use tokio::{
    io::AsyncWriteExt,
    sync::{Mutex, Semaphore},
};
use tokio_util::sync::CancellationToken;
use url::Url;

#[cfg(test)]
mod anonymous_tests;
mod favorites;
pub use favorites::{BatchResult, BatchSelection, FavoriteFolder, FavoritePage};

const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";
const MAX_AUDIO: u64 = 1024 * 1024 * 1024;
const PREVIEW_TTL: u64 = 30 * 60 * 1000;
const MAX_QUEUE: usize = 100;

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Part {
    pub cid: u64,
    pub page: u32,
    pub title: String,
    pub duration: u64,
}
#[derive(Clone, Serialize, Debug)]
pub struct Preview {
    pub id: String,
    pub bvid: String,
    pub title: String,
    pub author: String,
    pub parts: Vec<Part>,
    pub selected_page: u32,
    #[serde(skip)]
    created: u64,
}
#[derive(Clone, Serialize, Debug)]
pub struct DownloadJob {
    pub id: String,
    pub title: String,
    pub phase: String,
    pub downloaded: u64,
    pub total: Option<u64>,
    pub error: Option<String>,
    pub track_id: Option<String>,
    pub path: Option<String>,
}
#[derive(Clone)]
struct JobEntry {
    view: DownloadJob,
    preview: Preview,
    part: Part,
    cancel: CancellationToken,
}
#[derive(Serialize)]
pub struct DownloadState {
    pub jobs: Vec<DownloadJob>,
    pub directory: String,
    pub authenticated: bool,
}
pub struct Downloader {
    directory: PathBuf,
    client: reqwest::Client,
    cookie: Mutex<String>,
    visitor_cookie: Mutex<String>,
    wbi: Mutex<Option<(Instant, String)>>,
    previews: Mutex<HashMap<String, Preview>>,
    jobs: Mutex<Vec<JobEntry>>,
    queue: Semaphore,
    #[cfg(test)]
    test_origin: Option<Url>,
}

impl Downloader {
    pub fn new(data_dir: &Path) -> Result<Arc<Self>, String> {
        Ok(Arc::new(Self {
            directory: data_dir.join("downloads"),
            client: online::client(UA)?,
            cookie: Mutex::new(String::new()),
            visitor_cookie: Mutex::new(String::new()),
            wbi: Mutex::new(None),
            previews: Mutex::new(HashMap::new()),
            jobs: Mutex::new(Vec::new()),
            queue: Semaphore::new(1),
            #[cfg(test)]
            test_origin: None,
        }))
    }
    pub async fn state(&self) -> DownloadState {
        DownloadState {
            jobs: self
                .jobs
                .lock()
                .await
                .iter()
                .map(|j| j.view.clone())
                .collect(),
            directory: self.directory.to_string_lossy().into_owned(),
            authenticated: !self.cookie.lock().await.is_empty(),
        }
    }
    pub async fn set_session(&self, raw: &str) -> Result<(), String> {
        let cookie = online::cookies(
            raw,
            &["SESSDATA", "bili_jct", "buvid3", "buvid4", "DedeUserID"],
        )?;
        if !cookie.starts_with("SESSDATA=") {
            return Err("尚未完成 Bilibili 登录，请登录后重试。".into());
        }
        let data = self.api("/x/web-interface/nav", &[], &cookie).await?;
        if data["data"]["isLogin"].as_bool() != Some(true) {
            return Err("Bilibili 登录会话已过期，请重新登录。".into());
        }
        *self.cookie.lock().await = cookie;
        Ok(())
    }
    pub async fn disconnect(&self) {
        self.cookie.lock().await.clear();
    }

    /// Server-issued visitor identifiers are not an account session. Keep them
    /// separate so anonymous imports never claim login or inherit account data.
    async fn request_cookie(&self) -> Result<String, String> {
        let account = self.cookie.lock().await.clone();
        if !account.is_empty() {
            return Ok(account);
        }
        let mut visitor = self.visitor_cookie.lock().await;
        if visitor.is_empty() {
            let response = self.api("/x/frontend/finger/spi", &[], "").await?;
            check_code(&response)?;
            let value = |key: &str| {
                response["data"][key]
                    .as_str()
                    .filter(|s| {
                        !s.is_empty()
                            && s.len() <= 512
                            && s.bytes().all(|b| b.is_ascii_graphic() && b != b';')
                    })
                    .ok_or("Bilibili 匿名访客信息无效，请稍后重试。")
            };
            let raw = format!("buvid3={}; buvid4={}", value("b_3")?, value("b_4")?);
            *visitor = online::cookies(&raw, &["buvid3", "buvid4"])
                .map_err(|_| "Bilibili 匿名访客信息无效，请稍后重试。")?;
        }
        Ok(visitor.clone())
    }

    async fn signed_api(
        &self,
        path: &str,
        params: BTreeMap<String, String>,
        cookie: &str,
    ) -> Result<Value, String> {
        // The public key is present even when nav reports anonymous code -101.
        // Serialize refreshes and briefly cache it across previews and downloads.
        let key = {
            let mut cached = self.wbi.lock().await;
            if let Some((_, key)) = cached
                .as_ref()
                .filter(|(created, _)| created.elapsed() < Duration::from_secs(30))
            {
                key.clone()
            } else {
                let nav = self.api("/x/web-interface/nav", &[], cookie).await?;
                let key = wbi_key(&nav["data"]["wbi_img"])?;
                *cached = Some((Instant::now(), key.clone()));
                key
            }
        };
        self.api(
            path,
            &sign_wbi(params, &key, online::now_ms() / 1000),
            cookie,
        )
        .await
    }

    fn endpoint(&self, url: Url) -> Url {
        #[cfg(test)]
        if let Some(origin) = &self.test_origin {
            return origin
                .join(&format!(
                    "{}{}",
                    url.path(),
                    url.query().map(|q| format!("?{q}")).unwrap_or_default()
                ))
                .unwrap();
        }
        url
    }

    async fn api(
        &self,
        path: &str,
        params: &[(String, String)],
        cookie: &str,
    ) -> Result<Value, String> {
        let request = self
            .client
            .get(
                self.endpoint(
                    Url::parse(&format!("https://api.bilibili.com{path}"))
                        .map_err(|_| "解析服务地址无效")?,
                ),
            )
            .query(params)
            .header(reqwest::header::REFERER, "https://www.bilibili.com/")
            .header(reqwest::header::ORIGIN, "https://www.bilibili.com")
            .timeout(Duration::from_secs(25));
        let request = if cookie.is_empty() {
            request
        } else {
            request.header(reqwest::header::COOKIE, cookie)
        };
        let response = request
            .send()
            .await
            .map_err(|_| "Bilibili 连接失败，请检查网络后重试。")?;
        if matches!(response.status().as_u16(), 403 | 412 | 429) {
            return Err("Bilibili 请求暂时受限，请稍后重试。".into());
        }
        online::json(response).await
    }

    pub async fn parse(&self, input: &str) -> Result<Preview, String> {
        let mut input = parse_input(input)?;
        if let Input::Short(url) = input {
            let response = self
                .client
                .get(url.clone())
                .timeout(Duration::from_secs(15))
                .send()
                .await
                .map_err(|_| "Bilibili 短链接无法打开。")?;
            if !response.status().is_redirection() {
                return Err("该短链接未指向视频，请复制完整的视频地址。".into());
            }
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|h| h.to_str().ok())
                .ok_or("短链接缺少跳转地址。")?;
            let url = url.join(location).map_err(|_| "短链接跳转地址无效")?;
            input = parse_input(url.as_str())?;
            if matches!(input, Input::Short(_)) {
                return Err("短链接跳转次数过多，请使用完整视频链接。".into());
            }
        }
        let Input::Video { key, value, page } = input else {
            unreachable!()
        };
        let cookie = self.request_cookie().await?;
        let data = self
            .signed_api(
                "/x/web-interface/wbi/view/detail",
                BTreeMap::from([(key.into(), value), ("platform".into(), "web".into())]),
                &cookie,
            )
            .await?;
        check_code(&data)?;
        let mut preview = parse_preview(&data["data"]["View"], page)?;
        preview.id = uuid::Uuid::new_v4().to_string();
        let mut previews = self.previews.lock().await;
        previews.retain(|_, p| online::now_ms().saturating_sub(p.created) < PREVIEW_TTL);
        if previews.len() >= 128 {
            if let Some(oldest) = previews
                .values()
                .min_by_key(|p| p.created)
                .map(|p| p.id.clone())
            {
                previews.remove(&oldest);
            }
        }
        previews.insert(preview.id.clone(), preview.clone());
        Ok(preview)
    }

    pub async fn start(
        self: &Arc<Self>,
        engine: Weak<Engine>,
        preview_id: &str,
        cid: u64,
        shutdown: CancellationToken,
    ) -> Result<DownloadJob, String> {
        let preview = self
            .previews
            .lock()
            .await
            .get(preview_id)
            .cloned()
            .filter(|p| online::now_ms().saturating_sub(p.created) < PREVIEW_TTL)
            .ok_or("预览已过期，请重新解析链接。")?;
        let part = preview
            .parts
            .iter()
            .find(|p| p.cid == cid)
            .cloned()
            .ok_or("请选择有效的视频分 P。")?;
        self.enqueue(engine, preview, part, shutdown).await
    }
    pub async fn retry(
        self: &Arc<Self>,
        engine: Weak<Engine>,
        id: &str,
        shutdown: CancellationToken,
    ) -> Result<DownloadJob, String> {
        let entry = self
            .jobs
            .lock()
            .await
            .iter()
            .find(|j| j.view.id == id)
            .cloned()
            .ok_or("下载记录已不存在，请重新解析。")?;
        if !["failed", "cancelled"].contains(&entry.view.phase.as_str()) {
            return Err("仅未完成的任务可重试。".into());
        }
        self.enqueue(engine, entry.preview, entry.part, shutdown)
            .await
    }
    async fn enqueue(
        self: &Arc<Self>,
        engine: Weak<Engine>,
        preview: Preview,
        part: Part,
        shutdown: CancellationToken,
    ) -> Result<DownloadJob, String> {
        let source_id = format!("{}:{}", preview.bvid, part.cid);
        if let Some(active) = engine.upgrade() {
            if active.snapshot().await?.tracks.iter().any(|t| {
                t.source
                    .as_ref()
                    .is_some_and(|s| s.provider == "bilibili" && s.id == source_id)
            }) {
                return Err("这个分 P 已在曲库中，可直接搜索播放。".into());
            }
        } else {
            return Err("客户端正在退出".into());
        }
        let mut jobs = self.jobs.lock().await;
        if jobs.iter().any(|j| {
            j.preview.bvid == preview.bvid && j.part.cid == part.cid && active_phase(&j.view.phase)
        }) {
            return Err("这个分 P 已在下载队列中。".into());
        }
        // A worker may finish between the library snapshot and this lock.
        if jobs.iter().any(|j| {
            j.preview.bvid == preview.bvid && j.part.cid == part.cid && j.view.phase == "completed"
        }) {
            // Completed jobs are history: a user may since have removed the
            // track. Recheck the library under this lock to retain race safety.
            let active = engine.upgrade().ok_or("客户端正在退出")?;
            if active.snapshot().await?.tracks.iter().any(|t| {
                t.source
                    .as_ref()
                    .is_some_and(|s| s.provider == "bilibili" && s.id == source_id)
            }) {
                return Err("这个分 P 已在曲库中，可直接搜索播放。".into());
            }
        }
        if jobs.iter().filter(|j| active_phase(&j.view.phase)).count() >= MAX_QUEUE {
            return Err("最多排队 100 首，请等待部分任务完成后重试。".into());
        }
        if jobs.len() >= 200 {
            if let Some(index) = jobs.iter().position(|j| !active_phase(&j.view.phase)) {
                jobs.remove(index);
            }
        }
        let title = if preview.parts.len() == 1 {
            preview.title.clone()
        } else {
            part.title.clone()
        };
        let view = DownloadJob {
            id: uuid::Uuid::new_v4().to_string(),
            title,
            phase: "queued".into(),
            downloaded: 0,
            total: None,
            error: None,
            track_id: None,
            path: None,
        };
        let cancel = shutdown.child_token();
        let entry = JobEntry {
            view: view.clone(),
            preview,
            part,
            cancel,
        };
        jobs.push(entry.clone());
        drop(jobs);
        let downloader = self.clone();
        tokio::spawn(async move {
            let result = downloader.run(&engine, &entry).await;
            let mut jobs = downloader.jobs.lock().await;
            if let Some(job) = jobs.iter_mut().find(|j| j.view.id == entry.view.id) {
                match result {
                    Ok((track, path)) => {
                        job.view.phase = "completed".into();
                        job.view.track_id = Some(track);
                        job.view.path = Some(path);
                    }
                    Err(error) => {
                        job.view.phase =
                            if entry.cancel.is_cancelled() && job.view.phase != "importing" {
                                "cancelled"
                            } else {
                                "failed"
                            }
                            .into();
                        job.view.error = if job.view.phase == "cancelled" {
                            None
                        } else {
                            Some(error)
                        };
                    }
                }
            }
        });
        Ok(view)
    }
    pub async fn cancel(&self, id: &str) -> Result<(), String> {
        let mut jobs = self.jobs.lock().await;
        let job = jobs
            .iter_mut()
            .find(|j| j.view.id == id)
            .ok_or("下载任务不存在。")?;
        if job.view.phase == "importing" {
            return Err("音频已下载，正在保存到曲库，请稍候。".into());
        }
        if active_phase(&job.view.phase) {
            job.cancel.cancel();
            job.view.phase = "cancelling".into();
        }
        Ok(())
    }
    async fn update(&self, id: &str, phase: &str, downloaded: u64, total: Option<u64>) {
        if let Some(job) = self.jobs.lock().await.iter_mut().find(|j| j.view.id == id) {
            if !job.cancel.is_cancelled() {
                job.view.phase = phase.into();
            }
            job.view.downloaded = downloaded;
            job.view.total = total;
        }
    }

    async fn run(
        &self,
        engine: &Weak<Engine>,
        entry: &JobEntry,
    ) -> Result<(String, String), String> {
        let _permit = tokio::select! { _ = entry.cancel.cancelled() => return Err("已取消".into()), p = self.queue.acquire() => p.map_err(|_| "下载队列已关闭")? };
        self.update(&entry.view.id, "resolving", 0, None).await;
        let cookie = tokio::select! {
            _ = entry.cancel.cancelled() => return Err("已取消".into()),
            cookie = self.request_cookie() => cookie?,
        };
        let audio = tokio::select! {
            _ = entry.cancel.cancelled() => return Err("已取消".into()),
            audio = self.audio(&entry.preview.bvid, entry.part.cid, &cookie) => audio?,
        };
        let directory = self.directory.clone();
        let temp =
            tokio::task::spawn_blocking(move || -> Result<tempfile::NamedTempFile, String> {
                std::fs::create_dir_all(&directory)
                    .map_err(|e| format!("无法创建下载目录：{e}"))?;
                if std::fs::symlink_metadata(&directory)
                    .map_err(|e| e.to_string())?
                    .file_type()
                    .is_symlink()
                {
                    return Err("下载目录不能是符号链接。".into());
                }
                tempfile::Builder::new()
                    .prefix(".audio-")
                    .suffix(".part")
                    .tempfile_in(directory)
                    .map_err(|e| format!("无法创建下载文件：{e}"))
            })
            .await
            .map_err(|e| e.to_string())??;
        let mut output = tokio::fs::File::from_std(temp.reopen().map_err(|e| e.to_string())?);
        let mut response = None;
        for address in audio.urls {
            let attempt = tokio::select! {
                _ = entry.cancel.cancelled() => return Err("已取消".into()),
                response = self.audio_response(&address) => response,
            };
            if let Ok(result) = attempt {
                response = Some(result);
                break;
            }
        }
        let response = response.ok_or("音轨地址已过期或无法下载，请重试。")?;
        let total = response.content_length();
        if total.is_some_and(|n| n > MAX_AUDIO) {
            return Err("单个音频超过 1 GB，请选择较短的分 P。".into());
        }
        let mut stream = response.bytes_stream();
        let mut count = 0u64;
        self.update(&entry.view.id, "downloading", 0, total).await;
        loop {
            let chunk = tokio::select! { _ = entry.cancel.cancelled() => return Err("已取消".into()), chunk = stream.next() => chunk };
            let Some(chunk) = chunk else {
                break;
            };
            let chunk = chunk.map_err(|_| "下载连接中断，可重试此任务。")?;
            count += chunk.len() as u64;
            if count > MAX_AUDIO {
                return Err("音频超过 1 GB，已停止下载。".into());
            }
            output
                .write_all(&chunk)
                .await
                .map_err(|e| format!("保存音频失败，请检查磁盘空间：{e}"))?;
            self.update(&entry.view.id, "downloading", count, total)
                .await;
        }
        if total.is_some_and(|n| n != count) {
            return Err("音频下载不完整，请重试。".into());
        }
        output.flush().await.map_err(|e| e.to_string())?;
        output.sync_all().await.map_err(|e| e.to_string())?;
        drop(output);
        let mut check_file = temp.reopen().map_err(|e| e.to_string())?;
        tokio::task::spawn_blocking(move || validate_mp4(&mut check_file))
            .await
            .map_err(|e| e.to_string())??;
        // Cancellation and committing are ordered under the same job lock.
        {
            let mut jobs = self.jobs.lock().await;
            if entry.cancel.is_cancelled() {
                return Err("已取消".into());
            }
            if let Some(job) = jobs.iter_mut().find(|j| j.view.id == entry.view.id) {
                job.view.phase = "importing".into();
            }
        }
        let filename = format!(
            "{} [{}-{}].m4a",
            safe_title(&entry.view.title),
            entry.preview.bvid,
            entry.part.cid
        );
        let path = self.directory.join(filename);
        let publish_path = path.clone();
        let (path, owned) =
            tokio::task::spawn_blocking(move || -> Result<(PathBuf, bool), String> {
                match temp.persist_noclobber(&publish_path) {
                    Ok(_) => Ok((publish_path, true)),
                    Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
                        // A previous process may have exited between publish and indexing.
                        if std::fs::symlink_metadata(&publish_path)
                            .map_err(|e| e.to_string())?
                            .file_type()
                            .is_symlink()
                        {
                            return Err("同名下载路径是链接，已停止保存。".into());
                        }
                        validate_mp4(
                            &mut std::fs::File::open(&publish_path).map_err(|e| e.to_string())?,
                        )?;
                        Ok((publish_path, false))
                    }
                    Err(error) => Err(format!("无法保存下载文件：{}", error.error)),
                }
            })
            .await
            .map_err(|e| e.to_string())??;
        let source = TrackSource {
            provider: "bilibili".into(),
            id: format!("{}:{}", entry.preview.bvid, entry.part.cid),
            url: format!(
                "https://www.bilibili.com/video/{}?p={}",
                entry.preview.bvid, entry.part.page
            ),
        };
        let result = match engine.upgrade() {
            Some(engine) => {
                engine
                    .import_download(path.clone(), entry.view.title.clone(), source)
                    .await
            }
            None => Err("客户端已退出，下次可重新导入已下载文件。".into()),
        };
        match result {
            Ok(track) => Ok((track.id, path.to_string_lossy().into_owned())),
            Err(error) => {
                if owned {
                    let _ = tokio::fs::remove_file(path).await;
                }
                Err(error)
            }
        }
    }
    async fn audio(&self, bvid: &str, cid: u64, cookie: &str) -> Result<Audio, String> {
        let params = BTreeMap::from([
            ("bvid".into(), bvid.into()),
            ("cid".into(), cid.to_string()),
            ("fnval".into(), "16".into()),
            ("fnver".into(), "0".into()),
            ("fourk".into(), "0".into()),
        ]);
        let data = self
            .signed_api("/x/player/wbi/playurl", params, cookie)
            .await?;
        check_code(&data)?;
        if data["data"]["is_preview"].as_bool() == Some(true) {
            return Err("该内容仅提供试听，请在 Bilibili 确认访问权限。".into());
        }
        select_audio(&data["data"])
    }
    async fn audio_response(&self, address: &str) -> Result<reqwest::Response, String> {
        let mut url = audio_url(address)?;
        for _ in 0..4 {
            let response = self
                .client
                .get(self.endpoint(url.clone()))
                .header(reqwest::header::REFERER, "https://www.bilibili.com/")
                .send()
                .await
                .map_err(|_| "音轨下载连接失败")?;
            if response.status().is_redirection() {
                let location = response
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|h| h.to_str().ok())
                    .ok_or("音轨跳转地址缺失")?;
                url = audio_url(url.join(location).map_err(|_| "音轨跳转无效")?.as_str())?;
                continue;
            }
            if response.status() != reqwest::StatusCode::OK {
                return Err("音轨服务暂时不可用".into());
            }
            return Ok(response);
        }
        Err("音轨跳转次数过多".into())
    }
}

fn active_phase(phase: &str) -> bool {
    !["completed", "failed", "cancelled"].contains(&phase)
}
#[derive(Debug, PartialEq)]
enum Input {
    Video {
        key: &'static str,
        value: String,
        page: u32,
    },
    Short(Url),
}
fn parse_input(raw: &str) -> Result<Input, String> {
    let raw = raw.trim();
    if raw.len() > 2048 {
        return Err("链接过长，请复制视频页的完整地址。".into());
    }
    if valid_bvid(raw) {
        return Ok(Input::Video {
            key: "bvid",
            value: raw.into(),
            page: 1,
        });
    }
    if let Some(id) = raw
        .strip_prefix("av")
        .filter(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
    {
        return Ok(Input::Video {
            key: "aid",
            value: id.into(),
            page: 1,
        });
    }
    let raw = if raw.starts_with("b23.tv/") || raw.starts_with("www.bilibili.com/") {
        format!("https://{raw}")
    } else {
        raw.into()
    };
    let mut url = Url::parse(&raw).map_err(|_| "请输入 Bilibili 视频链接、BV 号或 av 号。")?;
    if !["http", "https"].contains(&url.scheme())
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
    {
        return Err("仅支持 Bilibili 官方视频链接。".into());
    }
    let _ = url.set_scheme("https");
    if url.host_str() == Some("b23.tv") {
        return Ok(Input::Short(url));
    }
    if ![
        Some("www.bilibili.com"),
        Some("bilibili.com"),
        Some("m.bilibili.com"),
    ]
    .contains(&url.host_str())
    {
        return Err("仅支持 bilibili.com 或 b23.tv 的视频链接。".into());
    }
    let mut segments: Vec<_> = url.path_segments().ok_or("视频链接无效")?.collect();
    if matches!(segments.first(), Some(&"zh-Hans") | Some(&"zh-Hant")) {
        segments.remove(0);
    }
    if segments.len() < 2 || segments[0] != "video" {
        return Err(
            "请使用普通投稿视频链接；收藏夹请切换到“收藏夹”入口，暂不支持番剧或直播。".into(),
        );
    }
    let page = url
        .query_pairs()
        .find(|(k, _)| k == "p")
        .map(|(_, v)| v.parse::<u32>())
        .transpose()
        .map_err(|_| "分 P 编号无效")?
        .unwrap_or(1);
    if page == 0 {
        return Err("分 P 编号从 1 开始。".into());
    }
    match parse_input(segments[1])? {
        Input::Video { key, value, .. } => Ok(Input::Video { key, value, page }),
        _ => Err("视频编号无效".into()),
    }
}
fn valid_bvid(s: &str) -> bool {
    s.len() == 12 && s.starts_with("BV") && s.bytes().all(|b| b.is_ascii_alphanumeric())
}
fn check_code(data: &Value) -> Result<(), String> {
    match data["code"].as_i64() {
        Some(0) => Ok(()),
        Some(-101) | Some(-403) | Some(-10403) => {
            Err("该视频需要有效登录或访问权限，请在 Bilibili 登录后重试。".into())
        }
        Some(-404) | Some(62002) => Err("视频不存在、已删除或不可访问，请换一个链接。".into()),
        Some(-352) | Some(-412) | Some(-401) => Err("Bilibili 请求暂时受限，请稍后重试。".into()),
        Some(code) => Err(format!("Bilibili 未提供此内容（错误码 {code}）。")),
        None => Err("Bilibili 返回格式已改变，解析未完成。".into()),
    }
}
fn parse_preview(data: &Value, page: u32) -> Result<Preview, String> {
    let bvid = data["bvid"]
        .as_str()
        .filter(|s| valid_bvid(s))
        .ok_or("视频编号无效")?
        .to_owned();
    let title = data["title"]
        .as_str()
        .filter(|t| !t.trim().is_empty())
        .ok_or("视频缺少标题")?
        .chars()
        .take(300)
        .collect();
    let parts: Vec<_> = data["pages"]
        .as_array()
        .ok_or("视频缺少分 P 信息")?
        .iter()
        .take(1000)
        .filter_map(|p| {
            Some(Part {
                cid: p["cid"].as_u64().filter(|n| *n > 0)?,
                page: p["page"].as_u64().filter(|n| *n > 0 && *n <= 1000)? as u32,
                title: p["part"].as_str()?.chars().take(300).collect(),
                duration: p["duration"].as_u64().unwrap_or(0),
            })
        })
        .collect();
    if parts.is_empty() || !parts.iter().any(|p| p.page == page) {
        return Err("这个分 P 不存在，请检查链接中的 p 参数。".into());
    }
    Ok(Preview {
        id: String::new(),
        bvid,
        title,
        author: data["owner"]["name"]
            .as_str()
            .unwrap_or("未知作者")
            .chars()
            .take(100)
            .collect(),
        parts,
        selected_page: page,
        created: online::now_ms(),
    })
}
struct Audio {
    urls: Vec<String>,
}
fn select_audio(data: &Value) -> Result<Audio, String> {
    let streams = data["dash"]["audio"]
        .as_array()
        .ok_or("这个视频未提供可下载的独立音轨，暂时无法导入。")?;
    let audio = streams
        .iter()
        .filter(|a| {
            a["codecs"].as_str().is_some_and(|c| c.starts_with("mp4a."))
                && (a["mimeType"].as_str().or(a["mime_type"].as_str()) == Some("audio/mp4"))
        })
        .max_by_key(|a| a["bandwidth"].as_u64().unwrap_or(0))
        .ok_or("该视频没有兼容的 AAC 音轨。")?;
    let mut urls = Vec::new();
    if let Some(url) = audio["baseUrl"].as_str().or(audio["base_url"].as_str()) {
        if let Ok(url) = audio_url(url) {
            urls.push(url.to_string());
        }
    }
    if let Some(backups) = audio["backupUrl"]
        .as_array()
        .or(audio["backup_url"].as_array())
    {
        for url in backups.iter().filter_map(Value::as_str).take(4) {
            if let Ok(url) = audio_url(url) {
                urls.push(url.to_string());
            }
        }
    }
    if urls.is_empty() {
        return Err("Bilibili 返回了不支持的音轨地址。".into());
    }
    Ok(Audio { urls })
}
fn audio_url(raw: &str) -> Result<Url, String> {
    let mut url = Url::parse(raw).map_err(|_| "音轨地址无效")?;
    let host = url.host_str().unwrap_or_default();
    let trusted = ["bilivideo.com", "bilivideo.cn", "bilivideo.net"]
        .iter()
        .any(|d| host == *d || host.ends_with(&format!(".{d}")))
        || host == "upos-hz-mirrorakam.akamaized.net";
    if !trusted
        || !["http", "https"].contains(&url.scheme())
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
    {
        return Err("音轨来源不在 Bilibili 下载域名范围内。".into());
    }
    let _ = url.set_scheme("https");
    Ok(url)
}
fn wbi_key(images: &Value) -> Result<String, String> {
    let mut source = String::new();
    for key in ["img_url", "sub_url"] {
        let value = images[key]
            .as_str()
            .ok_or("无法取得 Bilibili 解析参数，请稍后重试。")?;
        source.push_str(
            value
                .rsplit('/')
                .next()
                .unwrap_or_default()
                .split('.')
                .next()
                .unwrap_or_default(),
        );
    }
    if source.len() != 64 || !source.is_ascii() {
        return Err("Bilibili 解析参数格式已改变。".into());
    }
    const ORDER: [usize; 32] = [
        46, 47, 18, 2, 53, 8, 23, 32, 15, 50, 10, 31, 58, 3, 45, 35, 27, 43, 5, 49, 33, 9, 42, 19,
        29, 28, 14, 39, 12, 38, 41, 13,
    ];
    Ok(ORDER
        .iter()
        .map(|i| source.as_bytes()[*i] as char)
        .collect())
}
fn sign_wbi(
    mut params: BTreeMap<String, String>,
    key: &str,
    timestamp: u64,
) -> Vec<(String, String)> {
    params.insert("wts".into(), timestamp.to_string());
    for value in params.values_mut() {
        value.retain(|c| !"!'()*".contains(c));
    }
    let query = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(&params)
        .finish();
    params.insert(
        "w_rid".into(),
        format!("{:x}", md5::compute(format!("{query}{key}"))),
    );
    params.into_iter().collect()
}
fn safe_title(title: &str) -> String {
    let text: String = title
        .chars()
        .map(|c| {
            if c.is_control() || "<>:\"/\\|?*".contains(c) {
                '_'
            } else {
                c
            }
        })
        .take(70)
        .collect();
    let mut text = text.as_str();
    while text.len() > 190 {
        text = &text[..text.char_indices().last().unwrap().0];
    }
    let text = text.trim().trim_matches('.');
    if text.is_empty() {
        "Bilibili 音频".into()
    } else {
        text.into()
    }
}
fn validate_mp4(file: &mut std::fs::File) -> Result<(), String> {
    let length = file.metadata().map_err(|e| e.to_string())?.len();
    if length == 0 || length > MAX_AUDIO {
        return Err("音频文件大小无效。".into());
    }
    file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    let mut offset = 0;
    let mut ftyp = false;
    let mut moov = false;
    let mut mdat = false;
    let mut boxes = 0;
    while offset < length {
        boxes += 1;
        if boxes > 100_000 || length - offset < 8 {
            return Err("音频文件结构不完整，请重试。".into());
        }
        let mut header = [0u8; 8];
        file.read_exact(&mut header)
            .map_err(|_| "音频文件头不完整")?;
        let mut size = u32::from_be_bytes(header[..4].try_into().unwrap()) as u64;
        let mut header_size = 8;
        if size == 1 {
            let mut extended = [0; 8];
            file.read_exact(&mut extended)
                .map_err(|_| "音频文件头不完整")?;
            size = u64::from_be_bytes(extended);
            header_size = 16;
        }
        if size == 0 {
            size = length - offset;
        }
        if size < header_size || size > length - offset {
            return Err("音频文件损坏或下载不完整，请重试。".into());
        }
        match &header[4..] {
            b"ftyp" if offset == 0 && size >= 16 => ftyp = true,
            b"moov" => moov = true,
            b"mdat" if size > header_size => mdat = true,
            _ => {}
        }
        offset += size;
        file.seek(SeekFrom::Start(offset))
            .map_err(|e| e.to_string())?;
    }
    if ftyp && moov && mdat {
        Ok(())
    } else {
        Err("下载内容不是完整的 M4A 音频，请重试。".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Write;
    #[test]
    fn input_boundaries_and_part_selection() {
        assert_eq!(
            parse_input("https://www.bilibili.com/video/BV1xx411c7mD/?p=2").unwrap(),
            Input::Video {
                key: "bvid",
                value: "BV1xx411c7mD".into(),
                page: 2
            }
        );
        for input in [
            "https://bilibili.com.evil.org/video/BV1xx411c7mD",
            "http://localhost:9000/video/BV1xx411c7mD",
            "https://user@www.bilibili.com/video/BV1xx411c7mD",
            "https://www.bilibili.com/video/BV1xx411c7mD?p=0",
            "https://www.bilibili.com/bangumi/play/ss1",
        ] {
            assert!(parse_input(input).is_err(), "{input}");
        }
        assert!(matches!(
            parse_input("b23.tv/example").unwrap(),
            Input::Short(_)
        ));
    }
    #[test]
    fn wbi_reference_vector() {
        let key = wbi_key(&json!({"img_url":"https://i0.hdslb.com/bfs/wbi/7cd084941338484aae1ad9425b84077c.png", "sub_url":"https://i0.hdslb.com/bfs/wbi/4932caff0ff746eab6f01bf08b70ac45.png"})).unwrap();
        assert_eq!(key, "ea1db124af3c7062474693fa704f4ff8");
        let params = sign_wbi(
            BTreeMap::from([
                ("foo".into(), "114".into()),
                ("bar".into(), "514".into()),
                ("baz".into(), "1919810".into()),
            ]),
            &key,
            1702204169,
        );
        assert_eq!(
            params.iter().find(|(k, _)| k == "w_rid").unwrap().1,
            "6149fdadf571698ca7e6a567265cd0ee"
        );
    }
    #[test]
    fn audio_only_codec_selection_and_domain_boundary() {
        let audio = select_audio(&json!({"dash":{"audio":[
            {"codecs":"flac","mimeType":"audio/mp4","bandwidth":999999,"baseUrl":"https://cdn.bilivideo.com/flac"},
            {"codecs":"mp4a.40.2","mimeType":"audio/mp4","bandwidth":128000,"baseUrl":"http://cdn.bilivideo.com/audio"}
        ]}})).unwrap();
        assert_eq!(audio.urls, vec!["https://cdn.bilivideo.com/audio"]);
        for url in [
            "https://127.0.0.1/a",
            "https://bilivideo.com.evil.org/a",
            "https://cdn.bilivideo.com:999/a",
            "file:///tmp/a",
        ] {
            assert!(audio_url(url).is_err());
        }
    }
    #[test]
    fn incomplete_audio_never_validates() {
        let mut file = tempfile::tempfile().unwrap();
        file.write_all(b"<html>error</html>").unwrap();
        assert!(validate_mp4(&mut file).is_err());
        file.set_len(0).unwrap();
        file.rewind().unwrap();
        for (kind, body) in [
            (b"ftyp", b"M4A     ".as_slice()),
            (b"moov", b"test".as_slice()),
            (b"mdat", b"audio".as_slice()),
        ] {
            file.write_all(&((8 + body.len()) as u32).to_be_bytes())
                .unwrap();
            file.write_all(kind).unwrap();
            file.write_all(body).unwrap();
        }
        assert!(validate_mp4(&mut file).is_ok());
        file.set_len(file.metadata().unwrap().len() - 1).unwrap();
        assert!(validate_mp4(&mut file).is_err());
    }
    #[test]
    fn hostile_titles_and_missing_parts() {
        assert_eq!(safe_title("../../歌曲:测试?"), "_.._歌曲_测试_");
        assert!(parse_preview(&json!({"bvid":"BV1xx411c7mD","title":"视频","pages":[{"cid":1,"page":1,"part":"第一首"}]}),2).is_err());
    }

    #[tokio::test]
    async fn queue_download_publish_cancel_retry_and_reopen_library() {
        use axum::{
            extract::{Query, State},
            http::{HeaderMap, Uri},
            response::{IntoResponse, Response},
            routing::get,
            Router,
        };
        use std::sync::atomic::{AtomicBool, Ordering};
        struct ServerState {
            repaired: AtomicBool,
            leaked_cookie: AtomicBool,
        }
        fn audio_bytes() -> Vec<u8> {
            let mut data = Vec::new();
            for (kind, body) in [
                (b"ftyp", b"M4A     ".as_slice()),
                (b"moov", b"fixture".as_slice()),
                (b"mdat", b"test audio".as_slice()),
            ] {
                data.extend_from_slice(&((body.len() + 8) as u32).to_be_bytes());
                data.extend_from_slice(kind);
                data.extend_from_slice(body);
            }
            data
        }
        async fn serve(
            State(state): State<Arc<ServerState>>,
            uri: Uri,
            Query(query): Query<HashMap<String, String>>,
            headers: HeaderMap,
        ) -> Response {
            let value = match uri.path() {
                "/x/web-interface/nav" => {
                    json!({"code":0,"data":{"isLogin":true,"wbi_img":{"img_url":"https://i.example/7cd084941338484aae1ad9425b84077c.png","sub_url":"https://i.example/4932caff0ff746eab6f01bf08b70ac45.png"}}})
                }
                "/x/web-interface/wbi/view/detail" => {
                    json!({"code":0,"data":{"View":{"bvid":"BV1xx411c7mD","title":"测试视频","owner":{"name":"作者"},"pages":[{"cid":10,"page":1,"part":"自创一","duration":10},{"cid":20,"page":2,"part":"自创二","duration":10},{"cid":30,"page":3,"part":"下载取消测试","duration":10}]}}})
                }
                "/x/player/wbi/playurl" => {
                    assert!(query.contains_key("w_rid"));
                    json!({"code":0,"data":{"dash":{"audio":[{"codecs":"mp4a.40.2","mimeType":"audio/mp4","bandwidth":128000,"baseUrl":format!("https://cdn.bilivideo.com/audio/{}",query["cid"])}]}}})
                }
                path if path.starts_with("/audio/") => {
                    if headers.contains_key("cookie") {
                        state.leaked_cookie.store(true, Ordering::SeqCst);
                    }
                    if path.ends_with("30") {
                        let stream = futures_util::stream::unfold(false, |sent| async move {
                            if sent {
                                std::future::pending::<()>().await;
                            }
                            Some((Ok::<_, std::io::Error>(audio_bytes()), true))
                        });
                        return Response::new(axum::body::Body::from_stream(stream));
                    }
                    let data = if path.ends_with("20") && !state.repaired.load(Ordering::SeqCst) {
                        b"<html>error</html>".to_vec()
                    } else {
                        audio_bytes()
                    };
                    return ([("content-type", "audio/mp4")], data).into_response();
                }
                _ => return axum::http::StatusCode::NOT_FOUND.into_response(),
            };
            axum::Json(value).into_response()
        }
        async fn terminal(downloader: &Downloader, id: &str) -> DownloadJob {
            tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    let job = downloader
                        .state()
                        .await
                        .jobs
                        .into_iter()
                        .find(|j| j.id == id)
                        .unwrap();
                    if !active_phase(&job.phase) {
                        break job;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap()
        }
        let directory = tempfile::tempdir().unwrap();
        let engine = Engine::new(directory.path().to_owned()).await.unwrap();
        let state = Arc::new(ServerState {
            repaired: AtomicBool::new(false),
            leaked_cookie: AtomicBool::new(false),
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let app = Router::new().fallback(get(serve)).with_state(state.clone());
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let mut downloader = Downloader::new(directory.path()).unwrap();
        let owned = Arc::get_mut(&mut downloader).unwrap();
        owned.client = reqwest::Client::builder().no_proxy().build().unwrap();
        owned.test_origin = Some(format!("http://{address}").parse().unwrap());
        downloader
            .set_session("SESSDATA=fixture-only; ignored=secret")
            .await
            .unwrap();
        let preview = downloader
            .parse("https://www.bilibili.com/zh-Hans/video/BV1xx411c7mD/?p=2")
            .await
            .unwrap();
        assert_eq!(preview.selected_page, 2);
        // Hold the worker to deterministically exercise queued cancellation/dedup.
        let permit = downloader.queue.acquire().await.unwrap();
        let queued = downloader
            .start(
                Arc::downgrade(&engine),
                &preview.id,
                10,
                engine.cancellation(),
            )
            .await
            .unwrap();
        assert!(downloader
            .start(
                Arc::downgrade(&engine),
                &preview.id,
                10,
                engine.cancellation()
            )
            .await
            .is_err());
        downloader.cancel(&queued.id).await.unwrap();
        drop(permit);
        assert_eq!(terminal(&downloader, &queued.id).await.phase, "cancelled");
        assert!(engine.snapshot().await.unwrap().tracks.is_empty());
        let retry = downloader
            .retry(Arc::downgrade(&engine), &queued.id, engine.cancellation())
            .await
            .unwrap();
        let completed = terminal(&downloader, &retry.id).await;
        assert_eq!(completed.phase, "completed", "{:?}", completed.error);
        let bytes = std::fs::read(completed.path.unwrap()).unwrap();
        assert_eq!(bytes, audio_bytes());
        assert!(downloader
            .start(
                Arc::downgrade(&engine),
                &preview.id,
                10,
                engine.cancellation()
            )
            .await
            .unwrap_err()
            .contains("已在曲库"));
        let broken = downloader
            .start(
                Arc::downgrade(&engine),
                &preview.id,
                20,
                engine.cancellation(),
            )
            .await
            .unwrap();
        assert_eq!(terminal(&downloader, &broken.id).await.phase, "failed");
        assert_eq!(engine.snapshot().await.unwrap().tracks.len(), 1);
        state.repaired.store(true, Ordering::SeqCst);
        let repaired = downloader
            .retry(Arc::downgrade(&engine), &broken.id, engine.cancellation())
            .await
            .unwrap();
        assert_eq!(terminal(&downloader, &repaired.id).await.phase, "completed");
        let slow = downloader
            .start(
                Arc::downgrade(&engine),
                &preview.id,
                30,
                engine.cancellation(),
            )
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if downloader
                    .state()
                    .await
                    .jobs
                    .iter()
                    .any(|j| j.id == slow.id && j.downloaded > 0)
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        downloader.cancel(&slow.id).await.unwrap();
        assert_eq!(terminal(&downloader, &slow.id).await.phase, "cancelled");
        assert!(!state.leaked_cookie.load(Ordering::SeqCst));
        let library = crate::library::Library::load(directory.path().to_owned()).unwrap();
        assert_eq!(library.tracks().len(), 2);
        assert_eq!(library.tracks()[0].title, "自创一");
        assert!(std::fs::read_dir(directory.path().join("downloads"))
            .unwrap()
            .all(|e| e.unwrap().path().extension().unwrap() == "m4a"));
        engine.shutdown();
        server.abort();
    }
}
