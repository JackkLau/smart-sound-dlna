use super::*;
use axum::{
    extract::State,
    http::HeaderMap,
    routing::{get, post},
    Router,
};
use tokio::sync::Notify;

#[derive(Default)]
struct Renderer {
    block_position: bool,
    offline: bool,
    actions: Vec<String>,
    uri: String,
    transport: String,
    position: u64,
    position_unavailable: bool,
    zero_duration: bool,
}
struct Wire {
    state: Mutex<Renderer>,
    position_started: Notify,
    release_position: CancellationToken,
}
struct Fixture {
    _directory: tempfile::TempDir,
    engine: Arc<Engine>,
    wire: Arc<Wire>,
    server: tokio::task::JoinHandle<()>,
    track: String,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.engine.shutdown();
        self.server.abort();
    }
}
impl Fixture {
    async fn new() -> Self {
        async fn description() -> &'static str {
            r#"<root><device><deviceType>urn:schemas-upnp-org:device:MediaRenderer:1</deviceType><friendlyName>Connection test</friendlyName><UDN>uuid:connection-test</UDN><serviceList><service><serviceType>urn:schemas-upnp-org:service:AVTransport:1</serviceType><controlURL>/transport</controlURL></service></serviceList></device></root>"#
        }
        async fn soap(
            State(wire): State<Arc<Wire>>,
            headers: HeaderMap,
            body: String,
        ) -> (axum::http::StatusCode, String) {
            let header = headers["soapaction"].to_str().unwrap().trim_matches('"');
            let (service, action) = header.rsplit_once('#').unwrap();
            let (blocked, offline) = {
                let mut state = wire.state.lock().unwrap();
                state.actions.push(action.into());
                (
                    state.block_position && action == "GetPositionInfo",
                    state.offline,
                )
            };
            if offline {
                return (axum::http::StatusCode::SERVICE_UNAVAILABLE, String::new());
            }
            if blocked {
                wire.position_started.notify_one();
                wire.release_position.cancelled().await;
            }
            if action == "GetPositionInfo" && wire.state.lock().unwrap().position_unavailable {
                return (axum::http::StatusCode::INTERNAL_SERVER_ERROR, String::new());
            }
            let output = {
                let mut state = wire.state.lock().unwrap();
                match action {
                    "SetAVTransportURI" => {
                        let doc = roxmltree::Document::parse(&body).unwrap();
                        state.uri = doc.descendants().find(|n| n.has_tag_name("CurrentURI")).unwrap().text().unwrap().into();
                        state.position = 0;
                        String::new()
                    }
                    "Play" => { state.transport = "PLAYING".into(); String::new() }
                    "Stop" => { state.transport = "STOPPED".into(); String::new() }
                    "Pause" => { state.transport = "PAUSED_PLAYBACK".into(); String::new() }
                    "GetTransportInfo" => format!("<CurrentTransportState>{}</CurrentTransportState>", state.transport),
                    "GetPositionInfo" => format!("<RelTime>00:{:02}:{:02}</RelTime><TrackDuration>{}</TrackDuration><TrackURI>{}</TrackURI>", state.position / 60, state.position % 60, if state.zero_duration { "00:00:00" } else { "00:03:00" }, state.uri.replace('&', "&amp;")),
                    _ => String::new(),
                }
            };
            (axum::http::StatusCode::OK, format!("<s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\"><s:Body><u:{action}Response xmlns:u=\"{service}\">{output}</u:{action}Response></s:Body></s:Envelope>"))
        }
        let directory = tempfile::tempdir().unwrap();
        let engine = Engine::new(directory.path().join("data")).await.unwrap();
        let wire = Arc::new(Wire {
            state: Mutex::new(Renderer {
                transport: "STOPPED".into(),
                ..Renderer::default()
            }),
            position_started: Notify::new(),
            release_position: CancellationToken::new(),
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/description.xml", listener.local_addr().unwrap());
        let router = Router::new()
            .route("/description.xml", get(description))
            .route("/transport", post(soap))
            .with_state(wire.clone());
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let devices = engine.add_device(&url).await.unwrap().devices;
        engine.select(&devices[0].id).await.unwrap();
        let file = directory.path().join("test.mp3");
        std::fs::write(&file, b"ID3 fixture audio").unwrap();
        engine.import(vec![file]).await.unwrap();
        let track = engine.snapshot().await.unwrap().tracks[0].id.clone();
        Self {
            _directory: directory,
            engine,
            wire,
            server,
            track,
        }
    }
}

#[tokio::test]
async fn automatic_queue_survives_a_startup_stop_and_terminal_position_loss() {
    for terminal in ["STOPPED", "NO_MEDIA_PRESENT"] {
        let fixture = Fixture::new().await;
        let second_file = fixture._directory.path().join("second.mp3");
        std::fs::write(&second_file, b"second audio").unwrap();
        fixture.engine.import(vec![second_file]).await.unwrap();
        let tracks = fixture.engine.snapshot().await.unwrap().tracks;
        let second = tracks[1].id.clone();
        fixture.engine.play(&fixture.track).await.unwrap();
        // The speaker has accepted Play, but its first status still says STOPPED.
        fixture.wire.state.lock().unwrap().transport = "STOPPED".into();
        fixture.engine.poll().await;
        {
            let mut state = fixture.wire.state.lock().unwrap();
            state.transport = "PLAYING".into();
            state.position = 179;
        }
        fixture.engine.poll().await;
        {
            let mut state = fixture.wire.state.lock().unwrap();
            state.transport = terminal.into();
            state.position_unavailable = true;
        }
        fixture.engine.poll().await;
        assert_eq!(
            fixture
                .engine
                .snapshot()
                .await
                .unwrap()
                .current_track_id
                .as_deref(),
            Some(second.as_str()),
            "{terminal} must advance in library order"
        );
        fixture.wire.state.lock().unwrap().position_unavailable = false;
        fixture.engine.poll().await;
        fixture.wire.state.lock().unwrap().transport = "STOPPED".into();
        fixture.engine.poll().await;
        assert_eq!(
            fixture
                .wire
                .state
                .lock()
                .unwrap()
                .actions
                .iter()
                .filter(|a| *a == "Play")
                .count(),
            2,
            "an early stop on the next song must not skip it"
        );
    }
}

#[tokio::test]
async fn playback_does_not_wait_for_a_blocked_background_position_query() {
    let fixture = Fixture::new().await;
    fixture.wire.state.lock().unwrap().block_position = true;
    let engine = fixture.engine.clone();
    let poll = tokio::spawn(async move {
        engine.poll().await;
    });
    tokio::time::timeout(
        Duration::from_secs(2),
        fixture.wire.position_started.notified(),
    )
    .await
    .unwrap();
    let result = tokio::time::timeout(
        Duration::from_millis(500),
        fixture.engine.play(&fixture.track),
    )
    .await;
    fixture.wire.release_position.cancel();
    poll.await.unwrap();
    assert!(
        result.is_ok(),
        "playback waited for a background query instead of taking priority"
    );
    let snapshot = result.unwrap().unwrap();
    assert_eq!(
        snapshot.current_track_id.as_deref(),
        Some(fixture.track.as_str())
    );
    assert_eq!(snapshot.transport_state, "TRANSITIONING");
}

#[tokio::test]
async fn pause_preempts_poll_and_a_late_read_cannot_restore_playing() {
    let fixture = Fixture::new().await;
    fixture.engine.play(&fixture.track).await.unwrap();
    fixture.wire.state.lock().unwrap().block_position = true;
    let engine = fixture.engine.clone();
    let poll = tokio::spawn(async move { engine.poll().await });
    tokio::time::timeout(
        Duration::from_secs(2),
        fixture.wire.position_started.notified(),
    )
    .await
    .unwrap();
    let result = tokio::time::timeout(
        Duration::from_millis(500),
        fixture.engine.control("pause", Value::Null),
    )
    .await;
    fixture.wire.release_position.cancel();
    poll.await.unwrap();
    assert_eq!(
        result
            .expect("pause waited behind polling")
            .unwrap()
            .transport_state,
        "PAUSED_PLAYBACK"
    );
    assert_eq!(
        fixture.engine.snapshot().await.unwrap().transport_state,
        "PAUSED_PLAYBACK"
    );
    assert_eq!(
        fixture.wire.state.lock().unwrap().transport,
        "PAUSED_PLAYBACK"
    );
}

#[tokio::test]
async fn offline_device_is_visible_and_recovers_without_replaying_or_advancing() {
    let fixture = Fixture::new().await;
    fixture.engine.play(&fixture.track).await.unwrap();
    let selected = fixture.engine.snapshot().await.unwrap().selected_device_id;
    {
        let mut player = fixture.engine.player.lock().await;
        player.queue_mode = "repeat-one".into();
        player
            .completion
            .observe("PLAYING", 179, 180, Instant::now());
    }
    fixture.wire.state.lock().unwrap().offline = true;
    fixture.engine.poll().await;
    let offline = fixture.engine.snapshot().await.unwrap();
    assert_eq!(offline.device_connection, ConnectionState::Disconnected);
    assert_eq!(offline.transport_state, "UNAVAILABLE");
    assert_eq!(offline.selected_device_id, selected);
    assert_eq!(offline.devices.len(), 1);
    assert!(offline.status_error.is_some());
    let actions = fixture.wire.state.lock().unwrap().actions.clone();
    fixture.engine.poll().await;
    assert_eq!(
        fixture.wire.state.lock().unwrap().actions,
        actions,
        "backoff must suppress immediate retries"
    );

    // Simulate the retry deadline, then the same renderer becoming available.
    fixture.engine.player.lock().await.connection.next_probe = Some(Instant::now());
    {
        let mut wire = fixture.wire.state.lock().unwrap();
        wire.offline = false;
        wire.transport = "STOPPED".into();
    }
    fixture.engine.poll().await;
    let recovered = fixture.engine.snapshot().await.unwrap();
    assert_eq!(recovered.device_connection, ConnectionState::Connected);
    assert_eq!(recovered.selected_device_id, selected);
    assert_eq!(recovered.transport_state, "STOPPED");
    assert!(recovered.status_error.is_none());
    assert_eq!(
        recovered.current_track_id.as_deref(),
        Some(fixture.track.as_str())
    );
    let wire = fixture.wire.state.lock().unwrap();
    assert_eq!(wire.actions.iter().filter(|a| *a == "Play").count(), 1);
    assert_eq!(
        wire.actions
            .iter()
            .filter(|a| *a == "SetAVTransportURI")
            .count(),
        1
    );
}

#[tokio::test]
async fn reselecting_offline_device_retries_immediately_without_playback_commands() {
    let fixture = Fixture::new().await;
    fixture.wire.state.lock().unwrap().offline = true;
    fixture.engine.poll().await;
    let id = fixture
        .engine
        .snapshot()
        .await
        .unwrap()
        .selected_device_id
        .unwrap();
    fixture.engine.player.lock().await.connection.next_probe =
        Some(Instant::now() + Duration::from_secs(30));
    fixture.wire.state.lock().unwrap().offline = false;
    assert_eq!(
        fixture.engine.select(&id).await.unwrap().device_connection,
        ConnectionState::Checking
    );
    fixture.engine.poll().await;
    assert_eq!(
        fixture.engine.snapshot().await.unwrap().device_connection,
        ConnectionState::Connected
    );
    assert!(fixture
        .wire
        .state
        .lock()
        .unwrap()
        .actions
        .iter()
        .all(|action| action.starts_with("Get")));
}

#[test]
fn discovery_runs_only_while_disconnected_and_retries_never_give_up() {
    let now = Instant::now();
    let mut connection = Connection::default();
    assert!(!connection.scan_due(now));
    connection.retry_now();
    assert!(!connection.scan_due(now));
    connection.connected();
    assert!(!connection.scan_due(now + Duration::from_secs(3600)));
    for expected in [2, 4, 8, 16, 30, 30, 30] {
        connection.failed(now);
        assert_eq!(
            connection.next_probe,
            Some(now + Duration::from_secs(expected))
        );
    }
    assert!(connection.scan_due(now));
    assert!(!connection.scan_due(now + Duration::from_secs(29)));
    assert!(connection.scan_due(now + Duration::from_secs(30)));
    connection.connected();
    assert!(!connection.scan_due(now + Duration::from_secs(3600)));
    connection.failed(now + Duration::from_secs(3600));
    assert_eq!(connection.next_probe, Some(now + Duration::from_secs(3602)));
    assert!(connection.scan_due(now + Duration::from_secs(3600)));
}

#[tokio::test]
async fn slow_optional_status_does_not_mark_a_responsive_speaker_offline() {
    let fixture = Fixture::new().await;
    fixture.wire.state.lock().unwrap().block_position = true;
    tokio::time::timeout(Duration::from_millis(1800), fixture.engine.poll())
        .await
        .expect("optional status blocked confirmation of a healthy connection");
    fixture.wire.release_position.cancel();
    let snapshot = fixture.engine.snapshot().await.unwrap();
    assert_eq!(snapshot.device_connection, ConnectionState::Connected);
    assert_eq!(snapshot.transport_state, "STOPPED");
    assert!(snapshot.status_error.unwrap().contains("读取进度超时"));
}

#[tokio::test]
async fn brief_disconnect_preserves_queue_only_after_fresh_playing_evidence() {
    let fixture = Fixture::new().await;
    fixture.engine.play(&fixture.track).await.unwrap();
    fixture
        .engine
        .control("mode", Value::from("repeat-one"))
        .await
        .unwrap();
    fixture.engine.poll().await;
    fixture.wire.state.lock().unwrap().offline = true;
    fixture.engine.poll().await;
    fixture.wire.state.lock().unwrap().offline = false;
    fixture.engine.player.lock().await.connection.next_probe = None;
    fixture.engine.poll().await;
    assert_eq!(
        fixture
            .wire
            .state
            .lock()
            .unwrap()
            .actions
            .iter()
            .filter(|a| *a == "Play")
            .count(),
        1
    );
    // Playback continued on the speaker during the network interruption. Its
    // subsequent confirmed natural end must still advance the existing queue.
    fixture.wire.state.lock().unwrap().position = 179;
    fixture.engine.poll().await;
    fixture.wire.state.lock().unwrap().transport = "STOPPED".into();
    fixture.engine.poll().await;
    assert_eq!(
        fixture
            .wire
            .state
            .lock()
            .unwrap()
            .actions
            .iter()
            .filter(|a| *a == "Play")
            .count(),
        2
    );
}

#[tokio::test]
async fn wire_queue_uses_local_audio_duration_when_renderer_reports_zero() {
    let f = Fixture::new().await;
    let samples = 8000u32 * 180;
    let mut wav = b"RIFF".to_vec();
    wav.extend_from_slice(&(36 + samples).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&8000u32.to_le_bytes());
    wav.extend_from_slice(&8000u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&8u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&samples.to_le_bytes());
    wav.extend(vec![128; samples as usize]);
    let file = f._directory.path().join("actual-duration.wav");
    std::fs::write(&file, wav).unwrap();
    f.engine.import(vec![file]).await.unwrap();
    let tracks = f.engine.snapshot().await.unwrap().tracks;
    let song = &tracks[1].id;
    f.engine
        .control("mode", serde_json::json!("repeat-all"))
        .await
        .unwrap();
    f.engine.play(song).await.unwrap();
    assert_eq!(f.engine.snapshot().await.unwrap().duration, 180);
    {
        let mut wire = f.wire.state.lock().unwrap();
        wire.zero_duration = true;
        wire.position = 179;
    }
    f.engine.poll().await;
    f.wire.state.lock().unwrap().transport = "STOPPED".into();
    f.engine.poll().await;
    assert_eq!(
        f.engine
            .snapshot()
            .await
            .unwrap()
            .current_track_id
            .as_deref(),
        Some(f.track.as_str())
    );
    assert!(f
        .wire
        .state
        .lock()
        .unwrap()
        .actions
        .iter()
        .any(|a| a == "SetPlayMode"));
    f.engine.poll().await;
    let selected = f
        .engine
        .snapshot()
        .await
        .unwrap()
        .selected_device_id
        .unwrap();
    assert!(f.engine.remote_play("old-device", song).await.is_err());
    let all = f
        .engine
        .remote_control(&selected, "play-all")
        .await
        .unwrap();
    assert_eq!(all.queue_mode, "sequential");
    assert_eq!(all.current_track_id.as_deref(), Some(f.track.as_str()));
}
