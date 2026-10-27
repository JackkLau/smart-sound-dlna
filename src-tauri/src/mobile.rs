//! Opt-in, paired LAN import and control of the explicitly selected speaker.
use crate::{
    bilibili::{BatchSelection, Downloader},
    engine::{Engine, Snapshot},
};
use axum::{
    extract::{DefaultBodyLimit, Query, Request, State},
    http::{header, HeaderValue, StatusCode},
    middleware::{self, Next},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    net::{Ipv4Addr, SocketAddrV4},
    path::PathBuf,
    sync::{Arc, Weak},
    time::{Duration, Instant},
};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

pub const MOBILE_PORT: u16 = 8998;
pub struct Receiver {
    running: Mutex<Option<Running>>,
    cache: PathBuf,
}
struct Running {
    state: Arc<Session>,
    urls: Vec<String>,
    server: Option<tokio::task::JoinHandle<()>>,
}
struct Session {
    engine: Weak<Engine>,
    downloader: Arc<Downloader>,
    shutdown: CancellationToken,
    hosts: HashSet<String>,
    ticket: String,
    cache: PathBuf,
    created: Instant,
    pairing: Mutex<Pairing>,
    operation: Mutex<()>,
    previews: Mutex<HashSet<String>>,
    jobs: Mutex<HashSet<String>>,
}
#[derive(Default)]
struct Pairing {
    attempts: u8,
    hash: Option<String>,
}
#[derive(Serialize)]
struct Entry {
    address: String,
    url: String,
}
#[derive(Default, Serialize)]
pub struct ReceiverStatus {
    enabled: bool,
    paired: bool,
    pairing_available: bool,
    entries: Vec<Entry>,
}

impl Receiver {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            running: Mutex::new(None),
            cache: data_dir.join("mobile-pairing-v1.sha256"),
        }
    }
    pub async fn status(&self) -> ReceiverStatus {
        let running = self.running.lock().await;
        let Some(running) = running
            .as_ref()
            .filter(|r| !r.state.shutdown.is_cancelled())
        else {
            return ReceiverStatus::default();
        };
        let pairing = running.state.pairing.lock().await;
        let paired = pairing.hash.is_some();
        let available = !paired
            && pairing.attempts < 5
            && running.state.created.elapsed() < Duration::from_secs(600);
        ReceiverStatus {
            enabled: true,
            paired,
            pairing_available: available,
            entries: running
                .urls
                .iter()
                .map(|url| Entry {
                    address: url::Url::parse(url)
                        .ok()
                        .and_then(|u| u.host_str().map(str::to_owned))
                        .unwrap_or_default(),
                    url: if available {
                        format!("{url}/#pair={}", running.state.ticket)
                    } else {
                        format!("{url}/")
                    },
                })
                .collect(),
        }
    }

    pub async fn enable(&self, engine: &Arc<Engine>) -> Result<ReceiverStatus, String> {
        let mut running = self.running.lock().await;
        if running
            .as_ref()
            .is_some_and(|r| r.state.shutdown.is_cancelled())
        {
            if let Some(previous) = running.take() {
                previous.stop().await;
            }
        }
        if running.is_none() {
            let mut addresses: Vec<_> = if_addrs::get_if_addrs()
                .map_err(|_| "无法读取本机网络地址")?
                .into_iter()
                .filter_map(|i| match i.ip() {
                    std::net::IpAddr::V4(ip) if ip.is_private() && !ip.is_loopback() => Some(ip),
                    _ => None,
                })
                .collect();
            addresses.sort();
            addresses.dedup();
            if addresses.is_empty() {
                return Err("未找到局域网地址，请先连接 Wi-Fi 或有线网络。".into());
            }
            *running = Some(
                start(
                    engine,
                    addresses,
                    Ipv4Addr::UNSPECIFIED,
                    MOBILE_PORT,
                    self.cache.clone(),
                )
                .await?,
            );
        }
        drop(running);
        Ok(self.status().await)
    }

    pub async fn disable(&self) -> ReceiverStatus {
        let mut state = self.running.lock().await;
        if let Some(running) = state.take() {
            running.stop().await;
        }
        ReceiverStatus::default()
    }

    pub async fn forget(&self) -> Result<ReceiverStatus, String> {
        // Keep the gate held until disk removal completes; enable cannot reload
        // the old credential while an explicit revocation is in progress.
        let mut state = self.running.lock().await;
        if let Some(running) = state.take() {
            running.stop().await;
        }
        crate::mobile_pairing::forget(&self.cache)?;
        Ok(ReceiverStatus::default())
    }
}
impl Running {
    async fn stop(mut self) {
        self.state.shutdown.cancel();
        if let Some(mut server) = self.server.take() {
            if tokio::time::timeout(Duration::from_secs(2), &mut server)
                .await
                .is_err()
            {
                server.abort();
                let _ = server.await;
            }
        }
    }
}
impl Drop for Running {
    fn drop(&mut self) {
        self.state.shutdown.cancel();
    }
}

