//! Video is an additive domain. The existing actions mutex owns every renderer mutation.
use super::*;
use crate::video_subtitles::{self, SubtitleMode, SubtitleSelection};
use crate::{
    dlna::VideoDescriptor,
    media::VideoTransferMonitor,
    video::{self, SourceStamp, VideoInfo, VideoSource},
    video_catalog::{self, CatalogSnapshot, RecentVideo, SavedProgress},
    video_diagnostics::{SessionTrace, VideoDiagnostics},
};
use std::sync::atomic::{AtomicU8, Ordering as AtomicOrdering};

const MUSIC_BUSY: &str =
    "ACTIVE_SESSION_CONFLICT：当前正在影视播放或核对播放状态，请先在电脑切换到音乐。";

#[derive(Clone, Serialize)]
pub struct PlaybackContext {
    pub active_kind: String,
    pub owner_revision: u64,
    pub music_control_blocked_reason: Option<String>,
}

#[derive(Clone, Serialize)]
pub struct ProbeSnapshot {
    pub percent: Option<u8>,
    pub task_id: String,
    pub status: String,
    pub error: Option<String>,
}

#[derive(Clone, Serialize)]
pub struct VideoPlan {
    pub subtitle: Option<SubtitleSelection>,
    pub id: String,
    pub media_id: String,
    pub device_id: String,
    pub mode: String,
    pub reason: String,
    pub can_pause: bool,
    pub can_seek: bool,
}

#[derive(Clone, Serialize)]
pub struct VideoSession {
    pub subtitle: Option<SubtitleSelection>,
    pub id: String,
    pub generation: u64,
    pub media_id: String,
    pub device_id: String,
    pub title: String,
    pub phase: String,
    pub position_ms: Option<u64>,
    pub duration_ms: Option<u64>,
    pub can_pause: bool,
    pub can_seek: bool,
    pub command_accepted: bool,
    pub http_requested: bool,
    pub transport_state: String,
    pub error: Option<String>,
    pub diagnostics: VideoDiagnostics,
}

#[derive(Clone, Serialize)]
pub struct VideoSnapshot {
    pub enabled: bool,
    pub entry: Option<VideoInfo>,
    pub probe: Option<ProbeSnapshot>,
    pub plan: Option<VideoPlan>,
    pub session: Option<VideoSession>,
    pub owner_revision: u64,
    pub owner_kind: String,
    pub music_active: bool,
    pub recent_videos: Vec<RecentVideo>,
    pub catalog_warning: Option<String>,
    pub error: Option<String>,
}

struct Prepared {
    original: VideoSource,
    view: VideoPlan,
    source: VideoSource,
    device: Device,
    device_stamp: String,
}

struct RunningVideo {
    view: VideoSession,
    device: Device,
    uri: String,
    grant_id: String,
    resource_revoked: bool,
    network_changed: bool,
    transfer: VideoTransferMonitor,
    trace: SessionTrace,
    media: VideoInfo,
    progress_sample: Option<SavedProgress>,
    progress_saved: Option<(u64, u64)>,
    progress_saved_at: Option<Instant>,
}

pub(super) struct VideoRuntime {
    enabled: bool,
    catalog: CatalogSnapshot,
    entry: Option<VideoSource>,
    probe: Option<ProbeSnapshot>,
    probe_cancel: Option<CancellationToken>,
    subtitle_progress: Option<Arc<AtomicU8>>,
    plan: Option<Prepared>,
    session: Option<RunningVideo>,
    owner_kind: &'static str,
    owner_revision: u64,
    uncertain_device: Option<Device>,
    error: Option<String>,
}

impl VideoRuntime {
    pub(super) fn new(enabled: bool, catalog: CatalogSnapshot) -> Self {
        Self {
            enabled,
            catalog,
            entry: None,
            probe: None,
            probe_cancel: None,
            subtitle_progress: None,
            plan: None,
            session: None,
            owner_kind: "none",
            owner_revision: 0,
            uncertain_device: None,
            error: None,
        }
    }

    fn music_blocked(&self) -> bool {
        matches!(self.owner_kind, "video" | "unknown")
    }

    fn require_enabled(&self) -> Result<(), String> {
        if self.enabled {
            Ok(())
        } else {
            Err("VIDEO_DISABLED：影视功能尚未开启。".into())
        }
    }

    fn check_owner(&self, expected: u64, replace_current: bool) -> Result<(), String> {
        self.require_enabled()?;
        if expected != self.owner_revision {
            return Err("STALE_SESSION：播放状态已经改变，请刷新后重试。".into());
        }
        if self.owner_kind != "none" && !replace_current {
            return Err("ACTIVE_SESSION_CONFLICT：请明确选择切换当前播放。".into());
        }
        Ok(())
    }
}

fn device_stamp(device: &Device) -> String {
    serde_json::to_string(device).unwrap_or_default()
}

impl Engine {
    pub async fn video_enabled(&self) -> bool {
        self.video.lock().await.enabled
    }

    #[cfg(test)]
    pub(super) async fn set_video_source_for_test(&self, source: VideoSource) {
        let mut state = self.video.lock().await;
        let source = self.remember_video(&mut state, source).await;
        state.entry = Some(source);
    }

    pub(super) async fn require_music_control(&self) -> Result<(), String> {
        if self.video.lock().await.music_blocked() {
            Err(MUSIC_BUSY.into())
        } else {
            Ok(())
        }
    }

