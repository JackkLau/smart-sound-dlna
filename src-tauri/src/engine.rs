//! Shared state. User commands preempt read-only polling; mutations stay serialized.
use crate::{
    bilibili::Downloader,
    dlna::{self, Controller, Device},
    library::{self, ImportSummary, Library, Track, TrackSource},
    media::{self, MediaServer},
    voice::{self, VoiceService},
};
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::HashMap,
    net::UdpSocket,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::{watch, Mutex as AsyncMutex, MutexGuard, Notify};
use tokio_util::sync::CancellationToken;

#[cfg(test)]
#[path = "engine_connection_tests.rs"]
mod connection_tests;

#[derive(Serialize)]
pub struct Snapshot {
    pub tracks: Vec<Track>,
    pub devices: Vec<Device>,
    pub selected_device_id: Option<String>,
    pub device_connection: ConnectionState,
    pub current_track_id: Option<String>,
    pub transport_state: String,
    pub position: u64,
    pub duration: u64,
    pub volume: Option<u8>,
    pub status_error: Option<String>,
    pub warnings: Vec<String>,
    pub queue_mode: String,
    pub discovering: bool,
    pub discovery_error: Option<String>,
    pub interfaces: Vec<String>,
    pub media_port: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionState {
    Unselected,
    Checking,
    Connected,
    Disconnected,
}

struct Connection {
    state: ConnectionState,
    failures: u32,
    next_probe: Option<Instant>,
    last_scan: Option<Instant>,
}

impl Default for Connection {
    fn default() -> Self {
        Self {
            state: ConnectionState::Unselected,
            failures: 0,
            next_probe: None,
            last_scan: None,
        }
    }
}

impl Connection {
    fn retry_now(&mut self) {
        self.state = ConnectionState::Checking;
        self.next_probe = None;
    }

    fn connected(&mut self) {
        self.state = ConnectionState::Connected;
        self.failures = 0;
        self.next_probe = None;
        self.last_scan = None;
    }

    fn failed(&mut self, now: Instant) {
        self.state = ConnectionState::Disconnected;
        self.failures = self.failures.saturating_add(1);
        let delay = (2u64 << (self.failures - 1).min(4)).min(30);
        self.next_probe = Some(now + Duration::from_secs(delay));
    }