async fn start(
    engine: &Arc<Engine>,
    addresses: Vec<Ipv4Addr>,
    bind: Ipv4Addr,
    port: u16,
    cache: PathBuf,
) -> Result<Running, String> {
    let hash = crate::mobile_pairing::load(&cache)?;
    let listener = tokio::net::TcpListener::bind(SocketAddrV4::new(bind, port))
        .await
        .map_err(|_| format!("无法开启手机接收：固定端口 {port} 不可用，请关闭重复运行的应用或占用该端口的服务后重试。"))?;
    let port = listener
        .local_addr()
        .map_err(|_| "无法读取接收端口")?
        .port();
    let hosts: HashSet<_> = addresses.iter().map(|ip| format!("{ip}:{port}")).collect();
    let urls = addresses
        .iter()
        .map(|ip| format!("http://{ip}:{port}"))
        .collect();
    let state = Arc::new(Session {
        engine: Arc::downgrade(engine),
        downloader: engine.downloads.clone(),
        shutdown: engine.cancellation().child_token(),
        hosts,
        ticket: format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        ),
        cache,
        created: Instant::now(),
        pairing: Mutex::new(Pairing { attempts: 0, hash }),
        operation: Mutex::new(()),
        previews: Mutex::new(HashSet::new()),
        jobs: Mutex::new(HashSet::new()),
    });
    let router = Router::new()
        .route(
            "/",
            get(|| async { Html(include_str!("../../dist/mobile/index.html")) }),
        )
        .route(
            "/app.js",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
                    include_str!("../../dist/mobile/app.js"),
                )
            }),
        )
        .route(
            "/app.css",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
                    include_str!("../../dist/mobile/app.css"),
                )
            }),
        )
        .route("/api/pair", post(pair))
        .route("/api/parse", post(parse))
        .route("/api/download", post(download))
        .route("/api/jobs", get(jobs))
        .route("/api/library", get(library))
        .route("/api/playback", post(playback))
        .layer(DefaultBodyLimit::max(16 * 1024))
        .layer(middleware::from_fn_with_state(state.clone(), guard))
        .with_state(state.clone());
    let shutdown = state.shutdown.clone();
    let server = tokio::spawn(async move {
        // Close the listener and idle connections; guard cancels active requests.
        let _ = axum::serve(listener, router)
            .with_graceful_shutdown(shutdown.clone().cancelled_owned())
            .await;
        shutdown.cancel();
    });
    Ok(Running {
        state,
        urls,
        server: Some(server),
    })
}