    pub(super) async fn music_owner_changed(&self, active: bool) {
        let mut video = self.video.lock().await;
        video.owner_revision = video.owner_revision.wrapping_add(1);
        video.owner_kind = if active { "audio" } else { "none" };
        video.uncertain_device = None;
    }

    pub(super) async fn music_switch_stop_unknown(&self, device: Device, error: &str) -> String {
        let message = format!(
            "REMOTE_STATE_UNKNOWN：原音箱未确认停止，未切换设备。请在电脑明确重试切换：{error}"
        );
        {
            let mut video = self.video.lock().await;
            video.owner_kind = "unknown";
            video.owner_revision = video.owner_revision.wrapping_add(1);
            video.uncertain_device = Some(device);
            video.error = Some(message.clone());
        }
        let mut player = self.player.lock().await;
        player.completion = Completion::default();
        player.transport = "UNAVAILABLE".into();
        player.error = Some(message.clone());
        message
    }

    pub(super) async fn playback_context(&self) -> PlaybackContext {
        let video = self.video.lock().await;
        PlaybackContext {
            active_kind: video.owner_kind.into(),
            owner_revision: video.owner_revision,
            music_control_blocked_reason: video.music_blocked().then(|| MUSIC_BUSY.into()),
        }
    }

    pub async fn video_state(&self) -> VideoSnapshot {
        let video = self.video.lock().await;
        VideoSnapshot {
            enabled: video.enabled,
            entry: video.entry.as_ref().map(|source| source.info.clone()),
            probe: video.probe.clone().map(|mut probe| {
                probe.percent = video
                    .subtitle_progress
                    .as_ref()
                    .map(|p| p.load(AtomicOrdering::Relaxed));
                probe
            }),
            plan: video.plan.as_ref().map(|plan| plan.view.clone()),
            session: video.session.as_ref().map(|session| {
                let mut view = session.view.clone();
                view.diagnostics = session.trace.snapshot(&session.transfer);
                view.http_requested |= view.diagnostics.transfer.bytes_streamed != "0";
                view
            }),
            owner_revision: video.owner_revision,
            owner_kind: video.owner_kind.into(),
            music_active: video.owner_kind == "audio",
            recent_videos: video.catalog.recent.clone(),
            catalog_warning: video.catalog.warning.clone(),
            error: video.error.clone(),
        }
    }

    pub async fn import_video(
        self: &Arc<Self>,
        path: PathBuf,
        ffprobe_path: PathBuf,
    ) -> Result<VideoSnapshot, String> {
        self.begin_video_probe(path, ffprobe_path, None).await
    }

    async fn begin_video_probe(
        self: &Arc<Self>,
        path: PathBuf,
        ffprobe_path: PathBuf,
        expected: Option<SourceStamp>,
    ) -> Result<VideoSnapshot, String> {
        let task_id = uuid::Uuid::new_v4().to_string();
        let cancel = self.shutdown.child_token();
        {
            let mut video = self.video.lock().await;
            video.require_enabled()?;
            if let Some(previous) = video.probe_cancel.take() {
                previous.cancel();
            }
            video.subtitle_progress = None;
            video.entry = None;
            video.plan = None;
            video.error = None;
            video.probe = Some(ProbeSnapshot {
                percent: None,
                task_id: task_id.clone(),
                status: "probing".into(),
                error: None,
            });
            video.probe_cancel = Some(cancel.clone());
        }
        let engine = self.clone();
        tokio::spawn(async move {
            let result = video::probe_file(path, ffprobe_path, cancel.clone())
                .await
                .and_then(|source| {
                    if expected
                        .as_ref()
                        .is_some_and(|expected| *expected != source.stamp())
                    {
                        Err("SOURCE_CHANGED：影片已在分析期间改变，请重新选择文件。".into())
                    } else {
                        Ok(source)
                    }
                });
            let mut state = engine.video.lock().await;
            if state.probe.as_ref().map(|probe| &probe.task_id) != Some(&task_id)
                || cancel.is_cancelled()
            {
                return;
            }
            state.probe_cancel = None;
            match result {
                Ok(source) => {
                    let source = engine.remember_video(&mut state, source).await;
                    state.entry = Some(source);
                    if let Some(probe) = state.probe.as_mut() {
                        probe.status = "ready".into();
                    }
                }
                Err(error) => {
                    state.error = Some(error.clone());
                    if let Some(probe) = state.probe.as_mut() {
                        probe.status = "failed".into();
                        probe.error = Some(error);
                    }
                }
            }
        });
        Ok(self.video_state().await)
    }

    async fn remember_video(&self, state: &mut VideoRuntime, source: VideoSource) -> VideoSource {
        let Some(catalog) = self.video_catalog.clone() else {
            return source;
        };
        let fallback = source.clone();
        match tokio::task::spawn_blocking(move || {
            let mut catalog = catalog.lock().map_err(|_| "最近影片索引暂不可用。")?;
            let mut source = source;
            let _ = catalog.remember(&mut source);
            Ok::<_, String>((source, catalog.snapshot()))
        })
        .await
        {
            Ok(Ok((source, snapshot))) => {
                state.catalog = snapshot;
                source
            }
            _ => {
                state.catalog.warning = Some("最近影片记录未能保存，原文件仍可投送。".into());
                fallback
            }
        }
    }

