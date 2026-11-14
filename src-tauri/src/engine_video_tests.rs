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
    position_text: Option<String>,
    reported_uri: Option<String>,
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
        async fn multiple_description() -> String {
            let other = description()
                .await
                .replace("uuid:video-test", "uuid:aaa-sibling")
                .replace("Video test", "AAA sibling");
            let other = other
                .strip_prefix("<root>")
                .unwrap()
                .strip_suffix("</root>")
                .unwrap();
            description().await.replace(
                "</device></root>",
                &format!("<deviceList>{other}</deviceList></device></root>"),
            )
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
                    "GetPositionInfo" => format!("<RelTime>{}</RelTime><TrackDuration>00:03:00</TrackDuration><TrackURI>{}</TrackURI>", renderer.position_text.clone().unwrap_or_else(|| format!("00:{:02}:{:02}", renderer.position / 60, renderer.position % 60)), renderer.reported_uri.as_ref().unwrap_or(&renderer.uri).replace('&', "&amp;")),
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
            .route("/multi-description.xml", get(multiple_description))
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
    fixture.engine.set_video_source_for_test(source).await;
    let media_id = fixture.engine.video_state().await.entry.unwrap().id;
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

#[tokio::test]
async fn session_diagnostics_report_real_transfer_without_exposing_the_grant() {
    let fixture = Fixture::new().await;
    let state = fixture.started().await;
    let encoded = serde_json::to_value(state.session.unwrap()).unwrap();
    assert!(
        encoded.get("diagnostics").is_some(),
        "video sessions need transfer and command diagnostics"
    );
    assert_eq!(encoded["diagnostics"]["transfer"]["get_requests"], 1);
    assert_eq!(encoded["diagnostics"]["transfer"]["bytes_streamed"], "33");
    assert_eq!(encoded["diagnostics"]["last_command"]["action"], "Play");
    assert_eq!(
        encoded["diagnostics"]["last_command"]["outcome"],
        "accepted"
    );
}

#[tokio::test]
async fn explicit_recheck_recovers_without_replaying_and_background_failures_back_off() {
    let fixture = Fixture::new().await;
    let session = fixture.started().await.session.unwrap();
    fixture.renderer.lock().unwrap().fail = Some("GetTransportInfo".into());
    fixture.engine.poll().await;
    let failed = fixture.engine.video_state().await;
    assert_eq!(failed.owner_kind, "unknown");
    assert_eq!(
        failed
            .session
            .unwrap()
            .diagnostics
            .consecutive_poll_failures,
        1
    );
    let before = fixture.actions();
    fixture.engine.poll().await;
    assert_eq!(
        fixture.actions(),
        before,
        "failed status polls must respect backoff"
    );
    fixture.renderer.lock().unwrap().fail = None;
    let checked = fixture
        .engine
        .recheck_video_session(&session.id, session.generation)
        .await
        .unwrap();
    assert_eq!(checked.owner_kind, "video");
    let view = checked.session.unwrap();
    assert_eq!(view.phase, "playing");
    assert_eq!(view.diagnostics.state_source, "poll");
    assert_eq!(view.diagnostics.consecutive_poll_failures, 0);
    assert!(view.diagnostics.last_poll_age_ms.is_some());
    assert!(fixture.actions()[before.len()..]
        .iter()
        .all(|action| action.starts_with("Get")));
    fixture
        .engine
        .start_music_play(
            &fixture.track_id,
            &fixture.device_id,
            checked.owner_revision,
            true,
        )
        .await
        .unwrap();
    let before = fixture.actions();
    assert!(fixture
        .engine
        .recheck_video_session(&session.id, session.generation)
        .await
        .is_err());
    assert_eq!(
        fixture.actions(),
        before,
        "a stale recheck must not inspect or control music"
    );
}

#[tokio::test]
async fn rebooted_renderer_is_only_restarted_after_explicit_video_start() {
    let fixture = Fixture::new().await;
    let session = fixture.started().await.session.unwrap();
    let rebooted = Fixture::new().await;
    let location = rebooted.engine.snapshot().await.unwrap().devices[0]
        .location
        .clone();
    fixture.renderer.lock().unwrap().transport = "STOPPED".into();
    fixture.engine.add_device(&location).await.unwrap();
    let before = rebooted.actions();
    let checked = fixture
        .engine
        .recheck_video_session(&session.id, session.generation)
        .await
        .unwrap();
    assert_eq!(checked.owner_kind, "unknown");
    assert!(rebooted.actions()[before.len()..]
        .iter()
        .all(|action| action.starts_with("Get")));
    assert!(!rebooted.actions().iter().any(|action| action == "Play"));
    let replayed = fixture.started().await;
    assert!(replayed.session.unwrap().command_accepted);
    assert_eq!(
        rebooted.renderer.lock().unwrap().received,
        b"original video bytes: no encoding"
    );
    assert_eq!(
        rebooted
            .actions()
            .iter()
            .filter(|action| *action == "Play")
            .count(),
        1
    );
}

#[tokio::test]
async fn changed_device_identity_cannot_receive_commands_during_recheck() {
    let fixture = Fixture::new().await;
    let session = fixture.started().await.session.unwrap();
    {
        let mut player = fixture.engine.player.lock().await;
        let device = &mut player.devices.get_mut(&fixture.device_id).unwrap().0;
        device.location = device
            .location
            .replace("/description.xml", "/other-description.xml");
    }
    let before = fixture.actions();
    assert!(fixture
        .engine
        .recheck_video_session(&session.id, session.generation)
        .await
        .err()
        .unwrap()
        .contains("身份"));
    assert_eq!(fixture.actions(), before);
    let detached = fixture.engine.video_state().await;
    assert_eq!(detached.owner_kind, "none");
    assert_eq!(detached.session.unwrap().phase, "detached");
    assert!(fixture
        .engine
        .control_video(&session.id, session.generation, "stop", None)
        .await
        .is_err());
    assert_eq!(
        fixture.actions(),
        before,
        "do not stop a different renderer at the old address"
    );
    let uri = fixture.renderer.lock().unwrap().uri.clone();
    assert_eq!(
        reqwest::get(uri).await.unwrap().status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn diagnostic_export_retains_stopped_evidence_and_rejects_replaced_session() {
    let fixture = Fixture::new().await;
    let session = fixture.started().await.session.unwrap();
    let uri = fixture.renderer.lock().unwrap().uri.clone();
    fixture.renderer.lock().unwrap().fail = Some("Pause".into());
    assert!(fixture
        .engine
        .control_video(&session.id, session.generation, "pause", None)
        .await
        .is_err());
    let report = fixture
        .engine
        .video_diagnostic_report(&session.id, session.generation)
        .await
        .unwrap();
    assert_eq!(report["diagnostics"]["last_command"]["action"], "Pause");
    assert_eq!(report["diagnostics"]["last_command"]["outcome"], "failed");
    let encoded = report.to_string();
    for secret in [
        uri.as_str(),
        "film.mkv",
        fixture.device_id.as_str(),
        fixture.video_id.as_str(),
        session.id.as_str(),
        fixture.directory.path().to_str().unwrap(),
        "injected renderer failure",
    ] {
        assert!(
            !encoded.contains(secret),
            "diagnostic export must omit sensitive/freeform text"
        );
    }
    fixture.renderer.lock().unwrap().fail = None;
    fixture
        .engine
        .control_video(&session.id, session.generation, "stop", None)
        .await
        .unwrap();
    let stopped = fixture
        .engine
        .video_diagnostic_report(&session.id, session.generation)
        .await
        .unwrap();
    assert_eq!(stopped["diagnostics"]["transfer"]["revoked"], true);
    assert_eq!(stopped["diagnostics"]["transfer"]["bytes_streamed"], "33");
    fixture.started().await;
    assert!(fixture
        .engine
        .video_diagnostic_report(&session.id, session.generation)
        .await
        .is_err());
}

#[tokio::test]
async fn recheck_selects_the_current_udn_from_a_multi_renderer_description() {
    let fixture = Fixture::new().await;
    let session = fixture.started().await.session.unwrap();
    {
        let mut player = fixture.engine.player.lock().await;
        let device = &mut player.devices.get_mut(&fixture.device_id).unwrap().0;
        device.location = device
            .location
            .replace("/description.xml", "/multi-description.xml");
    }
    let before = fixture.actions();
    let checked = fixture
        .engine
        .recheck_video_session(&session.id, session.generation)
        .await
        .unwrap();
    assert_eq!(checked.owner_kind, "video");
    assert_eq!(checked.session.unwrap().phase, "playing");
    assert!(fixture.actions()[before.len()..]
        .iter()
        .all(|action| action.starts_with("Get")));
    let uri = fixture.renderer.lock().unwrap().uri.clone();
    assert_eq!(reqwest::get(uri).await.unwrap().status(), StatusCode::OK);
}

#[tokio::test]
async fn buffering_status_is_visible_without_replaying_the_video() {
    let fixture = Fixture::new().await;
    fixture.started().await;
    fixture.engine.poll().await;
    assert_eq!(
        fixture.engine.video_state().await.session.unwrap().phase,
        "playing"
    );
    fixture.renderer.lock().unwrap().transport = "TRANSITIONING".into();
    fixture.engine.poll().await;
    assert_eq!(
        fixture.engine.video_state().await.session.unwrap().phase,
        "awaiting_evidence"
    );
    fixture.renderer.lock().unwrap().transport = "PLAYING".into();
    fixture.engine.poll().await;
    assert_eq!(
        fixture.engine.video_state().await.session.unwrap().phase,
        "playing"
    );
    assert_eq!(
        fixture
            .actions()
            .iter()
            .filter(|action| *action == "Play")
            .count(),
        1
    );
}

#[tokio::test]
async fn history_saves_only_confirmed_positions_throttles_writes_and_survives_restart() {
    let fixture = Fixture::new().await;
    fixture
        .engine
        .control("mode", Value::from("repeat-all"))
        .await
        .unwrap();
    let data = fixture.directory.path().join("data");
    let music_before = std::fs::read(data.join("library.json")).unwrap();
    let session = fixture.started().await.session.unwrap();
    assert!(
        fixture.engine.video_state().await.recent_videos[0]
            .progress
            .is_none(),
        "HTTP delivery is not position evidence"
    );
    fixture.renderer.lock().unwrap().position = 45;
    fixture.engine.poll().await;
    let initial = std::fs::read(data.join("video-library.json")).unwrap();
    assert_eq!(
        fixture.engine.video_state().await.recent_videos[0]
            .progress
            .as_ref()
            .unwrap()
            .position_ms,
        45_000
    );
    fixture.renderer.lock().unwrap().position = 55;
    fixture.engine.poll().await;
    assert_eq!(
        std::fs::read(data.join("video-library.json")).unwrap(),
        initial,
        "ordinary polling must not write every time"
    );
    fixture
        .engine
        .control_video(&session.id, session.generation, "seek", Some(90_000))
        .await
        .unwrap();
    fixture.renderer.lock().unwrap().position_text = Some("NOT_IMPLEMENTED".into());
    fixture.engine.poll().await;
    fixture
        .engine
        .control_video(&session.id, session.generation, "pause", None)
        .await
        .unwrap();
    assert_eq!(
        fixture.engine.video_state().await.recent_videos[0]
            .progress
            .as_ref()
            .unwrap()
            .position_ms,
        55_000,
        "pause flushes the last confirmed sample, never a requested Seek target"
    );
    fixture
        .engine
        .control_video(&session.id, session.generation, "stop", None)
        .await
        .unwrap();
    assert_eq!(
        std::fs::read(data.join("library.json")).unwrap(),
        music_before
    );
    fixture.engine.shutdown();
    let restored = Engine::new_with_video_enabled(data, true).await.unwrap();
    let video = restored.video_state().await;
    assert_eq!(video.recent_videos.len(), 1);
    assert_eq!(video.recent_videos[0].id, fixture.video_id);
    assert_eq!(
        video.recent_videos[0]
            .progress
            .as_ref()
            .unwrap()
            .position_ms,
        55_000
    );
    assert!(video.entry.is_none() && video.session.is_none() && video.plan.is_none());
    let audio = restored.snapshot().await.unwrap();
    assert_eq!(audio.tracks[0].id, fixture.track_id);
    // Queue mode has always been runtime-only; video history must not migrate music settings.
    assert_eq!(audio.queue_mode, "sequential");
    assert!(audio.current_track_id.is_none());
    restored.shutdown();
}

#[tokio::test]
async fn history_ignores_missing_uri_invalid_times_and_temporary_stopped() {
    let fixture = Fixture::new().await;
    let session = fixture.started().await.session.unwrap();
    {
        let mut renderer = fixture.renderer.lock().unwrap();
        renderer.position = 25;
        renderer.reported_uri = Some(String::new());
    }
    fixture.engine.poll().await;
    assert!(fixture.engine.video_state().await.recent_videos[0]
        .progress
        .is_none());
    {
        let mut renderer = fixture.renderer.lock().unwrap();
        renderer.reported_uri = None;
        renderer.position_text = Some("NOT_IMPLEMENTED".into());
    }
    fixture.engine.poll().await;
    assert!(fixture.engine.video_state().await.recent_videos[0]
        .progress
        .is_none());
    {
        let mut renderer = fixture.renderer.lock().unwrap();
        renderer.position_text = None;
        renderer.position = 181;
    }
    fixture.engine.poll().await;
    assert!(fixture.engine.video_state().await.recent_videos[0]
        .progress
        .is_none());
    fixture.renderer.lock().unwrap().position = 35;
    fixture.engine.poll().await;
    {
        let mut renderer = fixture.renderer.lock().unwrap();
        renderer.position = 0;
        renderer.transport = "STOPPED".into();
    }
    fixture.engine.poll().await;
    fixture
        .engine
        .control_video(&session.id, session.generation, "stop", None)
        .await
        .unwrap();
    assert_eq!(
        fixture.engine.video_state().await.recent_videos[0]
            .progress
            .as_ref()
            .unwrap()
            .position_ms,
        35_000
    );
}

#[tokio::test]
async fn forgetting_active_history_preserves_file_grant_and_music_handoff() {
    let fixture = Fixture::new().await;
    fixture
        .engine
        .control("mode", Value::from("repeat-all"))
        .await
        .unwrap();
    fixture.started().await;
    fixture.renderer.lock().unwrap().position = 40;
    fixture.engine.poll().await;
    let actions = fixture.actions();
    let uri = fixture.renderer.lock().unwrap().uri.clone();
    let state = fixture
        .engine
        .forget_recent_video(&fixture.video_id)
        .await
        .unwrap();
    assert!(state.recent_videos.is_empty());
    assert_eq!(state.session.unwrap().phase, "playing");
    assert_eq!(fixture.actions(), actions);
    assert_eq!(
        reqwest::get(&uri)
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap()
            .as_ref(),
        b"original video bytes: no encoding"
    );
    fixture.renderer.lock().unwrap().position = 50;
    fixture.engine.poll().await;
    let revision = fixture.engine.video_state().await.owner_revision;
    fixture
        .engine
        .start_music_play(&fixture.track_id, &fixture.device_id, revision, true)
        .await
        .unwrap();
    assert!(fixture.engine.video_state().await.recent_videos.is_empty());
    let music = fixture.engine.snapshot().await.unwrap();
    assert_eq!(
        music.current_track_id.as_deref(),
        Some(fixture.track_id.as_str())
    );
    assert_eq!(music.queue_mode, "repeat-all");
    assert!(fixture.directory.path().join("film.mkv").is_file());
}

#[tokio::test]
async fn recent_selection_rejects_changed_sources_before_probe_or_playback() {
    let fixture = Fixture::new().await;
    fixture.engine.play(&fixture.track_id).await.unwrap();
    std::fs::write(fixture.directory.path().join("film.mkv"), b"changed").unwrap();
    let actions = fixture.actions();
    let error = fixture
        .engine
        .select_recent_video(&fixture.video_id, PathBuf::from("/unavailable/ffprobe"))
        .await
        .err()
        .unwrap();
    assert!(error.contains("SOURCE_CHANGED"));
    assert_eq!(fixture.actions(), actions);
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
async fn damaged_history_does_not_block_music_or_direct_video() {
    let mut fixture = Fixture::new().await;
    let data = fixture.directory.path().join("data");
    std::fs::write(data.join("video-library.json"), b"broken index").unwrap();
    let target = fixture.engine.player.lock().await.devices[&fixture.device_id]
        .0
        .clone();
    fixture.engine.shutdown();
    fixture.engine = Engine::new_with_video_enabled(data.clone(), true)
        .await
        .unwrap();
    assert!(fixture.engine.video_state().await.catalog_warning.is_some());
    fixture.engine.add_device(&target.location).await.unwrap();
    fixture.engine.select(&fixture.device_id).await.unwrap();
    let source =
        crate::video::VideoSource::open_import(fixture.directory.path().join("film.mkv")).unwrap();
    fixture.video_id = source.info.id.clone();
    fixture.engine.set_video_source_for_test(source).await;
    fixture.engine.play(&fixture.track_id).await.unwrap();
    let state = fixture.started().await;
    assert!(state.session.unwrap().http_requested);
    assert!(state.catalog_warning.is_some());
    assert_eq!(
        std::fs::read(data.join("video-library.json")).unwrap(),
        b"broken index"
    );
    fixture
        .engine
        .start_music_play(
            &fixture.track_id,
            &fixture.device_id,
            state.owner_revision,
            true,
        )
        .await
        .unwrap();
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
async fn disabled_video_never_loads_or_creates_history() {
    let fixture = Fixture::with_video_enabled(false).await;
    let path = fixture.directory.path().join("data/video-library.json");
    assert!(!path.exists());
    std::fs::write(&path, b"disabled feature sentinel").unwrap();
    let engine = Engine::new_with_video_enabled(fixture.directory.path().join("data"), false)
        .await
        .unwrap();
    let video = engine.video_state().await;
    assert!(video.catalog_warning.is_none() && video.recent_videos.is_empty());
    assert!(engine.forget_recent_video(&fixture.video_id).await.is_err());
    assert_eq!(std::fs::read(path).unwrap(), b"disabled feature sentinel");
    engine.shutdown();
}

#[tokio::test]
#[ignore = "requires the hash-verified host FFprobe; run npm run prepare:video first"]
async fn packaged_ffprobe_restores_recent_video_through_real_engine_without_auto_play() {
    let target = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("windows", "x86_64") => "x86_64-pc-windows-msvc",
        _ => panic!("requires a supported video-preview platform"),
    };
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let tool = root.join("binaries").join(format!(
        "ffprobe-{target}{}",
        if cfg!(windows) { ".exe" } else { "" }
    ));
    assert!(tool.is_file(), "run npm run prepare:video first");
    let directory = tempfile::tempdir().unwrap();
    let data = directory.path().join("data");
    let movie = directory.path().join("真实样片.mkv");
    std::fs::copy(root.join("tests/fixtures/generated-video.mkv"), &movie).unwrap();
    async fn ready(engine: &Arc<Engine>) -> VideoSnapshot {
        tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                let state = engine.video_state().await;
                match state.probe.as_ref().map(|probe| probe.status.as_str()) {
                    Some("ready") => return state,
                    Some("failed") => panic!("probe failed: {:?}", state.error),
                    _ => tokio::time::sleep(Duration::from_millis(20)).await,
                }
            }
        })
        .await
        .unwrap()
    }
    let first = Engine::new_with_video_enabled(data.clone(), true)
        .await
        .unwrap();
    first.import_video(movie, tool.clone()).await.unwrap();
    let imported = ready(&first).await;
    let id = imported.entry.unwrap().id;
    assert_eq!(imported.recent_videos[0].id, id);
    first.shutdown();
    let restored = Engine::new_with_video_enabled(data.clone(), true)
        .await
        .unwrap();
    assert!(restored.video_state().await.entry.is_none());
    restored
        .select_recent_video(&id, tool.clone())
        .await
        .unwrap();
    let selected = ready(&restored).await;
    let info = selected.entry.unwrap();
    assert_eq!(info.id, id);
    assert_eq!(info.video_codec.as_deref(), Some("png"));
    assert_eq!(info.duration_ms, Some(2000));
    assert_eq!(selected.recent_videos.len(), 1);
    assert!(selected.session.is_none() && selected.plan.is_none());
    assert!(restored
        .snapshot()
        .await
        .unwrap()
        .current_track_id
        .is_none());
    restored.shutdown();
}

#[tokio::test]
async fn explicit_stop_and_music_handoff_flush_latest_confirmed_video_sample() {
    for action in ["stop", "music"] {
        let fixture = Fixture::new().await;
        let session = fixture.started().await.session.unwrap();
        fixture.renderer.lock().unwrap().position = 20;
        fixture.engine.poll().await;
        fixture.renderer.lock().unwrap().position = 25;
        fixture.engine.poll().await;
        assert_eq!(
            fixture.engine.video_state().await.recent_videos[0]
                .progress
                .as_ref()
                .unwrap()
                .position_ms,
            20_000
        );
        if action == "stop" {
            fixture
                .engine
                .control_video(&session.id, session.generation, "stop", None)
                .await
                .unwrap();
        } else {
            fixture
                .engine
                .start_music_play(
                    &fixture.track_id,
                    &fixture.device_id,
                    session.generation,
                    true,
                )
                .await
                .unwrap();
        }
        assert_eq!(
            fixture.engine.video_state().await.recent_videos[0]
                .progress
                .as_ref()
                .unwrap()
                .position_ms,
            25_000
        );
    }
}

#[cfg(feature = "desktop")]
#[tokio::test]
#[ignore = "requires the hash-verified native video tools"]
async fn packaged_subtitles_reach_renderer_and_keep_music_until_handoff() {
    use crate::{
        video_subtitles::{SubtitleMode, SubtitleSelection},
        video_tools,
    };
    let fixture = Fixture::new().await;
    let probe = video_tools::ffprobe_path().unwrap();
    let encoder = video_tools::ffmpeg_path().unwrap();
    let samples = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    fixture.engine.play(&fixture.track_id).await.unwrap();
    let mut expected_owner = "audio";
    for (mode, index) in [
        (SubtitleMode::Remux, 3),
        (SubtitleMode::Burn, 3),
        (SubtitleMode::Burn, 2),
        #[cfg(not(all(target_os = "macos", target_arch = "x86_64")))]
        (SubtitleMode::BurnHdr10, 3),
    ] {
        let source = samples.join(if mode == SubtitleMode::BurnHdr10 {
            "generated-subtitles-hdr10.mkv"
        } else {
            "generated-subtitles.mkv"
        });
        let movie =
            crate::video::probe_file(source.clone(), probe.clone(), CancellationToken::new())
                .await
                .unwrap();
        fixture.engine.set_video_source_for_test(movie).await;
        let id = fixture.engine.video_state().await.entry.unwrap().id;
        let choice = SubtitleSelection { mode, index };
        let before = fixture.actions();
        fixture
            .engine
            .prepare_video_variant(
                &id,
                &fixture.device_id,
                Some((choice, (encoder.clone(), probe.clone()))),
            )
            .await
            .unwrap();
        let ready = tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                let state = fixture.engine.video_state().await;
                assert_ne!(
                    state.probe.as_ref().unwrap().status,
                    "failed",
                    "{:?}",
                    state.probe.as_ref().unwrap().error
                );
                if state.plan.is_some() {
                    break state;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(fixture.actions(), before);
        assert_eq!(ready.owner_kind, expected_owner);
        let plan = ready.plan.unwrap();
        assert_eq!(plan.subtitle, Some(choice));
        let state = fixture
            .engine
            .start_video_play(&plan.id, ready.owner_revision, true)
            .await
            .unwrap();
        let view = state.session.unwrap();
        assert_eq!(view.subtitle, Some(choice));
        assert!(view.http_requested);
        let copy = fixture.directory.path().join("received.mkv");
        std::fs::write(&copy, &fixture.renderer.lock().unwrap().received).unwrap();
        let info = crate::video::probe_file(copy.clone(), probe.clone(), CancellationToken::new())
            .await
            .unwrap()
            .info;
        assert_eq!(info.audio_tracks.len(), 1);
        if mode == SubtitleMode::Remux {
            assert_eq!(info.width, Some(320));
            assert_eq!(info.subtitle_tracks.len(), 1);
            assert_eq!(
                info.subtitle_tracks[0].codec.as_deref(),
                Some("hdmv_pgs_subtitle")
            );
            for selector in ["v", "a"] {
                let hash = |path: &std::path::Path| {
                    let out = std::process::Command::new(&encoder)
                        .args(["-loglevel", "error", "-i"])
                        .arg(path)
                        .args([
                            "-map",
                            &format!("0:{selector}"),
                            "-c",
                            "copy",
                            "-f",
                            "hash",
                            "-",
                        ])
                        .output()
                        .unwrap();
                    assert!(out.status.success());
                    out.stdout
                };
                assert_eq!(hash(&source), hash(&copy));
            }
            let flags = std::process::Command::new(&probe)
                .args([
                    "-v",
                    "error",
                    "-select_streams",
                    "s",
                    "-show_entries",
                    "stream_disposition=default,forced",
                    "-of",
                    "json",
                ])
                .arg(&copy)
                .output()
                .unwrap();
            let json: Value = serde_json::from_slice(&flags.stdout).unwrap();
            for flag in ["default", "forced"] {
                assert_eq!(json["streams"][0]["disposition"][flag], 1);
            }
        } else {
            if mode == SubtitleMode::BurnHdr10 {
                assert_eq!((info.width, info.height), (Some(320), Some(180)));
                assert_eq!(info.color_transfer.as_deref(), Some("smpte2084"));
                assert_eq!(info.video_codec.as_deref(), Some("hevc"));
                assert_eq!(info.pixel_format.as_deref(), Some("yuv420p10le"));
                assert!(info.can_burn_hdr10);
                let metadata = |file: &std::path::Path| {
                    let out = std::process::Command::new(&probe)
                        .args([
                            "-v",
                            "error",
                            "-select_streams",
                            "V:0",
                            "-read_intervals",
                            "%+#8",
                            "-show_frames",
                            "-of",
                            "json",
                        ])
                        .arg(file)
                        .output()
                        .unwrap();
                    assert!(out.status.success());
                    let json: Value = serde_json::from_slice(&out.stdout).unwrap();
                    json["frames"][0]["side_data_list"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .filter(|value| {
                            matches!(
                                value["side_data_type"].as_str(),
                                Some("Mastering display metadata" | "Content light level metadata")
                            )
                        })
                        .cloned()
                        .collect::<Vec<_>>()
                };
                let original = metadata(&source);
                assert_eq!(original.len(), 2);
                assert_eq!(metadata(&copy), original);
            } else {
                assert_eq!(info.height, Some(1080));
            }
            assert!(info.subtitle_tracks.is_empty());
            // The source is black. The PGS rectangle and text each create bright pixels only while their cue is visible.
            let pixels = |time: &str| {
                let out = std::process::Command::new(&encoder)
                    .args(["-loglevel", "error", "-ss", time, "-i"])
                    .arg(&copy)
                    .args([
                        "-map",
                        "0:v:0",
                        "-frames:v",
                        "1",
                        "-vf",
                        "scale=320:180",
                        "-pix_fmt",
                        "gray",
                        "-f",
                        "rawvideo",
                        "-",
                    ])
                    .output()
                    .unwrap();
                assert!(out.status.success());
                assert_eq!(
                    out.stdout.len(),
                    320 * 180,
                    "missing frame {time} for {choice:?}"
                );
                out.stdout.into_iter().filter(|&v| v > 100).count()
            };
            assert!(pixels("0.5") > 10);
            assert_eq!(pixels("1.0"), 0);
        }
        let uri = fixture.renderer.lock().unwrap().uri.clone();
        fixture
            .engine
            .control_video(&view.id, view.generation, "stop", None)
            .await
            .unwrap();
        assert_eq!(
            reqwest::get(uri).await.unwrap().status(),
            StatusCode::NOT_FOUND
        );
        expected_owner = "none";
    }
    assert_eq!(
        std::fs::read_dir(&fixture.engine.subtitle_work_dir)
            .unwrap()
            .count(),
        1,
        "only the directory lock can remain"
    );
}

#[cfg(all(unix, feature = "desktop"))]
#[tokio::test]
async fn cancelling_subtitle_preparation_revokes_partial_file_and_keeps_music() {
    use crate::{
        video::VideoTrackInfo,
        video_subtitles::{SubtitleMode, SubtitleSelection},
    };
    use std::os::unix::fs::PermissionsExt;
    let fixture = Fixture::new().await;
    let mut movie =
        crate::video::VideoSource::open_import(fixture.directory.path().join("film.mkv")).unwrap();
    movie.info.subtitle_tracks.push(VideoTrackInfo {
        index: 8,
        codec: Some("hdmv_pgs_subtitle".into()),
        language: Some("chi".into()),
        title: None,
        channels: None,
    });
    fixture.engine.set_video_source_for_test(movie).await;
    let id = fixture.engine.video_state().await.entry.unwrap().id;
    fixture.engine.play(&fixture.track_id).await.unwrap();
    let before = fixture.actions();
    let tool = fixture.directory.path().join("slow-ffmpeg");
    std::fs::write(
        &tool,
        b"#!/bin/sh\nprintf partial > movie.mkv\nexec /bin/sleep 30\n",
    )
    .unwrap();
    std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o700)).unwrap();
    let state = fixture
        .engine
        .prepare_video_variant(
            &id,
            &fixture.device_id,
            Some((
                SubtitleSelection {
                    index: 8,
                    mode: SubtitleMode::Remux,
                },
                (tool.clone(), tool),
            )),
        )
        .await
        .unwrap();
    let root = &fixture.engine.subtitle_work_dir;
    tokio::time::timeout(Duration::from_secs(5), async {
        while !std::fs::read_dir(root)
            .into_iter()
            .flatten()
            .flatten()
            .any(|e| e.path().join("movie.mkv").is_file())
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let cancelled = fixture
        .engine
        .cancel_video_probe(&state.probe.unwrap().task_id)
        .await
        .unwrap();
    assert_eq!(cancelled.probe.unwrap().status, "cancelled");
    tokio::time::timeout(Duration::from_secs(5), async {
        while std::fs::read_dir(root).unwrap().count() != 1 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let final_state = fixture.engine.video_state().await;
    assert!(final_state.plan.is_none());
    assert_eq!(final_state.owner_kind, "audio");
    assert_eq!(before, fixture.actions());
}