type ApiResult = Result<Json<Value>, (StatusCode, Json<Value>)>;
fn failure(status: StatusCode, message: impl Into<String>) -> (StatusCode, Json<Value>) {
    (status, Json(json!({"error": message.into()})))
}
fn same_secret(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |v, (a, b)| v | (a ^ b)) == 0
}
async fn guard(State(state): State<Arc<Session>>, request: Request, next: Next) -> Response {
    let host = request
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    let origin_ok = request
        .headers()
        .get(header::ORIGIN)
        .is_none_or(|h| h.to_str().ok() == Some(&format!("http://{host}")));
    if !state.hosts.contains(host) || !origin_ok {
        return failure(StatusCode::FORBIDDEN, "请扫描电脑上的二维码打开此页面。").into_response();
    }
    if request.uri().path().starts_with("/api/") && request.uri().path() != "/api/pair" {
        let bearer = request
            .headers()
            .get(header::AUTHORIZATION)
            .and_then(|h| h.to_str().ok())
            .and_then(|s| s.strip_prefix("Bearer "))
            .unwrap_or("");
        let hash = crate::mobile_pairing::digest(bearer);
        if !state
            .pairing
            .lock()
            .await
            .hash
            .as_ref()
            .is_some_and(|saved| same_secret(&hash, saved))
        {
            return failure(
                StatusCode::UNAUTHORIZED,
                "此浏览器尚未配对或配对已失效，请扫描电脑的配对二维码。",
            )
            .into_response();
        }
    }
    let mut response = tokio::select! {
        biased;
        _ = state.shutdown.cancelled() => failure(StatusCode::GONE, "电脑已关闭手机接收。").into_response(),
        response = next.run(request) => response,
    };
    for (name, value) in [
        ("cache-control", "no-store"), ("x-content-type-options", "nosniff"), ("referrer-policy", "no-referrer"),
        ("content-security-policy", "default-src 'self'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'none'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'; form-action 'self'"),
    ] { response.headers_mut().insert(name, HeaderValue::from_static(value)); }
    response
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PairRequest {
    ticket: String,
}
async fn pair(State(state): State<Arc<Session>>, Json(input): Json<PairRequest>) -> ApiResult {
    let mut pairing = state.pairing.lock().await;
    if pairing.hash.is_some()
        || pairing.attempts >= 5
        || state.created.elapsed() >= Duration::from_secs(600)
    {
        return Err(failure(
            StatusCode::FORBIDDEN,
            "二维码已使用或过期。请在电脑点击“忘记手机并重新配对”，再扫描新二维码。",
        ));
    }
    pairing.attempts += 1;
    if !same_secret(&input.ticket, &state.ticket) {
        return Err(failure(
            StatusCode::UNAUTHORIZED,
            "配对信息不正确，请重新扫描电脑上的二维码。",
        ));
    }
    let token = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let hash = crate::mobile_pairing::digest(&token);
    crate::mobile_pairing::save(&state.cache, &hash)
        .map_err(|e| failure(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    pairing.hash = Some(hash);
    Ok(Json(json!({"token": token})))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ParseRequest {
    text: String,
}
async fn parse(State(state): State<Arc<Session>>, Json(input): Json<ParseRequest>) -> ApiResult {
    let _operation = state.operation.try_lock().map_err(|_| {
        failure(
            StatusCode::TOO_MANY_REQUESTS,
            "正在处理上一个请求，请稍候。",
        )
    })?;
    let input = crate::bilibili::shared_video_input(&input.text)
        .map_err(|e| failure(StatusCode::BAD_REQUEST, e))?;
    let preview = state
        .downloader
        .parse(&input)
        .await
        .map_err(|e| failure(StatusCode::BAD_REQUEST, e))?;
    let mut previews = state.previews.lock().await;
    // Mobile only needs its current preview; desktop previews remain isolated.
    previews.clear();
    previews.insert(preview.id.clone());
    Ok(Json(json!(preview)))
}
async fn download(
    State(state): State<Arc<Session>>,
    Json(selection): Json<BatchSelection>,
) -> ApiResult {
    let _operation = state.operation.try_lock().map_err(|_| {
        failure(
            StatusCode::TOO_MANY_REQUESTS,
            "正在处理上一个请求，请稍候。",
        )
    })?;
    if !state.previews.lock().await.contains(&selection.preview_id) {
        return Err(failure(StatusCode::BAD_REQUEST, "请先在手机重新解析视频。"));
    }
    let engine = state
        .engine
        .upgrade()
        .ok_or_else(|| failure(StatusCode::GONE, "电脑应用已退出。"))?;
    let result = state
        .downloader
        .start_batch(
            Arc::downgrade(&engine),
            vec![selection],
            engine.cancellation(),
        )
        .await
        .map_err(|e| failure(StatusCode::BAD_REQUEST, e))?;
    let mut ids = state.jobs.lock().await;
    let mut items = Vec::new();
    for item in result.items {
        if let Some(job) = &item.job {
            ids.insert(job.id.clone());
        }
        items.push(json!({"cid":item.cid, "title":item.title, "accepted":item.job.is_some(), "error":item.error}));
    }
    Ok(Json(json!({"items":items})))
}
async fn jobs(State(state): State<Arc<Session>>) -> Json<Value> {
    let snapshot = state.downloader.state().await;
    let mut ids = state.jobs.lock().await;
    ids.retain(|id| snapshot.jobs.iter().any(|j| &j.id == id));
    Json(
        json!({"jobs": snapshot.jobs.into_iter().filter(|j| ids.contains(&j.id)).map(|j| json!({
        "id":j.id,"title":j.title,"phase":j.phase,"downloaded":j.downloaded,"total":j.total,
        "error":j.error.map(|_| "下载未完成，请在电脑的下载列表查看原因并重试。")
    })).collect::<Vec<_>>()}),
    )
}

#[cfg(test)]
#[path = "mobile_tests.rs"]
mod tests;

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct LibraryQuery {
    q: String,
    page: usize,
}
fn phone_player(snapshot: &Snapshot) -> Value {
    let device = snapshot
        .devices
        .iter()
        .find(|d| Some(&d.id) == snapshot.selected_device_id.as_ref());
    let current = snapshot
        .tracks
        .iter()
        .find(|t| Some(&t.id) == snapshot.current_track_id.as_ref());
    json!({
        "device_id": snapshot.selected_device_id,
        "device_name": device.map(|d| &d.name),
        "connection": snapshot.device_connection,
        "current_track_id": snapshot.current_track_id,
        "title": current.map(|t| &t.title),
        "transport": snapshot.transport_state,
        "mode": snapshot.queue_mode,
    })
}
fn phone_library(snapshot: &Snapshot, query: &LibraryQuery) -> Value {
    let q = query.q.trim().to_lowercase();
    let tracks: Vec<_> = snapshot
        .tracks
        .iter()
        .filter(|t| t.title.to_lowercase().contains(&q))
        .collect();
    let pages = tracks.len().div_ceil(20).max(1);
    let page = query.page.max(1).min(pages);
    json!({"player": phone_player(snapshot), "total": tracks.len(), "library_total": snapshot.tracks.len(), "page": page, "pages": pages,
        "tracks": tracks.iter().skip((page - 1) * 20).take(20).map(|t| json!({"id": t.id, "title": t.title, "format": t.format})).collect::<Vec<_>>()})
}
async fn library(
    State(state): State<Arc<Session>>,
    Query(query): Query<LibraryQuery>,
) -> ApiResult {
    if query.q.chars().count() > 120 {
        return Err(failure(StatusCode::BAD_REQUEST, "搜索词最多 120 个字。"));
    }
    let engine = state
        .engine
        .upgrade()
        .ok_or_else(|| failure(StatusCode::GONE, "电脑应用已退出。"))?;
    let snapshot = engine.snapshot().await.map_err(|_| {
        failure(
            StatusCode::SERVICE_UNAVAILABLE,
            "电脑曲库暂时不可用，请重试。",
        )
    })?;
    Ok(Json(phone_library(&snapshot, &query)))
}
#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
enum QueueMode {
    Sequential,
    RepeatAll,
    RepeatOne,
}
impl QueueMode {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Sequential => "sequential",
            Self::RepeatAll => "repeat-all",
            Self::RepeatOne => "repeat-one",
        }
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
enum TransportCommand {
    Pause,
    Resume,
    Stop,
    Next,
    Previous,
}
impl TransportCommand {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Pause => "pause",
            Self::Resume => "resume",
            Self::Stop => "stop",
            Self::Next => "next",
            Self::Previous => "previous",
        }
    }
}
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "kebab-case", deny_unknown_fields)]
enum PlaybackRequest {
    Play {
        device_id: String,
        track_id: String,
    },
    PlayAll {
        device_id: String,
    },
    Control {
        device_id: String,
        command: TransportCommand,
    },
    Mode {
        mode: QueueMode,
    },
}
async fn playback(
    State(state): State<Arc<Session>>,
    Json(input): Json<PlaybackRequest>,
) -> ApiResult {
    let _operation = state.operation.try_lock().map_err(|_| {
        failure(
            StatusCode::TOO_MANY_REQUESTS,
            "正在处理上一个请求，请稍候。",
        )
    })?;
    let engine = state
        .engine
        .upgrade()
        .ok_or_else(|| failure(StatusCode::GONE, "电脑应用已退出。"))?;
    // Once accepted, finish a serialized speaker mutation even if the phone
    // disconnects or reception is paused during SOAP. Dropping an HTTP handler
    // must not leave the renderer changed and PC state unreconciled.
    let result = tokio::spawn(async move {
        match input {
            PlaybackRequest::Play {
                device_id,
                track_id,
            } => engine.remote_play(&device_id, &track_id).await,
            PlaybackRequest::PlayAll { device_id } => {
                engine.remote_control(&device_id, "play-all").await
            }
            PlaybackRequest::Control { device_id, command } => {
                engine.remote_control(&device_id, command.as_str()).await
            }
            PlaybackRequest::Mode { mode } => engine.control("mode", json!(mode.as_str())).await,
        }
    })
    .await
    .map_err(|_| {
        failure(
            StatusCode::INTERNAL_SERVER_ERROR,
            "电脑未能完成操作，请刷新状态后重试。",
        )
    })?;
    // Never expose raw file/network diagnostics or paths to the phone.
    let snapshot = result.map_err(|_| failure(StatusCode::CONFLICT, "操作未完成，状态已刷新。请确认电脑仍连接所选音响、歌曲仍在曲库中，再重试；详细原因可在电脑查看。"))?;
    Ok(Json(phone_player(&snapshot)))
}