    pub async fn select_recent_video(
        self: &Arc<Self>,
        id: &str,
        ffprobe: PathBuf,
    ) -> Result<VideoSnapshot, String> {
        self.video.lock().await.require_enabled()?;
        let catalog = self.video_catalog.clone().ok_or("最近影片索引不可用。")?;
        let id = id.to_owned();
        let stamp = tokio::task::spawn_blocking(move || {
            let stamp = catalog
                .lock()
                .map_err(|_| "最近影片索引暂不可用。")?
                .source(&id)?;
            stamp.validate()?;
            Ok::<_, String>(stamp)
        })
        .await
        .map_err(|_| "影片记录核对未完成。")??;
        self.begin_video_probe(stamp.path.clone(), ffprobe, Some(stamp))
            .await
    }

    pub async fn forget_recent_video(&self, id: &str) -> Result<VideoSnapshot, String> {
        let mut state = self.video.lock().await;
        state.require_enabled()?;
        let catalog = self.video_catalog.clone().ok_or("最近影片索引不可用。")?;
        let id = id.to_owned();
        let (result, snapshot) = tokio::task::spawn_blocking(move || {
            let mut catalog = catalog.lock().map_err(|_| "最近影片索引暂不可用。")?;
            let result = catalog.forget(&id);
            Ok::<_, String>((result, catalog.snapshot()))
        })
        .await
        .map_err(|_| "影片记录移除未完成。")??;
        state.catalog = snapshot;
        result?;
        drop(state);
        Ok(self.video_state().await)
    }

    async fn checkpoint_video(&self, state: &mut VideoRuntime, force: bool) {
        let Some(session) = state.session.as_mut() else {
            return;
        };
        if session.view.generation != state.owner_revision {
            return;
        }
        let Some(sample) = session.progress_sample.clone() else {
            return;
        };
        let key = (sample.position_ms, sample.duration_ms);
        if session.progress_saved == Some(key)
            || (!force
                && session
                    .progress_saved_at
                    .is_some_and(|last| last.elapsed() < Duration::from_secs(30)))
        {
            return;
        }
        let Some(catalog) = self.video_catalog.clone() else {
            return;
        };
        let id = session.view.media_id.clone();
        session.progress_saved_at = Some(Instant::now());
        match tokio::task::spawn_blocking(move || {
            let mut catalog = catalog.lock().map_err(|_| "最近影片索引暂不可用。")?;
            let result = catalog.progress(&id, sample);
            Ok::<_, String>((result, catalog.snapshot()))
        })
        .await
        {
            Ok(Ok((result, snapshot))) => {
                state.catalog = snapshot;
                if result.is_ok() {
                    session.progress_saved = Some(key);
                }
            }
            _ => state.catalog.warning = Some("播放位置未保存；影片播放不受影响。".into()),
        }
    }

    pub async fn cancel_video_probe(&self, task_id: &str) -> Result<VideoSnapshot, String> {
        {
            let mut state = self.video.lock().await;
            state.require_enabled()?;
            if state.probe.as_ref().map(|probe| probe.task_id.as_str()) != Some(task_id) {
                return Err("STALE_SESSION：该分析任务已被替换。".into());
            }
            if let Some(cancel) = state.probe_cancel.take() {
                cancel.cancel();
                state.subtitle_progress = None;
                if let Some(probe) = state.probe.as_mut() {
                    probe.status = "cancelled".into();
                }
            }
        }
        Ok(self.video_state().await)
    }

    pub async fn prepare_video_play(
        self: &Arc<Self>,
        media_id: &str,
        device_id: &str,
    ) -> Result<VideoSnapshot, String> {
        self.prepare_video_variant(media_id, device_id, None).await
    }