    fn scan_due(&mut self, now: Instant) -> bool {
        if self.state != ConnectionState::Disconnected
            || self
                .last_scan
                .is_some_and(|last| now.duration_since(last) < Duration::from_secs(30))
        {
            return false;
        }
        self.last_scan = Some(now);
        true
    }
}

struct Player {
    devices: HashMap<String, (Device, Instant)>,
    selected: Option<String>,
    connection: Connection,
    current: Option<String>,
    uri: Option<String>,
    transport: String,
    position: u64,
    duration: u64,
    volume: Option<u8>,
    error: Option<String>,
    warnings: Vec<String>,
    queue_mode: String,
    discovering: bool,
    discovery_error: Option<String>,
    interfaces: Vec<String>,
    completion: Completion,
    local_duration: u64,
}

impl Default for Player {
    fn default() -> Self {
        Self {
            devices: HashMap::new(),
            selected: None,
            connection: Connection::default(),
            current: None,
            uri: None,
            transport: "STOPPED".into(),
            position: 0,
            duration: 0,
            volume: None,
            error: None,
            warnings: vec![],
            queue_mode: "sequential".into(),
            discovering: false,
            discovery_error: None,
            interfaces: vec![],
            completion: Completion::default(),
            local_duration: 0,
        }
    }
}

impl Player {
    fn observe_status(&mut self, status: dlna::TransportStatus, now: Instant) -> bool {
        self.connection.connected();
        self.error = if status.warnings.is_empty() {
            None
        } else {
            Some(status.warnings.join("；"))
        };
        // Another controller may have taken over this speaker.
        if status
            .current_uri
            .as_ref()
            .is_some_and(|uri| !uri.is_empty() && self.uri.as_ref().is_some_and(|ours| ours != uri))
        {
            self.current = None;
            self.uri = None;
            self.local_duration = 0;
            self.completion = Completion::default();
        }
        self.transport = status.transport_state.clone();
        self.position = status.position;
        self.duration = if status.duration > 0 {
            status.duration
        } else if self.current.is_some() {
            self.local_duration
        } else {
            0
        };
        if status.volume.is_some() {
            self.volume = status.volume;
        }
        let duration = self.duration;
        self.completion
            .observe(&status.transport_state, status.position, duration, now)
    }
}

/// A STOPPED observation alone is never sufficient to advance the queue.
#[derive(Default)]
struct Completion {
    armed: bool,
    playing: Option<(u64, u64, Instant)>,
    started: Option<Instant>,
    elapsed: u64,
}

impl Completion {
    fn clear_observation(&mut self) {
        self.playing = None;
        self.started = None;
        self.elapsed = 0;
    }
    fn pause(&mut self, now: Instant) {
        let elapsed = self
            .started
            .take()
            .map(|start| now.duration_since(start).as_secs())
            .unwrap_or(0);
        self.elapsed = self
            .elapsed
            .saturating_add(elapsed)
            .max(self.playing.map(|(position, _, _)| position).unwrap_or(0));
        self.playing = None;
    }
    fn observe(&mut self, state: &str, position: u64, duration: u64, now: Instant) -> bool {
        if !self.armed {
            return false;
        }
        match state {
            "PLAYING" => {
                // Some Xiaomi renderers report 00:00:00 throughout playback.
                // Elapsed evidence is used only after a confirmed STOPPED, never
                // to interrupt a PLAYING song with a timer.
                self.started.get_or_insert(now);
                self.playing = Some((position, duration, now));
                false
            }
            "PAUSED_PLAYBACK" => {
                self.pause(now);
                false
            }
            "STOPPED" | "NO_MEDIA_PRESENT" => {
                // Play is asynchronous on some speakers: its first poll can
                // still be STOPPED. Only a prior PLAYING sample may disarm it.
                let Some((last, length, at)) = self.playing.take() else {
                    return false;
                };
                let completed = {
                    let elapsed = now.duration_since(at);
                    let tolerance = now
                        .duration_since(at)
                        .as_secs()
                        .saturating_add(2)
                        .clamp(3, 10);
                    let observed = self
                        .started
                        .map(|start| now.duration_since(start).as_secs())
                        .unwrap_or(0)
                        .saturating_add(self.elapsed);
                    elapsed <= Duration::from_secs(10)
                        && length > 0
                        && last.max(observed).saturating_add(tolerance) >= length
                };
                self.armed = false;
                completed
            }
            _ => false,
        }
    }
}

pub struct Engine {
    pub voice: Arc<VoiceService>,
    pub downloads: Arc<Downloader>,
    library: Arc<Mutex<Library>>,
    media: MediaServer,
    player: AsyncMutex<Player>,
    actions: AsyncMutex<()>,
    discovery: AsyncMutex<()>,
    activity: watch::Sender<u64>,
    poll_wake: Notify,
    shutdown: CancellationToken,
}

impl Engine {
    pub async fn new(data_dir: PathBuf) -> Result<Arc<Self>, String> {
        let downloads = Downloader::new(&data_dir)?;
        let voice = VoiceService::new()?;
        let library = tokio::task::spawn_blocking(move || Library::load(data_dir))
            .await
            .map_err(|e| e.to_string())??;
        let library = Arc::new(Mutex::new(library));
        let shutdown = CancellationToken::new();
        let media = media::start(library.clone(), shutdown.clone()).await?;
        Ok(Arc::new(Self {
            voice,
            downloads,
            library,
            media,
            player: AsyncMutex::new(Player::default()),
            actions: AsyncMutex::new(()),
            discovery: AsyncMutex::new(()),
            activity: watch::channel(0).0,
            poll_wake: Notify::new(),
            shutdown,
        }))
    }

