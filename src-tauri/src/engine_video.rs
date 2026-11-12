//! Video is an additive domain. The existing actions mutex owns every renderer mutation.
use super::*;
use crate::{
    dlna::VideoDescriptor,
    video::{self, VideoInfo, VideoSource},
};

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
    pub task_id: String,
    pub status: String,
    pub error: Option<String>,
}

#[derive(Clone, Serialize)]
pub struct VideoPlan {
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
    pub error: Option<String>,
}

struct Prepared {
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
}

pub(super) struct VideoRuntime {
    enabled: bool,
    entry: Option<VideoSource>,
    probe: Option<ProbeSnapshot>,
    probe_cancel: Option<CancellationToken>,
    plan: Option<Prepared>,
    session: Option<RunningVideo>,
    owner_kind: &'static str,
    owner_revision: u64,
    uncertain_device: Option<Device>,
    error: Option<String>,
}

impl VideoRuntime {
    pub(super) fn new(enabled: bool) -> Self {
        Self {
            enabled,
            entry: None,
            probe: None,
            probe_cancel: None,
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
        self.video.lock().await.entry = Some(source);
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
            probe: video.probe.clone(),
            plan: video.plan.as_ref().map(|plan| plan.view.clone()),
            session: video.session.as_ref().map(|session| {
                let mut view = session.view.clone();
                view.http_requested |= self.media.video_evidence(&session.grant_id);
                view
            }),
            owner_revision: video.owner_revision,
            owner_kind: video.owner_kind.into(),
            music_active: video.owner_kind == "audio",
            error: video.error.clone(),
        }
    }

    pub async fn import_video(
        self: &Arc<Self>,
        path: PathBuf,
        ffprobe_path: PathBuf,
    ) -> Result<VideoSnapshot, String> {
        let task_id = uuid::Uuid::new_v4().to_string();
        let cancel = self.shutdown.child_token();
        {
            let mut video = self.video.lock().await;
            video.require_enabled()?;
            if let Some(previous) = video.probe_cancel.take() {
                previous.cancel();
            }
            video.entry = None;
            video.plan = None;
            video.error = None;
            video.probe = Some(ProbeSnapshot {
                task_id: task_id.clone(),
                status: "probing".into(),
                error: None,
            });
            video.probe_cancel = Some(cancel.clone());
        }
        let engine = self.clone();
        tokio::spawn(async move {
            let result = video::probe_file(path, ffprobe_path, cancel.clone()).await;
            let mut state = engine.video.lock().await;
            if state.probe.as_ref().map(|probe| &probe.task_id) != Some(&task_id)
                || cancel.is_cancelled()
            {
                return;
            }
            state.probe_cancel = None;
            match result {
                Ok(source) => {
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

    pub async fn cancel_video_probe(&self, task_id: &str) -> Result<VideoSnapshot, String> {
        {
            let mut state = self.video.lock().await;
            state.require_enabled()?;
            if state.probe.as_ref().map(|probe| probe.task_id.as_str()) != Some(task_id) {
                return Err("STALE_SESSION：该分析任务已被替换。".into());
            }
            if let Some(cancel) = state.probe_cancel.take() {
                cancel.cancel();
                if let Some(probe) = state.probe.as_mut() {
                    probe.status = "cancelled".into();
                }
            }
        }
        Ok(self.video_state().await)
    }

    pub async fn prepare_video_play(
        &self,
        media_id: &str,
        device_id: &str,
    ) -> Result<VideoSnapshot, String> {
        let source = {
            let state = self.video.lock().await;
            state.require_enabled()?;
            state
                .entry
                .as_ref()
                .filter(|entry| entry.info.id == media_id)
                .cloned()
                .ok_or("SOURCE_CHANGED：请先选择并分析影片。")?
        };
        let device = self.video_target(device_id).await?;
        let source_check = source.clone();
        tokio::task::spawn_blocking(move || source_check.open())
            .await
            .map_err(|error| error.to_string())??;
        let route_device = device.clone();
        tokio::task::spawn_blocking(move || local_address(&route_device))
            .await
            .map_err(|error| error.to_string())??;
        let caps = Controller::new(device.clone())
            .read_video_capabilities()
            .await?;
        let mut state = self.video.lock().await;
        if state.entry.as_ref().map(|entry| entry.info.id.as_str()) != Some(media_id) {
            return Err("SOURCE_CHANGED：分析期间选择的影片已改变。".into());
        }
        state.plan = Some(Prepared {
            view: VideoPlan {
                id: uuid::Uuid::new_v4().to_string(),
                media_id: media_id.into(),
                device_id: device_id.into(),
                mode: "trial".into(),
                reason: caps.reason.unwrap_or_else(|| {
                    "尚未完成真机格式验证，可试播原文件；不会转换画质或音轨。".into()
                }),
                can_pause: caps.can_pause,
                can_seek: caps.can_seek,
            },
            source,
            device_stamp: device_stamp(&device),
            device,
        });
        state.error = None;
        drop(state);
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
            if let Err(error) = Controller::new(device.clone()).stop().await {
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
        let (source, device, stamp, plan) = {
            let state = self.video.lock().await;
            state.require_enabled()?;
            let prepared = state
                .plan
                .as_ref()
                .filter(|plan| plan.view.id == plan_id)
                .ok_or("STALE_SESSION：播放计划已失效，请重新准备。")?;
            (
                prepared.source.clone(),
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
        tokio::task::spawn_blocking(move || source_check.open())
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
            state.session = Some(RunningVideo {
                view: VideoSession {
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
                },
                device: device.clone(),
                uri: grant.url.clone(),
                grant_id: grant.id.clone(),
                resource_revoked: false,
            });
            // A successful command cannot be replayed with the same plan.
            state.plan = None;
        }
        let controller = Controller::new(device.clone());
        let result = match controller.set_video_uri(&descriptor).await {
            Ok(()) => controller
                .start_video()
                .await
                .map_err(|error| format!("Play：{error}")),
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
        let result = match action {
            "stop" => controller.stop().await,
            "pause" => controller.pause().await,
            "resume" => controller.resume().await,
            "seek" => controller.seek(position_ms.unwrap_or(0) / 1000).await,
            _ => unreachable!(),
        };
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
        let mut state = self.video.lock().await;
        if activity.has_changed().unwrap_or(true)
            || !state.music_blocked()
            || state
                .session
                .as_ref()
                .is_none_or(|session| session.view.generation != generation)
        {
            return true;
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
                    .is_some_and(|uri| !uri.is_empty() && uri != &expected_uri);
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
                            && status.current_uri.as_deref() == Some(expected_uri.as_str());
                        match status.transport_state.as_str() {
                            "PLAYING" if confirmed_ours => {
                                session.view.phase = "playing".into();
                                session.view.error = None;
                            }
                            "PAUSED_PLAYBACK" if confirmed_ours => {
                                session.view.phase = "paused".into();
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
            }
        }
        true
    }
}