    pub async fn prepare_video_variant(
        self: &Arc<Self>,
        media_id: &str,
        device_id: &str,
        subtitle: Option<(SubtitleSelection, (PathBuf, PathBuf))>,
    ) -> Result<VideoSnapshot, String> {
        let task_id = uuid::Uuid::new_v4().to_string();
        let cancel = self.shutdown.child_token();
        let percent = Arc::new(AtomicU8::new(0));
        let source = {
            let mut state = self.video.lock().await;
            state.require_enabled()?;
            let source = state
                .entry
                .as_ref()
                .filter(|e| e.info.id == media_id)
                .cloned()
                .ok_or("SOURCE_CHANGED：请先选择并分析影片。")?;
            if let Some((choice, _)) = &subtitle {
                video_subtitles::codec(&source, *choice)?;
            }
            if let Some(previous) = state.probe_cancel.take() {
                previous.cancel();
            }
            state.plan = None;
            state.error = None;
            state.probe_cancel = Some(cancel.clone());
            state.probe = Some(ProbeSnapshot {
                task_id: task_id.clone(),
                percent: None,
                status: if subtitle.is_some() {
                    "subtitles"
                } else {
                    "checking"
                }
                .into(),
                error: None,
            });
            state.subtitle_progress = subtitle.as_ref().map(|_| percent.clone());
            source
        };
        let engine = self.clone();
        let device_id = device_id.to_owned();
        let background = subtitle.is_some();
        let prepare = async move {
            let result = async {
                let device = engine.video_target(&device_id).await?;
                let original = source.clone();
                let route_device = device.clone();
                tokio::task::spawn_blocking(move || {
                    original.open()?;
                    local_address(&route_device)
                })
                .await
                .map_err(|error| error.to_string())??;
                let caps = Controller::new(device.clone())
                    .read_video_capabilities()
                    .await?;
                let choice = subtitle.as_ref().map(|(choice, _)| *choice);
                let resource = if let Some((choice, tool)) = subtitle {
                    video_subtitles::prepare(
                        &source,
                        choice,
                        tool,
                        &cancel,
                        &percent,
                        &engine.subtitle_work_dir,
                    )
                    .await?
                } else {
                    source.clone()
                };
                let reason = match choice.map(|c| c.mode) {
                    Some(SubtitleMode::Remux) => {
                        "已保留原画面和音轨，并将所选字幕设为唯一的默认、强制字幕轨。".into()
                    }
                    Some(SubtitleMode::Burn) => "所选字幕已合入 1080p 画面，原音轨已复制。".into(),
                    Some(SubtitleMode::BurnHdr10) => {
                        "所选字幕已合入原分辨率 HDR10 画面，原音轨已复制。".into()
                    }
                    None => caps.reason.unwrap_or_else(|| {
                        "尚未完成真机格式验证，可试播原文件；不会转换画质或音轨。".into()
                    }),
                };
                Ok::<_, String>(Prepared {
                    view: VideoPlan {
                        id: uuid::Uuid::new_v4().to_string(),
                        media_id: source.info.id.clone(),
                        device_id,
                        mode: "trial".into(),
                        reason,
                        subtitle: choice,
                        can_pause: caps.can_pause,
                        can_seek: caps.can_seek,
                    },
                    original: source,
                    source: resource,
                    device_stamp: device_stamp(&device),
                    device,
                })
            }
            .await;
            let mut state = engine.video.lock().await;
            if cancel.is_cancelled() || state.probe.as_ref().map(|p| &p.task_id) != Some(&task_id) {
                return Err("STALE_SESSION：准备任务已取消或替换。".into());
            }
            state.probe_cancel = None;
            state.subtitle_progress = None;
            let probe = state.probe.as_mut().expect("checked task ID");
            match result {
                Ok(prepared) => {
                    probe.status = "ready".into();
                    state.plan = Some(prepared);
                    Ok(())
                }
                Err(error) => {
                    probe.status = "failed".into();
                    probe.error = Some(error.clone());
                    Err(error)
                }
            }
        };
        if background {
            tokio::spawn(prepare);
        } else {
            prepare.await?;
        }
        Ok(self.video_state().await)
    }

    async fn video_target(&self, id: &str) -> Result<Device, String> {
        self.player
            .lock()
            .await
            .devices
            .get(id)
            .map(|(device, _)| device.clone())
            .ok_or_else(|| "TARGET_UNAVAILABLE：设备已离线，请重新搜索。".into())
    }

    /// Called only while holding actions. A failed Stop retains ownership.
    async fn stop_for_transfer(&self) -> Result<(), String> {
        {
            let mut state = self.video.lock().await;
            self.checkpoint_video(&mut state, true).await;
        }
        let (kind, uncertain, video_device) = {
            let state = self.video.lock().await;
            (
                state.owner_kind,
                state.uncertain_device.clone(),
                state.session.as_ref().map(|session| session.device.clone()),
            )
        };
        let previous = match kind {
            "none" => None,
            "audio" => Some(self.selected().await?),
            "video" => video_device,
            _ => uncertain.or(video_device),
        };
        self.player.lock().await.completion = Completion::default();
        if let Some(device) = previous {
            self.record_video_command("Stop", "pending").await;
            let result = Controller::new(device.clone()).stop().await;
            self.record_video_command("Stop", if result.is_ok() { "accepted" } else { "failed" })
                .await;
            if let Err(error) = result {
                let mut state = self.video.lock().await;
                let owns_video = state
                    .session
                    .as_ref()
                    .is_some_and(|session| session.view.generation == state.owner_revision);
                state.owner_kind = "unknown";
                state.owner_revision = state.owner_revision.wrapping_add(1);
                let revision = state.owner_revision;
                state.uncertain_device = Some(device);
                let message =
                    format!("REMOTE_STATE_UNKNOWN：原设备未确认停止，尚未启动新的播放：{error}");
                state.error = Some(message.clone());
                if owns_video {
                    if let Some(session) = state.session.as_mut() {
                        session.view.generation = revision;
                        session.view.phase = "remote_state_unknown".into();
                        session.view.error = Some(message.clone());
                    }
                }
                drop(state);
                self.player.lock().await.transport = "UNAVAILABLE".into();
                return Err(message);
            }
        }
        {
            let mut state = self.video.lock().await;
            if let Some(session) = state.session.as_mut() {
                session.view.http_requested |= self.media.video_evidence(&session.grant_id);
                self.media.revoke_video(&session.grant_id);
                session.view.phase = "stopped".into();
                session.view.transport_state = "STOPPED".into();
            }
            state.owner_kind = "none";
            state.owner_revision = state.owner_revision.wrapping_add(1);
            state.uncertain_device = None;
        }
        let mut player = self.player.lock().await;
        player.current = None;
        player.uri = None;
        player.transport = "STOPPED".into();
        player.position = 0;
        player.duration = 0;
        player.local_duration = 0;
        Ok(())
    }