    pub fn start(self: &Arc<Self>) {
        let engine = self.clone();
        tokio::spawn(async move {
            // One startup scan; thereafter discovery is manual while healthy.
            tokio::select! {
                _ = engine.shutdown.cancelled() => return,
                _ = engine.discover() => {}
            }
            loop {
                tokio::select! {
                    _ = engine.shutdown.cancelled() => break,
                    _ = tokio::time::sleep(Duration::from_secs(5)) => {}
                }
                // A selected offline renderer may have changed its IP. Refresh
                // descriptions without changing the user's selected device ID.
                let recover = engine
                    .player
                    .lock()
                    .await
                    .connection
                    .scan_due(Instant::now());
                if recover {
                    tokio::select! {
                        _ = engine.shutdown.cancelled() => break,
                        _ = engine.discover() => {}
                    }
                }
            }
        });
        let engine = self.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = engine.shutdown.cancelled() => break,
                    _ = tokio::time::sleep(Duration::from_secs(2)) => {}
                    _ = engine.poll_wake.notified() => {}
                }
                tokio::select! {
                    _ = engine.shutdown.cancelled() => break,
                    _ = engine.poll() => {}
                }
            }
        });
    }

    pub fn cancellation(&self) -> CancellationToken {
        self.shutdown.clone()
    }

    pub fn shutdown(&self) {
        self.shutdown.cancel();
    }

    async fn begin_action(&self) -> MutexGuard<'_, ()> {
        // Cancel only read-only background requests. Foreground SOAP mutations
        // stay serialized and are never cancelled halfway through a URI change.
        self.activity
            .send_modify(|generation| *generation = generation.wrapping_add(1));
        self.actions.lock().await
    }

    async fn tracks(&self) -> Result<Vec<Track>, String> {
        let library = self.library.clone();
        tokio::task::spawn_blocking(move || {
            library
                .lock()
                .map_err(|_| "音乐库暂时不可用".to_string())
                .map(|library| library.tracks())
        })
        .await
        .map_err(|e| e.to_string())?
    }

    pub async fn snapshot(&self) -> Result<Snapshot, String> {
        let tracks = self.tracks().await?;
        let p = self.player.lock().await;
        let mut devices: Vec<Device> = p.devices.values().map(|(d, _)| d.clone()).collect();
        devices.sort_by(|a, b| {
            b.is_xiaomi_pro
                .cmp(&a.is_xiaomi_pro)
                .then_with(|| b.is_xiaomi.cmp(&a.is_xiaomi))
                .then_with(|| a.name.cmp(&b.name))
        });
        Ok(Snapshot {
            tracks,
            devices,
            selected_device_id: p.selected.clone(),
            device_connection: p.connection.state,
            current_track_id: p.current.clone(),
            transport_state: p.transport.clone(),
            position: p.position,
            duration: p.duration,
            volume: p.volume,
            status_error: p.error.clone(),
            warnings: p.warnings.clone(),
            queue_mode: p.queue_mode.clone(),
            discovering: p.discovering,
            discovery_error: p.discovery_error.clone(),
            interfaces: p.interfaces.clone(),
            media_port: self.media.port,
        })
    }

    pub async fn discover(&self) -> Result<Snapshot, String> {
        // A manual refresh can join an already running automatic scan.
        let _guard = match self.discovery.try_lock() {
            Ok(guard) => guard,
            Err(_) => {
                let _wait = self.discovery.lock().await;
                return self.snapshot().await;
            }
        };
        {
            let mut p = self.player.lock().await;
            p.discovering = true;
            p.discovery_error = None;
        }
        let report = dlna::discover().await;
        let mut p = self.player.lock().await;
        p.discovering = false;
        match report {
            Ok(report) => {
                let now = Instant::now();
                let selected = p.selected.clone();
                p.devices.retain(|id, (_, at)| {
                    selected.as_ref() == Some(id)
                        || now.duration_since(*at) < Duration::from_secs(120)
                });
                for device in report.devices {
                    if p.selected.as_ref() == Some(&device.id)
                        && p.connection.state == ConnectionState::Disconnected
                    {
                        p.connection.next_probe = None;
                    }
                    p.devices.insert(device.id.clone(), (device, now));
                }
                p.warnings = report.warnings;
                p.interfaces = report.interfaces;
            }
            Err(error) => {
                p.discovery_error = Some(error);
            }
        }
        drop(p);
        self.poll_wake.notify_one();
        self.snapshot().await
    }

    pub async fn add_device(&self, url: &str) -> Result<Snapshot, String> {
        let device = dlna::fetch_device(url).await?;
        self.player
            .lock()
            .await
            .devices
            .insert(device.id.clone(), (device, Instant::now()));
        self.snapshot().await
    }

    pub async fn select(&self, id: &str) -> Result<Snapshot, String> {
        let _guard = self.begin_action().await;
        let previous = {
            let p = self.player.lock().await;
            if !p.devices.contains_key(id) {
                return Err("设备已离线，请重新搜索。".into());
            }
            if p.selected.as_deref() != Some(id) && p.current.is_some() {
                p.selected
                    .as_ref()
                    .and_then(|id| p.devices.get(id))
                    .map(|(device, _)| device.clone())
            } else {
                None
            }
        };
        let stop_error = match previous {
            Some(device) => Controller::new(device).stop().await.err(),
            None => None,
        };
        let mut p = self.player.lock().await;
        if !p.devices.contains_key(id) {
            return Err("设备已离线，请重新搜索。".into());
        }
        if p.selected.as_deref() != Some(id) {
            p.selected = Some(id.to_string());
            p.current = None;
            p.uri = None;
            p.completion = Completion::default();
            p.transport = "STOPPED".into();
            p.position = 0;
            p.duration = 0;
            p.local_duration = 0;
            p.volume = None;
            p.error = None;
            p.connection = Connection::default();
            if let Some(error) = stop_error {
                p.warnings.push(format!(
                    "已切换设备，但原音箱未确认停止，请检查原音箱：{error}"
                ));
            }
        }
        p.connection.retry_now();
        drop(p);
        drop(_guard);
        self.poll_wake.notify_one();
        self.snapshot().await
    }

    pub async fn import(&self, paths: Vec<PathBuf>) -> Result<ImportSummary, String> {
        let library = self.library.clone();
        tokio::task::spawn_blocking(move || {
            library
                .lock()
                .map_err(|_| "音乐库暂时不可用".to_string())?
                .import_paths(&paths)
        })
        .await
        .map_err(|e| e.to_string())?
    }

    pub async fn import_download(
        &self,
        path: PathBuf,
        title: String,
        source: TrackSource,
    ) -> Result<Track, String> {
        let library = self.library.clone();
        tokio::task::spawn_blocking(move || {
            library
                .lock()
                .map_err(|_| "音乐库暂时不可用".to_owned())?
                .import_download(&path, &title, source)
        })
        .await
        .map_err(|e| e.to_string())?
    }

    pub async fn rename_track(
        &self,
        id: String,
        title: String,
        expected_title: String,
    ) -> Result<Snapshot, String> {
        let library = self.library.clone();
        tokio::task::spawn_blocking(move || {
            library
                .lock()
                .map_err(|_| "音乐库暂时不可用".to_owned())?
                .rename(&id, &title, &expected_title)
        })
        .await
        .map_err(|e| e.to_string())??;
        // Metadata-only: keep the active URI, queue, playback and voice binding.
        self.snapshot().await
    }

    pub async fn remove_track(&self, id: String) -> Result<Snapshot, String> {
        let _guard = self.begin_action().await;
        let library = self.library.clone();
        let check_id = id.clone();
        tokio::task::spawn_blocking(move || {
            library
                .lock()
                .map_err(|_| "音乐库暂时不可用".to_owned())?
                .get(&check_id)
        })
        .await
        .map_err(|e| e.to_string())??;
        let current = self.player.lock().await.current.as_deref() == Some(&id);
        if current {
            self.control_locked("stop", Value::Null)
                .await
                .map_err(|e| format!("音响未能停止，尚未移除这首音乐。请确认连接后重试：{e}"))?;
        }
        let library = self.library.clone();
        tokio::task::spawn_blocking(move || {
            library
                .lock()
                .map_err(|_| "音乐库暂时不可用".to_owned())?
                .remove(&id)
        })
        .await
        .map_err(|e| e.to_string())??;
        if current {
            let mut p = self.player.lock().await;
            p.current = None;
            p.uri = None;
            p.transport = "STOPPED".into();
            p.position = 0;
            p.duration = 0;
            p.local_duration = 0;
            p.completion = Completion::default();
        }
        drop(_guard);
        self.snapshot().await
    }

    pub async fn recognize_track_name(
        &self,
        id: String,
    ) -> Result<crate::track_names::Suggestion, String> {
        let library = self.library.clone();
        tokio::task::spawn_blocking(move || {
            library
                .lock()
                .map_err(|_| "音乐库暂时不可用".to_owned())?
                .recognize_name(&id)
        })
        .await
        .map_err(|e| e.to_string())?
    }

    pub async fn voice_command(
        &self,
        text: &str,
        expected_device: Option<&str>,
    ) -> Result<String, String> {
        let command = voice::parse(text)?;
        let _guard = self.begin_action().await;
        let selected = self.selected().await?;
        if expected_device.is_some_and(|id| id != selected.id) {
            return Err("播放音响已切换，请在语音设置里重新绑定后再点播。".into());
        }
        let result = match command {
            voice::Command::Play(query) => {
                let tracks = self.tracks().await?;
                let track = voice::match_track(&query, &tracks)?;
                self.play_locked(&track.id).await?;
                Ok(format!("已向音响发送播放：{}", track.title))
            }
            voice::Command::Control(action, value) => {
                if action == "next" || action == "previous" {
                    let tracks = self.tracks().await?;
                    let player = self.player.lock().await;
                    if next_track(
                        &tracks,
                        player.current.as_deref(),
                        &player.queue_mode,
                        if action == "next" { 1 } else { -1 },
                        false,
                    )
                    .is_none()
                    {
                        return Err("已到播放列表边界，可切换到列表循环或点播其他歌曲。".into());
                    }
                }
                self.control_locked(action, value.map(Value::from).unwrap_or(Value::Null))
                    .await?;
                Ok("音响已接受控制指令。".into())
            }
            voice::Command::Mode(mode) => {
                self.control_locked("mode", Value::from(mode)).await?;
                Ok("已更新本地播放顺序。".into())
            }
        };
        drop(_guard);
        self.poll_wake.notify_one();
        result
    }

    async fn selected(&self) -> Result<Device, String> {
        let p = self.player.lock().await;
        p.selected
            .as_ref()
            .and_then(|id| p.devices.get(id))
            .map(|(d, _)| d.clone())
            .ok_or_else(|| "请先选择音箱。".into())
    }

    pub async fn play(&self, id: &str) -> Result<Snapshot, String> {
        let _guard = self.begin_action().await;
        let result = self.play_locked(id).await;
        drop(_guard);
        self.poll_wake.notify_one();
        result?;
        self.snapshot().await
    }

    async fn check_remote_device(&self, expected: &str) -> Result<(), String> {
        let p = self.player.lock().await;
        if p.selected.as_deref() != Some(expected) {
            return Err("电脑选择的音响已改变，请刷新后重试。".into());
        }
        if p.connection.state != ConnectionState::Connected {
            return Err("音响尚未连接，请先在电脑连接音响。".into());
        }
        Ok(())
    }

    pub async fn remote_play(&self, expected: &str, id: &str) -> Result<Snapshot, String> {
        let guard = self.begin_action().await;
        self.check_remote_device(expected).await?;
        let result = self.play_locked(id).await;
        drop(guard);
        self.poll_wake.notify_one();
        result?;
        self.snapshot().await
    }

    pub async fn remote_control(&self, expected: &str, action: &str) -> Result<Snapshot, String> {
        let guard = self.begin_action().await;
        self.check_remote_device(expected).await?;
        let result = self.control_locked(action, Value::Null).await;
        drop(guard);
        self.poll_wake.notify_one();
        result
    }

    async fn play_locked(&self, id: &str) -> Result<(), String> {
        let library = self.library.clone();
        let track_id = id.to_owned();
        let (track, file) = tokio::task::spawn_blocking(move || {
            let library = library.lock().map_err(|_| "音乐库暂时不可用".to_string())?;
            let file = library.open_track(&track_id)?;
            Ok::<_, String>((library.get(&track_id)?, file))
        })
        .await
        .map_err(|e| e.to_string())??;
        let local_duration =
            tokio::task::spawn_blocking(move || crate::track_names::audio_duration(file))
                .await
                .map_err(|e| e.to_string())?;
        let device = self.selected().await?;
        let route_device = device.clone();
        let host = tokio::task::spawn_blocking(move || local_address(&route_device))
            .await
            .map_err(|e| e.to_string())??;
        let uri = self.media.url(&host, &track);
        // Disarm old playback before sending either SOAP request.
        self.player.lock().await.completion = Completion::default();
        let result = Controller::new(device)
            .play(&uri, &track.title, library::mime(&track.format))
            .await;
        let mut p = self.player.lock().await;
        if let Err(error) = result {
            p.connection.next_probe = None;
            p.error = Some(error.clone());
            return Err(error);
        }
        p.connection.connected();
        p.current = Some(id.to_string());
        p.uri = Some(uri);
        p.transport = "TRANSITIONING".into();
        p.position = 0;
        p.duration = local_duration;
        p.local_duration = local_duration;
        p.error = None;
        p.completion = Completion {
            armed: true,
            playing: None,
            started: None,
            elapsed: 0,
        };
        Ok(())
    }

    pub async fn control(&self, action: &str, value: Value) -> Result<Snapshot, String> {
        let _guard = self.begin_action().await;
        let result = self.control_locked(action, value).await;
        drop(_guard);
        self.poll_wake.notify_one();
        result
    }

    async fn control_locked(&self, action: &str, value: Value) -> Result<Snapshot, String> {
        if action == "mode" {
            let mode = value.as_str().ok_or("无效的播放模式")?;
            if !["sequential", "repeat-one", "repeat-all"].contains(&mode) {
                return Err("无效的播放模式".into());
            }
            let renderer = {
                let p = self.player.lock().await;
                p.current
                    .as_ref()
                    .filter(|_| p.connection.state == ConnectionState::Connected)
                    .and_then(|_| p.selected.as_ref())
                    .and_then(|id| p.devices.get(id))
                    .map(|(device, _)| device.clone())
            };
            if let Some(device) = renderer {
                Controller::new(device).normal_mode().await?;
            }
            self.player.lock().await.queue_mode = mode.to_string();
            return self.snapshot().await;
        }
        if action == "play-all" {
            let tracks = self.tracks().await?;
            let first = tracks.first().ok_or("曲库为空，请先添加音乐。")?;
            self.play_locked(&first.id).await?;
            self.player.lock().await.queue_mode = "sequential".into();
            return self.snapshot().await;
        }
        if action == "next" || action == "previous" {
            self.advance_locked(if action == "next" { 1 } else { -1 }, false)
                .await?;
            return self.snapshot().await;
        }
        let controller = Controller::new(self.selected().await?);
        let result = match action {
            "pause" => controller.pause().await,
            "resume" => controller.resume().await,
            "stop" => {
                self.player.lock().await.completion = Completion::default();
                controller.stop().await
            }
            "seek" => {
                let seconds = value
                    .as_u64()
                    .filter(|v| *v <= 359_999)
                    .ok_or("无效的播放位置")?;
                // Seeking invalidates the previous near-end observation.
                self.player.lock().await.completion.clear_observation();
                controller.seek(seconds).await
            }
            "volume" => {
                controller
                    .set_volume(
                        value
                            .as_u64()
                            .filter(|v| *v <= 100)
                            .ok_or("音量应为 0–100")? as u8,
                    )
                    .await
            }
            _ => return Err("不支持的播放操作".into()),
        };
        let mut p = self.player.lock().await;
        if let Err(error) = result {
            p.connection.next_probe = None;
            p.error = Some(error.clone());
            return Err(error);
        }
        p.connection.connected();
        p.error = None;
        match action {
            "pause" => {
                p.transport = "PAUSED_PLAYBACK".into();
                p.completion.pause(Instant::now());
            }
            "resume" => {
                p.transport = "TRANSITIONING".into();
                p.completion.armed = p.current.is_some();
            }
            "stop" => {
                p.transport = "STOPPED".into();
                p.position = 0;
            }
            "seek" => {
                p.position = value.as_u64().unwrap_or(0);
                p.completion.elapsed = p.position;
            }
            "volume" => p.volume = value.as_u64().map(|v| v as u8),
            _ => {}
        }
        drop(p);
        self.snapshot().await
    }

    async fn advance_locked(&self, direction: i32, automatic: bool) -> Result<(), String> {
        let tracks = self.tracks().await?;
        let p = self.player.lock().await;
        let next = next_track(
            &tracks,
            p.current.as_deref(),
            &p.queue_mode,
            direction,
            automatic,
        );
        drop(p);
        if let Some(id) = next {
            self.play_locked(&id).await?;
        }
        Ok(())
    }

    async fn poll(&self) {
        let mut activity = self.activity.subscribe();
        let Ok(_guard) = self.actions.try_lock() else {
            return;
        };
        if self
            .player
            .lock()
            .await
            .connection
            .next_probe
            .is_some_and(|at| Instant::now() < at)
        {
            return;
        }
        let Ok(device) = self.selected().await else {
            return;
        };
        let controller = Controller::new(device);
        let status = tokio::select! {
            biased;
            _ = activity.changed() => return,
            _ = self.shutdown.cancelled() => return,
            status = controller.get_status() => status,
        };
        if activity.has_changed().unwrap_or(true) {
            return;
        }
        let mut p = self.player.lock().await;
        if activity.has_changed().unwrap_or(true) {
            return;
        }
        let advance = match status {
            Ok(status) => p.observe_status(status, Instant::now()),
            Err(error) => {
                p.connection.failed(Instant::now());
                p.error = Some(error);
                p.transport = "UNAVAILABLE".into();
                p.volume = None;
                // Discard stale near-end evidence, but allow a fresh PLAYING
                // observation to preserve queue playback after a brief outage.
                p.completion.clear_observation();
                false
            }
        };
        drop(p);
        if advance && !activity.has_changed().unwrap_or(true) {
            if let Err(error) = self.advance_locked(1, true).await {
                self.player.lock().await.error = Some(error);
            }
        }
    }
}

