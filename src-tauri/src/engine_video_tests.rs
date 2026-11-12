use super::*;
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Router,
};

#[derive(Default)]
struct Renderer {
    actions: Vec<String>,
    uri: String,
    transport: String,
    position: u64,
    fail: Option<String>,
    received: Vec<u8>,
    block_position: bool,
    position_started: Arc<Notify>,
    release_position: CancellationToken,
}

struct Fixture {
    directory: tempfile::TempDir,
    engine: Arc<Engine>,
    renderer: Arc<Mutex<Renderer>>,
    server: tokio::task::JoinHandle<()>,
    device_id: String,
    track_id: String,
    video_id: String,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.engine.shutdown();
        self.server.abort();
    }
}

impl Fixture {
    async fn new() -> Self {
        Self::with_video_enabled(true).await
    }

    async fn with_video_enabled(enabled: bool) -> Self {
        async fn description() -> &'static str {
            r#"<root><device><deviceType>urn:schemas-upnp-org:device:MediaRenderer:1</deviceType><friendlyName>Video test</friendlyName><UDN>uuid:video-test</UDN><serviceList><service><serviceType>urn:schemas-upnp-org:service:AVTransport:1</serviceType><controlURL>/transport</controlURL><SCPDURL>/scpd.xml</SCPDURL></service></serviceList></device></root>"#
        }
        async fn other_description() -> String {
            description()
                .await
                .replace("uuid:video-test", "uuid:other-test")
        }
        async fn scpd() -> &'static str {
            r#"<scpd><actionList><action><name>SetAVTransportURI</name></action><action><name>Play</name></action><action><name>Pause</name></action><action><name>Stop</name></action><action><name>Seek</name></action></actionList></scpd>"#
        }
        async fn soap(
            State(state): State<Arc<Mutex<Renderer>>>,
            headers: HeaderMap,
            body: String,
        ) -> (StatusCode, String) {
            let header = headers["soapaction"].to_str().unwrap().trim_matches('"');
            let (service, action) = header.rsplit_once('#').unwrap();
            let gate = {
                let renderer = state.lock().unwrap();
                (renderer.block_position && action == "GetPositionInfo").then(|| {
                    (
                        renderer.position_started.clone(),
                        renderer.release_position.clone(),
                    )
                })
            };
            if let Some((started, release)) = gate {
                started.notify_one();
                release.cancelled().await;
            }
            let output = {
                let mut renderer = state.lock().unwrap();
                renderer.actions.push(action.into());
                if renderer.fail.as_deref() == Some(action) {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "injected renderer failure".into(),
                    );
                }
                match action {
                    "SetAVTransportURI" => {
                        let xml = roxmltree::Document::parse(&body).unwrap();
                        renderer.uri = xml.descendants().find(|node| node.has_tag_name("CurrentURI")).unwrap().text().unwrap().into();
                        renderer.position = 0;
                        String::new()
                    },
                    "Play" => { renderer.transport = "PLAYING".into(); String::new() },
                    "Stop" => { renderer.transport = "STOPPED".into(); String::new() },
                    "Pause" => { renderer.transport = "PAUSED_PLAYBACK".into(); String::new() },
                    "Seek" => {
                        let xml = roxmltree::Document::parse(&body).unwrap();
                        let target = xml.descendants().find(|node| node.has_tag_name("Target")).unwrap().text().unwrap();
                        let parts: Vec<u64> = target.split(':').map(|value| value.parse().unwrap()).collect();
                        renderer.position = parts[0] * 3600 + parts[1] * 60 + parts[2];
                        String::new()
                    },
                    "GetTransportInfo" => format!("<CurrentTransportState>{}</CurrentTransportState>", renderer.transport),
                    "GetPositionInfo" => format!("<RelTime>00:{:02}:{:02}</RelTime><TrackDuration>00:03:00</TrackDuration><TrackURI>{}</TrackURI>", renderer.position / 60, renderer.position % 60, renderer.uri.replace('&', "&amp;")),
                    _ => String::new(),
                }
            };
            if action == "Play" {
                let uri = state.lock().unwrap().uri.clone();
                if uri.contains("/video/") {
                    let response = reqwest::get(uri).await.unwrap();
                    assert_eq!(response.status(), StatusCode::OK);
                    let bytes = response.bytes().await.unwrap().to_vec();
                    state.lock().unwrap().received = bytes;
                }
            }
            (StatusCode::OK, format!("<s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\"><s:Body><u:{action}Response xmlns:u=\"{service}\">{output}</u:{action}Response></s:Body></s:Envelope>"))
        }
        let directory = tempfile::tempdir().unwrap();
        let engine = Engine::new_with_video_enabled(directory.path().join("data"), enabled)
            .await
            .unwrap();
        let renderer = Arc::new(Mutex::new(Renderer {
            transport: "STOPPED".into(),
            ..Renderer::default()
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/description.xml", listener.local_addr().unwrap());
        let router = Router::new()
            .route("/description.xml", get(description))
            .route("/other-description.xml", get(other_description))
            .route("/scpd.xml", get(scpd))
            .route("/transport", post(soap))
            .with_state(renderer.clone());
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let device_id = engine.add_device(&url).await.unwrap().devices[0].id.clone();
        engine.select(&device_id).await.unwrap();
        let audio = directory.path().join("music.mp3");
        std::fs::write(&audio, b"ID3 music fixture").unwrap();
        engine.import(vec![audio]).await.unwrap();
        let track_id = engine.snapshot().await.unwrap().tracks[0].id.clone();
        let movie = directory.path().join("film.mkv");
        std::fs::write(&movie, b"original video bytes: no encoding").unwrap();
        let mut source = crate::video::VideoSource::open_import(movie).unwrap();
        source.info.duration_ms = Some(180_000);
        let video_id = source.info.id.clone();
        engine.set_video_source_for_test(source).await;
        Self {
            directory,
            engine,
            renderer,
            server,
            device_id,
            track_id,
            video_id,
        }
    }

    async fn prepared(&self) -> VideoSnapshot {
        self.engine
            .prepare_video_play(&self.video_id, &self.device_id)
            .await
            .unwrap()
    }

    async fn started(&self) -> VideoSnapshot {
        let prepared = self.prepared().await;
        self.engine
            .start_video_play(&prepared.plan.unwrap().id, prepared.owner_revision, true)
            .await
            .unwrap()
    }

    fn actions(&self) -> Vec<String> {
        self.renderer.lock().unwrap().actions.clone()
    }
}

#[tokio::test]
async fn direct_video_delivers_original_bytes_and_music_round_trip_preserves_queue() {
    let fixture = Fixture::new().await;
    fixture.engine.play(&fixture.track_id).await.unwrap();
    fixture
        .engine
        .control("mode", Value::from("repeat-all"))
        .await
        .unwrap();
    let before = fixture.actions();
    let prepared = fixture.prepared().await;
    assert_eq!(
        fixture.actions(),
        before,
        "preparation must not issue playback SOAP"
    );
    let state = fixture
        .engine
        .start_video_play(&prepared.plan.unwrap().id, prepared.owner_revision, true)
        .await
        .unwrap();
    let session = state.session.unwrap();
    assert!(session.command_accepted);
    assert!(session.http_requested);
    assert_eq!(
        fixture.renderer.lock().unwrap().received,
        b"original video bytes: no encoding"
    );
    let snapshot = fixture.engine.snapshot().await.unwrap();
    assert_eq!(snapshot.current_track_id, None);
    assert_eq!(snapshot.playback_context.active_kind, "video");
    assert_eq!(snapshot.queue_mode, "repeat-all");
    fixture.engine.poll().await;
    assert_eq!(
        fixture.engine.video_state().await.session.unwrap().phase,
        "playing"
    );
    fixture
        .engine
        .control_video(&session.id, session.generation, "pause", None)
        .await
        .unwrap();
    fixture
        .engine
        .control_video(&session.id, session.generation, "resume", None)
        .await
        .unwrap();
    fixture
        .engine
        .control_video(&session.id, session.generation, "seek", Some(60_000))
        .await
        .unwrap();
    assert_eq!(fixture.renderer.lock().unwrap().position, 60);
    let revision = fixture.engine.video_state().await.owner_revision;
    let music = fixture
        .engine
        .start_music_play(&fixture.track_id, &fixture.device_id, revision, true)
        .await
        .unwrap();
    assert_eq!(
        music.current_track_id.as_deref(),
        Some(fixture.track_id.as_str())
    );
    assert_eq!(music.queue_mode, "repeat-all");
    assert_eq!(music.playback_context.active_kind, "audio");
}

#[tokio::test]
async fn every_legacy_music_mutation_is_blocked_during_video() {
    let fixture = Fixture::new().await;
    fixture.started().await;
    let before = fixture.actions();
    assert!(fixture.engine.play(&fixture.track_id).await.is_err());
    assert!(fixture.engine.select(&fixture.device_id).await.is_err());
    assert!(fixture
        .engine
        .remote_play(&fixture.device_id, &fixture.track_id)
        .await
        .is_err());
    assert!(fixture
        .engine
        .remote_control(&fixture.device_id, "pause")
        .await
        .is_err());
    for (action, value) in [
        ("mode", Value::from("repeat-one")),
        ("next", Value::Null),
        ("previous", Value::Null),
        ("pause", Value::Null),
        ("resume", Value::Null),
        ("stop", Value::Null),
        ("seek", Value::from(60)),
        ("volume", Value::from(20)),
        ("play-all", Value::Null),
    ] {
        assert!(
            fixture.engine.control(action, value).await.is_err(),
            "{action}"
        );
    }
    for command in [
        "本地暂停",
        "本地继续",
        "本地停止",
        "本地下一首",
        "本地上一首",
        "本地单曲循环",
    ] {
        assert!(
            fixture.engine.voice_command(command, None).await.is_err(),
            "{command}"
        );
    }
    assert!(fixture.engine.advance_locked(1, true).await.is_err());
    assert_eq!(fixture.actions(), before);
}

#[tokio::test]
async fn prepared_plan_cannot_override_a_new_music_owner_or_changed_file() {
    let fixture = Fixture::new().await;
    fixture.engine.play(&fixture.track_id).await.unwrap();
    let prepared = fixture.prepared().await;
    fixture.engine.play(&fixture.track_id).await.unwrap();
    let before = fixture.actions();
    assert!(fixture
        .engine
        .start_video_play(&prepared.plan.unwrap().id, prepared.owner_revision, true)
        .await
        .err()
        .unwrap()
        .contains("STALE_SESSION"));
    assert_eq!(fixture.actions(), before);
    let prepared = fixture.prepared().await;
    std::fs::write(fixture.directory.path().join("film.mkv"), b"changed").unwrap();
    assert!(fixture
        .engine
        .start_video_play(&prepared.plan.unwrap().id, prepared.owner_revision, true)
        .await
        .is_err());
    assert_eq!(fixture.actions(), before);
    assert_eq!(
        fixture
            .engine
            .snapshot()
            .await
            .unwrap()
            .current_track_id
            .as_deref(),
        Some(fixture.track_id.as_str())
    );
}

#[tokio::test]
async fn failed_old_stop_prevents_set_uri_and_blocks_music_until_explicit_recovery() {
    let fixture = Fixture::new().await;
    fixture.engine.play(&fixture.track_id).await.unwrap();
    let prepared = fixture.prepared().await;
    fixture.renderer.lock().unwrap().fail = Some("Stop".into());
    let before = fixture.actions().len();
    assert!(fixture
        .engine
        .start_video_play(&prepared.plan.unwrap().id, prepared.owner_revision, true)
        .await
        .is_err());
    assert_eq!(&fixture.actions()[before..], &["Stop"]);
    assert_eq!(
        fixture
            .engine
            .snapshot()
            .await
            .unwrap()
            .playback_context
            .active_kind,
        "unknown"
    );
    assert!(fixture
        .engine
        .control("mode", Value::from("repeat-one"))
        .await
        .is_err());
    fixture.renderer.lock().unwrap().fail = None;
    let revision = fixture.engine.video_state().await.owner_revision;
    fixture
        .engine
        .start_music_play(&fixture.track_id, &fixture.device_id, revision, true)
        .await
        .unwrap();
}

#[tokio::test]
async fn partial_start_never_restores_music_or_retries_play_and_stop_revokes_resource() {
    let fixture = Fixture::new().await;
    let prepared = fixture.prepared().await;
    fixture.renderer.lock().unwrap().fail = Some("Play".into());
    assert!(fixture
        .engine
        .start_video_play(&prepared.plan.unwrap().id, prepared.owner_revision, false)
        .await
        .is_err());
    let state = fixture.engine.video_state().await;
    let session = state.session.unwrap();
    assert_eq!(session.phase, "remote_state_unknown");
    assert_eq!(
        fixture
            .actions()
            .iter()
            .filter(|action| *action == "Play")
            .count(),
        1
    );
    assert!(fixture.engine.play(&fixture.track_id).await.is_err());
    let uri = fixture.renderer.lock().unwrap().uri.clone();
    assert!(!reqwest::get(uri).await.unwrap().status().is_success());
    fixture.renderer.lock().unwrap().fail = None;
    fixture
        .engine
        .control_video(&session.id, session.generation, "stop", None)
        .await
        .unwrap();
    assert_eq!(
        fixture
            .engine
            .snapshot()
            .await
            .unwrap()
            .playback_context
            .active_kind,
        "none"
    );
}

#[tokio::test]
async fn video_progress_wrap_and_foreign_uri_never_advance_music() {
    let fixture = Fixture::new().await;
    fixture
        .engine
        .control("mode", Value::from("repeat-all"))
        .await
        .unwrap();
    fixture.started().await;
    fixture.renderer.lock().unwrap().position = 179;
    fixture.engine.poll().await;
    fixture.renderer.lock().unwrap().position = 0;
    fixture.engine.poll().await;
    let count = fixture
        .actions()
        .iter()
        .filter(|action| *action == "SetAVTransportURI")
        .count();
    assert_eq!(count, 1);
    fixture.renderer.lock().unwrap().uri = "http://other-controller/movie.mkv".into();
    fixture.engine.poll().await;
    assert_eq!(
        fixture.engine.video_state().await.session.unwrap().phase,
        "detached"
    );
    assert_eq!(
        fixture
            .actions()
            .iter()
            .filter(|action| *action == "SetAVTransportURI")
            .count(),
        1
    );
}

#[tokio::test]
async fn missing_probe_only_disables_analysis_and_feature_flag_blocks_all_video_entrypoints() {
    let fixture = Fixture::new().await;
    let state = fixture
        .engine
        .import_video(
            fixture.directory.path().join("film.mkv"),
            fixture.directory.path().join("missing-ffprobe"),
        )
        .await
        .unwrap();
    assert!(state.probe.is_some());
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if fixture
                .engine
                .video_state()
                .await
                .probe
                .as_ref()
                .is_some_and(|probe| probe.status == "failed")
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    fixture.engine.play(&fixture.track_id).await.unwrap();
    let disabled = Engine::new_with_video_enabled(fixture.directory.path().join("disabled"), false)
        .await
        .unwrap();
    assert!(!disabled.video_state().await.enabled);
    assert!(disabled
        .import_video(PathBuf::from("missing"), PathBuf::from("missing"))
        .await
        .is_err());
    assert!(disabled
        .prepare_video_play("missing", "missing")
        .await
        .is_err());
    disabled.shutdown();
}

#[tokio::test]
async fn a_late_video_status_read_cannot_restore_playing_after_pause() {
    let fixture = Fixture::new().await;
    let session = fixture.started().await.session.unwrap();
    let started = {
        let mut renderer = fixture.renderer.lock().unwrap();
        renderer.block_position = true;
        renderer.position_started.clone()
    };
    let engine = fixture.engine.clone();
    let poll = tokio::spawn(async move {
        engine.poll().await;
    });
    tokio::time::timeout(Duration::from_secs(2), started.notified())
        .await
        .unwrap();
    tokio::time::timeout(
        Duration::from_secs(2),
        fixture
            .engine
            .control_video(&session.id, session.generation, "pause", None),
    )
    .await
    .unwrap()
    .unwrap();
    fixture.renderer.lock().unwrap().release_position.cancel();
    poll.await.unwrap();
    assert_eq!(
        fixture.engine.video_state().await.session.unwrap().phase,
        "paused"
    );
}

#[tokio::test]
async fn concurrent_start_consumes_plan_once_and_old_session_cannot_stop_unknown_music() {
    let fixture = Fixture::new().await;
    let prepared = fixture.prepared().await;
    let id = prepared.plan.unwrap().id;
    let (first, second) = tokio::join!(
        fixture
            .engine
            .start_video_play(&id, prepared.owner_revision, false),
        fixture
            .engine
            .start_video_play(&id, prepared.owner_revision, false)
    );
    assert_ne!(first.is_ok(), second.is_ok());
    assert_eq!(
        fixture
            .actions()
            .iter()
            .filter(|action| *action == "SetAVTransportURI")
            .count(),
        1
    );
    let state = fixture.engine.video_state().await;
    let old = state.session.unwrap();
    fixture.renderer.lock().unwrap().fail = Some("Play".into());
    assert!(fixture
        .engine
        .start_music_play(
            &fixture.track_id,
            &fixture.device_id,
            state.owner_revision,
            true
        )
        .await
        .is_err());
    let before = fixture.actions();
    assert!(fixture
        .engine
        .control_video(&old.id, old.generation, "stop", None)
        .await
        .err()
        .unwrap()
        .contains("STALE_SESSION"));
    assert_eq!(fixture.actions(), before);
    fixture.engine.poll().await;
    assert_eq!(
        fixture
            .engine
            .snapshot()
            .await
            .unwrap()
            .playback_context
            .active_kind,
        "unknown"
    );
}

#[tokio::test]
async fn failed_video_stop_retains_owner_and_can_be_explicitly_retried() {
    let fixture = Fixture::new().await;
    let old = fixture.started().await.session.unwrap();
    fixture.renderer.lock().unwrap().fail = Some("Stop".into());
    let revision = fixture.engine.video_state().await.owner_revision;
    assert!(fixture
        .engine
        .start_music_play(&fixture.track_id, &fixture.device_id, revision, true)
        .await
        .is_err());
    let current = fixture.engine.video_state().await.session.unwrap();
    assert!(current.generation > old.generation);
    assert!(fixture.engine.play(&fixture.track_id).await.is_err());
    fixture.renderer.lock().unwrap().fail = None;
    fixture
        .engine
        .control_video(&current.id, current.generation, "stop", None)
        .await
        .unwrap();
    assert_eq!(
        fixture
            .engine
            .snapshot()
            .await
            .unwrap()
            .playback_context
            .active_kind,
        "none"
    );
}

#[tokio::test]
async fn music_file_removed_while_waiting_for_control_lock_does_not_stop_video() {
    let fixture = Fixture::new().await;
    let state = fixture.started().await;
    let before = fixture.actions();
    let guard = fixture.engine.actions.lock().await;
    let mut activity = fixture.engine.activity.subscribe();
    let engine = fixture.engine.clone();
    let track_id = fixture.track_id.clone();
    let device_id = fixture.device_id.clone();
    let request = tokio::spawn(async move {
        engine
            .start_music_play(&track_id, &device_id, state.owner_revision, true)
            .await
            .map(|_| ())
    });
    tokio::time::timeout(Duration::from_secs(2), activity.changed())
        .await
        .unwrap()
        .unwrap();
    std::fs::remove_file(fixture.directory.path().join("music.mp3")).unwrap();
    drop(guard);
    assert!(request.await.unwrap().is_err());
    assert_eq!(
        fixture.actions(),
        before,
        "failed preflight must not stop the active video"
    );
    assert_eq!(
        fixture
            .engine
            .snapshot()
            .await
            .unwrap()
            .playback_context
            .active_kind,
        "video"
    );
}

#[tokio::test]
async fn failed_legacy_device_switch_retains_uncertain_owner_when_video_is_enabled() {
    let fixture = Fixture::new().await;
    fixture.engine.play(&fixture.track_id).await.unwrap();
    let original = fixture.engine.snapshot().await.unwrap().devices[0].clone();
    let second_url = original
        .location
        .replace("/description.xml", "/other-description.xml");
    let devices = fixture
        .engine
        .add_device(&second_url)
        .await
        .unwrap()
        .devices;
    let other = devices
        .iter()
        .find(|device| device.id != fixture.device_id)
        .unwrap();
    fixture.renderer.lock().unwrap().fail = Some("Stop".into());
    assert!(fixture.engine.select(&other.id).await.is_err());
    let snapshot = fixture.engine.snapshot().await.unwrap();
    assert_eq!(
        snapshot.selected_device_id.as_deref(),
        Some(fixture.device_id.as_str())
    );
    assert_eq!(snapshot.playback_context.active_kind, "unknown");
    assert!(fixture.engine.control("resume", Value::Null).await.is_err());
    let prepared = fixture.prepared().await;
    let before = fixture.actions().len();
    assert!(fixture
        .engine
        .start_video_play(&prepared.plan.unwrap().id, prepared.owner_revision, true)
        .await
        .is_err());
    assert_eq!(&fixture.actions()[before..], &["Stop"]);
    fixture.renderer.lock().unwrap().fail = None;
    let prepared = fixture.prepared().await;
    fixture
        .engine
        .start_video_play(&prepared.plan.unwrap().id, prepared.owner_revision, true)
        .await
        .unwrap();
}

#[tokio::test]
async fn disabled_video_preserves_legacy_device_switch_warning_behavior() {
    let fixture = Fixture::with_video_enabled(false).await;
    fixture.engine.play(&fixture.track_id).await.unwrap();
    let original = fixture.engine.snapshot().await.unwrap().devices[0].clone();
    let second_url = original
        .location
        .replace("/description.xml", "/other-description.xml");
    let devices = fixture
        .engine
        .add_device(&second_url)
        .await
        .unwrap()
        .devices;
    let other = devices
        .iter()
        .find(|device| device.id != fixture.device_id)
        .unwrap();
    fixture.renderer.lock().unwrap().fail = Some("Stop".into());
    let snapshot = fixture.engine.select(&other.id).await.unwrap();
    assert_eq!(
        snapshot.selected_device_id.as_deref(),
        Some(other.id.as_str())
    );
    assert!(snapshot
        .warnings
        .iter()
        .any(|warning| warning.contains("原音箱未确认停止")));
    assert_eq!(snapshot.playback_context.active_kind, "none");
}

#[tokio::test]
async fn temporary_stopped_after_seek_or_buffering_keeps_video_resource_and_owner() {
    let fixture = Fixture::new().await;
    let session = fixture.started().await.session.unwrap();
    fixture.engine.poll().await;
    fixture
        .engine
        .control_video(&session.id, session.generation, "seek", Some(60_000))
        .await
        .unwrap();
    for _ in 0..2 {
        fixture.renderer.lock().unwrap().transport = "STOPPED".into();
        fixture.engine.poll().await;
        let state = fixture.engine.video_state().await;
        assert_eq!(state.owner_kind, "unknown");
        assert_eq!(state.session.unwrap().phase, "remote_state_unknown");
        assert!(fixture.engine.play(&fixture.track_id).await.is_err());
        let uri = fixture.renderer.lock().unwrap().uri.clone();
        assert_eq!(
            reqwest::get(uri)
                .await
                .unwrap()
                .bytes()
                .await
                .unwrap()
                .as_ref(),
            b"original video bytes: no encoding"
        );
        fixture.renderer.lock().unwrap().transport = "PLAYING".into();
        fixture.engine.poll().await;
        assert_eq!(
            fixture.engine.video_state().await.session.unwrap().phase,
            "playing"
        );
    }
    assert_eq!(
        fixture
            .actions()
            .iter()
            .filter(|action| *action == "Play")
            .count(),
        1
    );
    assert_eq!(
        fixture
            .actions()
            .iter()
            .filter(|action| *action == "SetAVTransportURI")
            .count(),
        1
    );
}

#[tokio::test]
async fn music_target_removed_while_waiting_for_control_lock_does_not_stop_video() {
    let fixture = Fixture::new().await;
    let state = fixture.started().await;
    let before = fixture.actions();
    let guard = fixture.engine.actions.lock().await;
    let mut activity = fixture.engine.activity.subscribe();
    let engine = fixture.engine.clone();
    let track_id = fixture.track_id.clone();
    let device_id = fixture.device_id.clone();
    let request = tokio::spawn(async move {
        engine
            .start_music_play(&track_id, &device_id, state.owner_revision, true)
            .await
            .map(|_| ())
    });
    tokio::time::timeout(Duration::from_secs(2), activity.changed())
        .await
        .unwrap()
        .unwrap();
    fixture
        .engine
        .player
        .lock()
        .await
        .devices
        .remove(&fixture.device_id);
    drop(guard);
    assert!(request.await.unwrap().is_err());
    assert_eq!(fixture.actions(), before);
    assert_eq!(fixture.engine.video_state().await.owner_kind, "video");
}

#[tokio::test]
async fn unavailable_music_route_does_not_stop_current_video() {
    let fixture = Fixture::new().await;
    let state = fixture.started().await;
    let before = fixture.actions();
    {
        let mut player = fixture.engine.player.lock().await;
        let device = &mut player.devices.get_mut(&fixture.device_id).unwrap().0;
        device.interface_ip = None;
        device.av_transport_url = "invalid-control-url".into();
    }
    assert!(fixture
        .engine
        .start_music_play(
            &fixture.track_id,
            &fixture.device_id,
            state.owner_revision,
            true
        )
        .await
        .is_err());
    assert_eq!(
        fixture.actions(),
        before,
        "local preflight must not send Stop"
    );
    assert_eq!(fixture.engine.video_state().await.owner_kind, "video");
    let uri = fixture.renderer.lock().unwrap().uri.clone();
    assert_eq!(reqwest::get(uri).await.unwrap().status(), StatusCode::OK);
}

#[tokio::test]
async fn invalid_video_metadata_does_not_stop_current_music() {
    let fixture = Fixture::new().await;
    fixture.engine.play(&fixture.track_id).await.unwrap();
    let mut source =
        crate::video::VideoSource::open_import(fixture.directory.path().join("film.mkv")).unwrap();
    source.info.title = "film\u{1}.mkv".into();
    let media_id = source.info.id.clone();
    fixture.engine.set_video_source_for_test(source).await;
    let prepared = fixture
        .engine
        .prepare_video_play(&media_id, &fixture.device_id)
        .await
        .unwrap();
    let before = fixture.actions();
    let error = fixture
        .engine
        .start_video_play(&prepared.plan.unwrap().id, prepared.owner_revision, true)
        .await
        .err()
        .unwrap();
    assert!(error.contains("控制字符"));
    assert_eq!(
        fixture.actions(),
        before,
        "invalid title must not interrupt music"
    );
    let state = fixture.engine.video_state().await;
    assert_eq!(state.owner_kind, "audio");
    assert!(state.session.is_none());
}