    pub async fn start_video_play(
        &self,
        plan_id: &str,
        expected_owner_revision: u64,
        replace_current: bool,
    ) -> Result<VideoSnapshot, String> {
        // Preparation is read-only and does not hold the music mutation lock.
        let (source, original, device, stamp, plan) = {
            let state = self.video.lock().await;
            state.require_enabled()?;
            let prepared = state
                .plan
                .as_ref()
                .filter(|plan| plan.view.id == plan_id)
                .ok_or("STALE_SESSION：播放计划已失效，请重新准备。")?;
            (
                prepared.source.clone(),
                prepared.original.clone(),
                prepared.device.clone(),
                prepared.device_stamp.clone(),
                prepared.view.clone(),
            )
        };
        let guard = self.begin_action().await;
        {
            let state = self.video.lock().await;
            state.check_owner(expected_owner_revision, replace_current)?;
            if state.plan.as_ref().map(|plan| plan.view.id.as_str()) != Some(plan_id) {
                return Err("STALE_SESSION：播放计划已被替换。".into());
            }
        }
        if device_stamp(&self.video_target(&device.id).await?) != stamp {
            return Err("TARGET_UNAVAILABLE：设备地址已经改变，请重新准备。".into());
        }
        let source_check = source.clone();
        tokio::task::spawn_blocking(move || {
            if source_check.temporary.is_some() {
                original.open()?;
            }
            source_check.open()
        })
        .await
        .map_err(|error| error.to_string())??;
        let route_device = device.clone();
        let host = tokio::task::spawn_blocking(move || local_address(&route_device))
            .await
            .map_err(|error| error.to_string())??;
        // Allocate and validate before stopping music. No receiver has its URL yet.
        let grant = self.media.register_video(&host, source.clone())?;
        let descriptor = VideoDescriptor {
            uri: grant.url.clone(),
            title: source.info.title.clone(),
            mime: video::mime(&source.info.format).into(),
        };
        if let Err(error) = descriptor.validate() {
            self.media.revoke_video(&grant.id);
            return Err(error);
        }
        if let Err(error) = self.stop_for_transfer().await {
            self.media.revoke_video(&grant.id);
            return Err(error);
        }
        {
            let mut state = self.video.lock().await;
            state.owner_revision = state.owner_revision.wrapping_add(1);
            state.owner_kind = "video";
            state.error = None;
            let trace = SessionTrace::new();
            state.session = Some(RunningVideo {
                view: VideoSession {
                    subtitle: plan.subtitle,
                    id: uuid::Uuid::new_v4().to_string(),
                    generation: state.owner_revision,
                    media_id: source.info.id.clone(),
                    device_id: device.id.clone(),
                    title: source.info.title.clone(),
                    phase: "starting".into(),
                    position_ms: None,
                    duration_ms: source.info.duration_ms,
                    can_pause: plan.can_pause,
                    can_seek: plan.can_seek,
                    command_accepted: false,
                    http_requested: false,
                    transport_state: "TRANSITIONING".into(),
                    error: None,
                    diagnostics: trace.snapshot(&grant.transfer),
                },
                device: device.clone(),
                uri: grant.url.clone(),
                grant_id: grant.id.clone(),
                resource_revoked: false,
                network_changed: false,
                transfer: grant.transfer.clone(),
                trace,
                media: source.info.clone(),
                progress_sample: None,
                progress_saved: None,
                progress_saved_at: None,
            });
            // A successful command cannot be replayed with the same plan.
            state.plan = None;
        }
        let controller = Controller::new(device.clone());
        self.record_video_command("SetAVTransportURI", "pending")
            .await;
        let set_result = controller.set_video_uri(&descriptor).await;
        self.record_video_command(
            "SetAVTransportURI",
            if set_result.is_ok() {
                "accepted"
            } else {
                "failed"
            },
        )
        .await;
        let result = match set_result {
            Ok(()) => {
                self.record_video_command("Play", "pending").await;
                let result = controller.start_video().await;
                self.record_video_command(
                    "Play",
                    if result.is_ok() { "accepted" } else { "failed" },
                )
                .await;
                result.map_err(|error| format!("Play：{error}"))
            }
            Err(error) => Err(format!("SetAVTransportURI：{error}")),
        };
        let mut state = self.video.lock().await;
        match result {
            Ok(()) => {
                if let Some(session) = state.session.as_mut() {
                    session.view.phase = "awaiting_evidence".into();
                    session.view.command_accepted = true;
                }
            }
            Err(error) => {
                self.media.revoke_video(&grant.id);
                let message = format!("REMOTE_STATE_UNKNOWN：启动未确认完成（{error}），请核对电视状态后停止或重新准备。");
                state.owner_kind = "unknown";
                state.uncertain_device = Some(device);
                state.error = Some(message.clone());
                if let Some(session) = state.session.as_mut() {
                    session.view.phase = "remote_state_unknown".into();
                    session.view.error = Some(message.clone());
                    session.resource_revoked = true;
                }
                drop(state);
                drop(guard);
                self.poll_wake.notify_one();
                return Err(message);
            }
        }
        drop(state);
        drop(guard);
        self.poll_wake.notify_one();
        Ok(self.video_state().await)
    }

    async fn record_video_command(&self, action: &str, outcome: &str) {
        let mut state = self.video.lock().await;
        let revision = state.owner_revision;
        if let Some(session) = state
            .session
            .as_mut()
            .filter(|s| s.view.generation == revision)
        {
            session.trace.command(action, outcome);
        }
    }