fn next_track(
    tracks: &[Track],
    current: Option<&str>,
    mode: &str,
    direction: i32,
    automatic: bool,
) -> Option<String> {
    if tracks.is_empty() {
        return None;
    }
    let current = current.and_then(|id| tracks.iter().position(|track| track.id == id));
    let next = match current {
        None => 0,
        Some(index) if automatic && mode == "repeat-one" => index,
        Some(index) => {
            let next = index as i64 + direction as i64;
            if next < 0 || next >= tracks.len() as i64 {
                if mode == "repeat-all" {
                    next.rem_euclid(tracks.len() as i64) as usize
                } else {
                    return None;
                }
            } else {
                next as usize
            }
        }
    };
    Some(tracks[next].id.clone())
}

fn local_address(device: &Device) -> Result<String, String> {
    if let Some(ip) = &device.interface_ip {
        if let Ok(ip) = ip.parse::<std::net::Ipv4Addr>() {
            // Use the same NIC that successfully received this renderer's SSDP response.
            if if_addrs::get_if_addrs()
                .map_err(|e| e.to_string())?
                .iter()
                .any(|interface| interface.ip() == std::net::IpAddr::V4(ip))
            {
                return Ok(ip.to_string());
            }
        }
    }
    let url = url::Url::parse(&device.av_transport_url).map_err(|_| "设备控制地址无效")?;
    let host = url.host_str().ok_or("设备控制地址缺少主机")?;
    let socket = UdpSocket::bind("0.0.0.0:0").map_err(|e| format!("无法选择局域网网卡：{e}"))?;
    socket
        .connect((host, url.port_or_known_default().unwrap_or(80)))
        .map_err(|e| format!("无法连接音箱所在网络：{e}"))?;
    let address = socket.local_addr().map_err(|e| e.to_string())?.ip();
    if address.is_unspecified() {
        return Err("未找到可连接音箱的网卡，请检查 Wi-Fi 或 VPN。".into());
    }
    Ok(address.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn tracks() -> Vec<Track> {
        ["a", "b"]
            .iter()
            .map(|id| Track {
                id: id.to_string(),
                title: id.to_string(),
                filename: format!("{id}.mp3"),
                format: "mp3".into(),
                size: 1,
                source: None,
            })
            .collect()
    }
    fn status(state: &str, position: u64, duration: u64, valid: bool) -> dlna::TransportStatus {
        dlna::TransportStatus {
            transport_state: state.into(),
            position,
            duration,
            position_valid: valid,
            volume: None,
            current_uri: None,
            warnings: vec![],
        }
    }
    #[test]
    fn local_duration_handles_zero_missing_and_reset_speaker_timing() {
        for position_valid in [true, false] {
            let now = Instant::now();
            let mut player = Player {
                current: Some("a".into()),
                local_duration: 180,
                completion: Completion {
                    armed: true,
                    ..Default::default()
                },
                ..Player::default()
            };
            assert!(!player.observe_status(status("PLAYING", 0, 0, position_valid), now));
            // Timing can remain zero/unsupported throughout real playback.
            assert!(!player.observe_status(
                status("PLAYING", 0, 0, position_valid),
                now + Duration::from_secs(178)
            ));
            assert_eq!(player.duration, 180);
            assert!(player.observe_status(
                status("STOPPED", 0, 0, false),
                now + Duration::from_secs(180)
            ));
            assert!(!player.observe_status(
                status("STOPPED", 0, 0, false),
                now + Duration::from_secs(182)
            ));
        }
    }
    #[test]
    fn timing_fallback_never_interrupts_playing_or_advances_after_pause_outage_or_takeover() {
        let now = Instant::now();
        let mut c = Completion {
            armed: true,
            ..Default::default()
        };
        assert!(!c.observe("PLAYING", 0, 30, now));
        assert!(!c.observe("PLAYING", 0, 30, now + Duration::from_secs(60)));
        c.clear_observation(); // seek, pause or lost connection invalidates elapsed evidence
        assert!(!c.observe("STOPPED", 0, 0, now + Duration::from_secs(62)));
        let mut p = Player {
            current: Some("a".into()),
            uri: Some("http://pc/song".into()),
            local_duration: 30,
            completion: Completion {
                armed: true,
                ..Default::default()
            },
            ..Player::default()
        };
        p.observe_status(status("PLAYING", 29, 30, true), now);
        let mut external = status("STOPPED", 0, 0, false);
        external.current_uri = Some("http://another-controller/song".into());
        assert!(!p.observe_status(external, now + Duration::from_secs(2)));
        assert!(p.current.is_none());
        assert_eq!(p.duration, 0);
    }

    #[test]
    fn zero_progress_clock_excludes_pauses_and_respects_confirmed_seek() {
        let now = Instant::now();
        let mut c = Completion {
            armed: true,
            ..Default::default()
        };
        c.observe("PLAYING", 0, 180, now);
        c.observe("PAUSED_PLAYBACK", 0, 180, now + Duration::from_secs(80));
        c.observe("PAUSED_PLAYBACK", 0, 180, now + Duration::from_secs(200));
        assert!(!c.observe("STOPPED", 0, 0, now + Duration::from_secs(201)));
        c.observe("PLAYING", 0, 180, now + Duration::from_secs(300));
        c.observe("PLAYING", 0, 180, now + Duration::from_secs(398));
        assert!(c.observe("STOPPED", 0, 0, now + Duration::from_secs(400)));
        let mut seek = Completion {
            armed: true,
            elapsed: 170,
            ..Default::default()
        };
        assert!(!seek.observe("STOPPED", 0, 0, now));
        seek.observe("PLAYING", 0, 180, now);
        seek.observe("PLAYING", 0, 180, now + Duration::from_secs(8));
        assert!(seek.observe("STOPPED", 0, 0, now + Duration::from_secs(10)));
    }

    #[test]
    fn queue_modes_and_boundaries() {
        let tracks = tracks();
        assert_eq!(
            next_track(&tracks, Some("a"), "sequential", 1, true).as_deref(),
            Some("b")
        );
        assert_eq!(next_track(&tracks, Some("b"), "sequential", 1, true), None);
        assert_eq!(
            next_track(&tracks, Some("b"), "repeat-all", 1, true).as_deref(),
            Some("a")
        );
        assert_eq!(
            next_track(&tracks, Some("a"), "repeat-all", -1, false).as_deref(),
            Some("b")
        );
        assert_eq!(
            next_track(&tracks, Some("a"), "repeat-one", 1, true).as_deref(),
            Some("a")
        );
        assert_eq!(
            next_track(&tracks, Some("a"), "repeat-one", 1, false).as_deref(),
            Some("b")
        );
    }
    #[test]
    fn only_confirmed_near_end_playback_advances() {
        let now = Instant::now();
        let mut c = Completion {
            armed: true,
            playing: None,
            started: None,
            elapsed: 0,
        };
        assert!(!c.observe("STOPPED", 0, 0, now));
        c.armed = true;
        assert!(!c.observe("PLAYING", 12, 240, now));
        assert!(!c.observe("STOPPED", 0, 0, now + Duration::from_secs(2)));
        c.armed = true;
        c.observe("PLAYING", 238, 240, now);
        assert!(c.observe("STOPPED", 0, 0, now + Duration::from_secs(3)));
        assert!(!c.observe("STOPPED", 0, 0, now + Duration::from_secs(4)));
    }
    #[test]
    fn pause_or_explicit_stop_cannot_advance() {
        let now = Instant::now();
        let mut c = Completion {
            armed: true,
            playing: None,
            started: None,
            elapsed: 0,
        };
        c.observe("PLAYING", 238, 240, now);
        c.observe("PAUSED_PLAYBACK", 238, 240, now);
        assert!(!c.observe("STOPPED", 0, 0, now + Duration::from_secs(2)));
        c = Completion::default();
        c.observe("PLAYING", 238, 240, now);
        assert!(!c.observe("STOPPED", 0, 0, now + Duration::from_secs(2)));
    }

    #[test]
    fn startup_stop_does_not_disarm_but_stale_end_evidence_cannot_advance() {
        let now = Instant::now();
        let mut c = Completion {
            armed: true,
            playing: None,
            started: None,
            elapsed: 0,
        };
        assert!(!c.observe("STOPPED", 0, 0, now));
        assert!(c.armed);
        c.observe("PLAYING", 179, 180, now);
        assert!(!c.observe("NO_MEDIA_PRESENT", 0, 0, now + Duration::from_secs(30)));
        assert!(!c.armed);
    }
}