    /// Re-read identity, route and transport. Never restart, stop, seek or change URI here.
    pub async fn recheck_video_session(
        &self,
        session_id: &str,
        expected_generation: u64,
    ) -> Result<VideoSnapshot, String> {
        let guard = self.begin_action().await;
        let (previous, expected_uri) = {
            let state = self.video.lock().await;
            state.require_enabled()?;
            let session = state
                .session
                .as_ref()
                .filter(|s| s.view.id == session_id && s.view.generation == expected_generation)
                .ok_or("STALE_SESSION：该影片会话已被替换。")?;
            if !state.music_blocked()
                || state.owner_revision != expected_generation
                || state
                    .uncertain_device
                    .as_ref()
                    .is_some_and(|d| d.id != session.device.id)
            {
                return Err("STALE_SESSION：该影片已不再拥有控制权。".into());
            }
            (session.device.clone(), session.uri.clone())
        };
        let candidate = self
            .video_target(&previous.id)
            .await
            .unwrap_or(previous.clone());
        let result = async {
            let interface = candidate.interface_ip.as_deref().and_then(|ip| ip.parse().ok());
            let current = dlna::fetch_device_by_id(&candidate.location, &previous.id, interface).await?;
            let Some(current) = current else {
                // The old address now belongs to another renderer. Never send it Stop.
                {
                    let mut state = self.video.lock().await;
                    if let Some(session) = state.session.as_mut() {
                        self.media.revoke_video(&session.grant_id);
                        session.resource_revoked = true;
                        session.view.phase = "detached".into();
                        session.view.error = Some("设备身份已经改变，旧影片授权已关闭；请重新搜索电视。".into());
                        session.trace.poll(false);
                    }
                    state.owner_kind = "none";
                    state.owner_revision = state.owner_revision.wrapping_add(1);
                    state.uncertain_device = None;
                }
                let mut player = self.player.lock().await;
                if player.devices.get(&previous.id).is_some_and(|(device, _)| device.location == candidate.location) {
                    player.devices.remove(&previous.id);
                }
                return Err("TARGET_UNAVAILABLE：设备身份已经改变，旧影片授权已关闭；请重新搜索电视。".into());
            };
            let route = current.clone();
            let host = tokio::task::spawn_blocking(move || local_address(&route))
                .await.map_err(|error| error.to_string())??;
            // Update only after reading the same device identity. A later explicit Stop
            // may use this address; no control is sent during this recheck.
            {
                let mut state = self.video.lock().await;
                if let Some(session) = state.session.as_mut() {
                    session.device = current.clone();
                }
                if state.uncertain_device.as_ref().is_some_and(|d| d.id == previous.id) {
                    state.uncertain_device = Some(current.clone());
                }
            }
            let old_host = url::Url::parse(&expected_uri).ok()
                .and_then(|url| url.host_str().map(str::to_owned));
            let changed = old_host.as_deref() != Some(host.as_str());
            if let Some(session) = self.video.lock().await.session.as_mut() {
                session.network_changed = changed;
            }
            if changed {
                return Err("NETWORK_CHANGED：本机网络地址已改变，请明确停止本次播放，再重新准备影片；不会自动重新投送。".into());
            }
            Controller::new(current).get_status().await
        }.await;
        let error = result.as_ref().err().cloned();
        self.apply_video_observation(expected_generation, &expected_uri, result, None)
            .await;
        drop(guard);
        if let Some(error) = error {
            return Err(error);
        }
        Ok(self.video_state().await)
    }

    /// Allowlist export: exclude identifiers, titles, paths, addresses, URLs and raw errors.
    pub async fn video_diagnostic_report(
        &self,
        session_id: &str,
        expected_generation: u64,
    ) -> Result<Value, String> {
        let state = self.video.lock().await;
        state.require_enabled()?;
        let session = state
            .session
            .as_ref()
            .filter(|s| s.view.id == session_id && s.view.generation == expected_generation)
            .ok_or("STALE_SESSION：该影片会话已被替换，请导出当前会话。")?;
        let codec = session
            .media
            .video_codec
            .as_deref()
            .filter(|codec| {
                matches!(
                    *codec,
                    "h264"
                        | "hevc"
                        | "av1"
                        | "mpeg2video"
                        | "mpeg4"
                        | "vp8"
                        | "vp9"
                        | "vc1"
                        | "png"
                )
            })
            .unwrap_or("other");
        let diagnostics = session.trace.snapshot(&session.transfer);
        Ok(serde_json::json!({
            "schema_version": 1,
            "app_version": env!("CARGO_PKG_VERSION"),
            "phase": session.view.phase,
            "transport_state": session.view.transport_state,
            "position_ms": session.view.position_ms,
            "duration_ms": session.view.duration_ms,
            "command_accepted": session.view.command_accepted,
            "http_requested": session.view.http_requested || diagnostics.transfer.bytes_streamed != "0",
            "subtitle": session.view.subtitle,
            "media": {
                "container": session.media.format,
                "video_codec": codec,
                "width": session.media.width, "height": session.media.height,
                "audio_track_count": session.media.audio_tracks.len(),
                "subtitle_track_count": session.media.subtitle_tracks.len(),
            },
            "diagnostics": diagnostics,
            "notice": "HTTP字节仅表示已提供给响应流，含重复范围；不是已观看进度，也不是电视解码结果。"
        }))
    }

    async fn check_music_source(&self, track_id: &str) -> Result<(), String> {
        let library = self.library.clone();
        let id = track_id.to_owned();
        tokio::task::spawn_blocking(move || {
            library
                .lock()
                .map_err(|_| "音乐库暂时不可用".to_owned())?
                .open_track(&id)
        })
        .await
        .map_err(|error| error.to_string())??;
        Ok(())
    }

    pub async fn start_music_play(
        &self,
        track_id: &str,
        device_id: &str,
        expected_owner_revision: u64,
        replace_current: bool,
    ) -> Result<Snapshot, String> {
        self.video_target(device_id).await?;
        self.check_music_source(track_id).await?;
        let guard = self.begin_action().await;
        self.video
            .lock()
            .await
            .check_owner(expected_owner_revision, replace_current)?;
        // The earlier read may have waited behind another mutation. Validate
        // again before making any change to the currently playing video.
        let device = self.video_target(device_id).await?;
        self.check_music_source(track_id).await?;
        let route_device = device.clone();
        tokio::task::spawn_blocking(move || local_address(&route_device))
            .await
            .map_err(|error| error.to_string())??;
        self.stop_for_transfer().await?;
        {
            let mut player = self.player.lock().await;
            player.selected = Some(device.id.clone());
            player.connection = Connection::default();
            player.volume = None;
            player.error = None;
        }
        if let Err(error) = self.play_locked(track_id).await {
            let mut state = self.video.lock().await;
            state.owner_kind = "unknown";
            state.uncertain_device = Some(device);
            state.error = Some(format!("REMOTE_STATE_UNKNOWN：音乐启动未确认完成：{error}"));
            return Err(state.error.clone().unwrap());
        }
        drop(guard);
        self.poll_wake.notify_one();
        self.snapshot().await
    }

    pub async fn control_video(
        &self,
        session_id: &str,
        expected_generation: u64,
        action: &str,
        position_ms: Option<u64>,
    ) -> Result<VideoSnapshot, String> {
        let guard = self.begin_action().await;
        let device = {
            let state = self.video.lock().await;
            state.require_enabled()?;
            let session = state
                .session
                .as_ref()
                .filter(|session| {
                    session.view.id == session_id && session.view.generation == expected_generation
                })
                .ok_or("STALE_SESSION：该影片会话已经结束或被替换。")?;
            if !state.music_blocked()
                || state.owner_revision != session.view.generation
                || state
                    .uncertain_device
                    .as_ref()
                    .is_some_and(|device| device.id != session.device.id)
            {
                return Err("STALE_SESSION：该影片已不再拥有控制权。".into());
            }
            if state.owner_kind == "unknown" && action != "stop" {
                return Err("REMOTE_STATE_UNKNOWN：请先停止或核对设备状态。".into());
            }
            match action {
                "stop" => {}
                "pause" | "resume" if session.view.can_pause => {}
                "seek" if session.view.can_seek => {
                    let position = position_ms
                        .filter(|position| *position <= 359_999_000)
                        .ok_or("无效的播放位置")?;
                    if session
                        .view
                        .duration_ms
                        .is_some_and(|duration| position > duration)
                    {
                        return Err("无效的播放位置".into());
                    }
                }
                _ => return Err("ACTION_UNSUPPORTED：设备未声明支持此播放操作。".into()),
            }
            session.device.clone()
        };
        let controller = Controller::new(device.clone());
        let command = match action {
            "pause" => "Pause",
            "resume" => "Play",
            "stop" => "Stop",
            "seek" => "Seek",
            _ => unreachable!(),
        };
        self.record_video_command(command, "pending").await;
        let result = match action {
            "stop" => controller.stop().await,
            "pause" => controller.pause().await,
            "resume" => controller.resume().await,
            "seek" => controller.seek(position_ms.unwrap_or(0) / 1000).await,
            _ => unreachable!(),
        };
        self.record_video_command(command, if result.is_ok() { "accepted" } else { "failed" })
            .await;
        let mut state = self.video.lock().await;
        if let Err(error) = result {
            state.owner_kind = "unknown";
            state.uncertain_device = Some(device);
            let message = format!("REMOTE_STATE_UNKNOWN：{action} 未确认完成：{error}");
            state.error = Some(message.clone());
            if let Some(session) = state.session.as_mut() {
                session.view.phase = "remote_state_unknown".into();
                session.view.error = Some(message.clone());
            }
            return Err(message);
        }
        if let Some(session) = state.session.as_mut() {
            session.view.error = None;
            match action {
                "stop" => {
                    session.view.http_requested |= self.media.video_evidence(&session.grant_id);
                    self.media.revoke_video(&session.grant_id);
                    session.view.phase = "stopped".into();
                    session.view.transport_state = "STOPPED".into();
                }
                "pause" => {
                    session.view.phase = "paused".into();
                    session.view.transport_state = "PAUSED_PLAYBACK".into();
                }
                "resume" => {
                    session.view.phase = "awaiting_evidence".into();
                    session.view.transport_state = "TRANSITIONING".into();
                }
                "seek" => {
                    session.view.position_ms = position_ms.map(|position| position / 1000 * 1000)
                }
                _ => {}
            }
        }
        state.error = None;
        if matches!(action, "stop" | "pause") {
            self.checkpoint_video(&mut state, true).await;
        }
        if action == "stop" {
            state.owner_kind = "none";
            state.owner_revision = state.owner_revision.wrapping_add(1);
            state.uncertain_device = None;
        }
        drop(state);
        drop(guard);
        self.poll_wake.notify_one();
        Ok(self.video_state().await)
    }

    /// Return true when video/unknown ownership consumed this poll. Caller holds actions.
    pub(super) async fn poll_video_locked(&self, activity: &mut watch::Receiver<u64>) -> bool {
        let (device, generation, expected_uri) = {
            let state = self.video.lock().await;
            if !state.music_blocked() {
                return false;
            }
            let Some(session) = state.session.as_ref() else {
                return true;
            };
            if session.network_changed {
                return true;
            }
            if session
                .trace
                .next_poll
                .is_some_and(|next| Instant::now() < next)
            {
                return true;
            }
            if state.owner_revision != session.view.generation
                || state
                    .uncertain_device
                    .as_ref()
                    .is_some_and(|device| device.id != session.device.id)
            {
                return true;
            }
            (
                session.device.clone(),
                session.view.generation,
                session.uri.clone(),
            )
        };
        let controller = Controller::new(device);
        let result = tokio::select! {
            biased;
            _ = activity.changed() => return true,
            _ = self.shutdown.cancelled() => return true,
            result = controller.get_status() => result,
        };
        if activity.has_changed().unwrap_or(true) {
            return true;
        }
        self.apply_video_observation(generation, &expected_uri, result, Some(activity))
            .await;
        true
    }

    async fn apply_video_observation(
        &self,
        generation: u64,
        expected_uri: &str,
        result: Result<dlna::TransportStatus, String>,
        activity: Option<&watch::Receiver<u64>>,
    ) {
        let mut state = self.video.lock().await;
        if activity.is_some_and(|activity| activity.has_changed().unwrap_or(true))
            || !state.music_blocked()
            || state
                .session
                .as_ref()
                .is_none_or(|session| session.view.generation != generation)
        {
            return;
        }
        if let Some(session) = state.session.as_mut() {
            session.trace.poll(result.is_ok());
        }
        match result {
            Err(error) => {
                state.owner_kind = "unknown";
                if let Some(session) = state.session.as_mut() {
                    session.view.phase = "remote_state_unknown".into();
                    session.view.error = Some(error);
                }
            }
            Ok(status) => {
                let taken_over = status
                    .current_uri
                    .as_ref()
                    .is_some_and(|uri| !uri.is_empty() && uri != expected_uri);
                let mut release = false;
                let mut confirmed_ours = false;
                let mut uncertain_stop = false;
                if let Some(session) = state.session.as_mut() {
                    session.view.http_requested |= self.media.video_evidence(&session.grant_id);
                    session.view.transport_state = status.transport_state.clone();
                    if taken_over {
                        session.view.phase = "detached".into();
                        release = true;
                    } else {
                        if status.position_valid {
                            session.view.position_ms = Some(status.position.saturating_mul(1000));
                            if status.duration > 0 {
                                session.view.duration_ms =
                                    Some(status.duration.saturating_mul(1000));
                            }
                        }
                        // Recovery from uncertain SOAP requires actual URI ownership evidence.
                        confirmed_ours = !session.resource_revoked
                            && status.current_uri.as_deref() == Some(expected_uri);
                        if confirmed_ours
                            && status.position_valid
                            && status.duration > 0
                            && status.position <= status.duration
                            && status.duration <= 359_999
                            && matches!(
                                status.transport_state.as_str(),
                                "PLAYING" | "PAUSED_PLAYBACK"
                            )
                        {
                            session.progress_sample = Some(SavedProgress {
                                position_ms: status.position * 1000,
                                duration_ms: status.duration * 1000,
                                observed_at_ms: video_catalog::now_ms(),
                                device_id: Some(session.device.id.clone()),
                            });
                        }
                        match status.transport_state.as_str() {
                            "PLAYING" if confirmed_ours => {
                                session.view.phase = "playing".into();
                                session.view.error = None;
                            }
                            "PAUSED_PLAYBACK" if confirmed_ours => {
                                session.view.phase = "paused".into();
                                session.view.error = None;
                            }
                            "TRANSITIONING" if confirmed_ours => {
                                session.view.phase = "awaiting_evidence".into();
                                session.view.error = None;
                            }
                            "STOPPED" | "NO_MEDIA_PRESENT" if !session.resource_revoked => {
                                // Receivers may temporarily report STOPPED while seeking
                                // or buffering. M0 deliberately requires explicit Stop
                                // (or foreign-URI takeover) to revoke the file grant.
                                session.view.phase = "remote_state_unknown".into();
                                session.view.error = Some("电视暂报停止，文件服务仍保留；等待状态恢复，或明确停止本次播放。".into());
                                uncertain_stop = true;
                                confirmed_ours = false;
                            }
                            _ => {}
                        }
                    }
                    if release {
                        self.media.revoke_video(&session.grant_id);
                    }
                }
                if release {
                    state.owner_kind = "none";
                    state.owner_revision = state.owner_revision.wrapping_add(1);
                    state.uncertain_device = None;
                } else if uncertain_stop {
                    state.owner_kind = "unknown";
                } else if confirmed_ours {
                    state.owner_kind = "video";
                    state.uncertain_device = None;
                    state.error = None;
                }
                if confirmed_ours {
                    self.checkpoint_video(&mut state, status.transport_state == "PAUSED_PLAYBACK")
                        .await;
                }
            }
        }
    }
}
