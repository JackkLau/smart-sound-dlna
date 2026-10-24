//! End-to-end core tests using an actual HTTP UPnP renderer, without LAN discovery.

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Router,
};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio_util::sync::CancellationToken;
use xiaomi_dlna_desktop::engine::Engine;

#[derive(Default)]
struct RendererState {
    id: String,
    actions: Vec<(String, HashMap<String, String>)>,
    uri: String,
    transport: String,
    position: String,
    volume: u8,
    fail_action: Option<String>,
}

const DESCRIPTION: &str = r#"<?xml version="1.0"?>
<root xmlns="urn:schemas-upnp-org:device-1-0">
<specVersion><major>1</major><minor>0</minor></specVersion>
<device><deviceType>urn:schemas-upnp-org:device:MediaRenderer:1</deviceType>
<friendlyName>测试小爱音箱 Pro</friendlyName><manufacturer>Xiaomi</manufacturer>
<modelName>小爱音箱 Pro</modelName><UDN>uuid:integration-speaker</UDN>
<serviceList>
<service><serviceType>urn:schemas-upnp-org:service:AVTransport:1</serviceType>
<serviceId>urn:upnp-org:serviceId:AVTransport</serviceId><controlURL>/transport</controlURL>
<eventSubURL>/events</eventSubURL><SCPDURL>/avt.xml</SCPDURL></service>
<service><serviceType>urn:schemas-upnp-org:service:RenderingControl:1</serviceType>
<serviceId>urn:upnp-org:serviceId:RenderingControl</serviceId><controlURL>/rendering</controlURL>
<eventSubURL>/events</eventSubURL><SCPDURL>/rc.xml</SCPDURL></service>
</serviceList></device></root>"#;

async fn description(State(state): State<Arc<Mutex<RendererState>>>) -> impl IntoResponse {
    let id = state.lock().unwrap().id.clone();
    (
        [("content-type", "text/xml; charset=utf-8")],
        DESCRIPTION.replace("uuid:integration-speaker", &id),
    )
}

fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

async fn soap(
    State(state): State<Arc<Mutex<RendererState>>>,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    let action_header = headers
        .get("soapaction")
        .unwrap()
        .to_str()
        .unwrap()
        .trim_matches('"');
    let (service, action) = action_header.rsplit_once('#').unwrap();
    let document = roxmltree::Document::parse(&body).unwrap();
    let operation = document
        .descendants()
        .find(|node| node.is_element() && node.tag_name().name() == action)
        .unwrap();
    let arguments: HashMap<String, String> = operation
        .children()
        .filter(|node| node.is_element())
        .map(|node| {
            (
                node.tag_name().name().to_owned(),
                node.text().unwrap_or("").to_owned(),
            )
        })
        .collect();
    let mut state = state.lock().unwrap();
    state.actions.push((action.to_owned(), arguments.clone()));
    if state.fail_action.as_deref() == Some(action) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            [("content-type", "text/xml; charset=utf-8")],
            r#"<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/"><s:Body><s:Fault>
<faultcode>s:Client</faultcode><faultstring>UPnPError</faultstring><detail>
<UPnPError xmlns="urn:schemas-upnp-org:control-1-0"><errorCode>701</errorCode>
<errorDescription>模拟音箱拒绝播放</errorDescription></UPnPError></detail>
</s:Fault></s:Body></s:Envelope>"#
                .to_owned(),
        );
    }
    let output = match action {
        "SetAVTransportURI" => { state.uri = arguments["CurrentURI"].clone(); state.position = "00:00:00".into(); String::new() }
        "Play" => { state.transport = "PLAYING".into(); String::new() }
        "Pause" => { state.transport = "PAUSED_PLAYBACK".into(); String::new() }
        "Stop" => { state.transport = "STOPPED".into(); String::new() }
        "Seek" => { state.position = arguments["Target"].clone(); String::new() }
        "SetVolume" => { state.volume = arguments["DesiredVolume"].parse().unwrap(); String::new() }
        "GetVolume" => format!("<CurrentVolume>{}</CurrentVolume>", state.volume),
        "GetTransportInfo" => format!("<CurrentTransportState>{}</CurrentTransportState><CurrentTransportStatus>OK</CurrentTransportStatus><CurrentSpeed>1</CurrentSpeed>", state.transport),
        "GetPositionInfo" => format!("<Track>1</Track><TrackDuration>00:03:00</TrackDuration><TrackMetaData></TrackMetaData><TrackURI>{}</TrackURI><RelTime>{}</RelTime><AbsTime>{}</AbsTime><RelCount>0</RelCount><AbsCount>0</AbsCount>", escape(&state.uri), state.position, state.position),
        _ => String::new(),
    };
    (StatusCode::OK, [("content-type", "text/xml; charset=utf-8")], format!(
        "<s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\"><s:Body><u:{action}Response xmlns:u=\"{service}\">{output}</u:{action}Response></s:Body></s:Envelope>"))
}

struct Fixture {
    directory: tempfile::TempDir,
    engine: Arc<Engine>,
    renderer: Arc<Mutex<RendererState>>,
    renderer_url: String,
    renderer_shutdown: CancellationToken,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.engine.shutdown();
        self.renderer_shutdown.cancel();
    }
}

impl Fixture {
    async fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let engine = Engine::new(directory.path().join("data")).await.unwrap();
        let renderer = Arc::new(Mutex::new(RendererState {
            id: format!("uuid:{}", uuid::Uuid::new_v4()),
            transport: "STOPPED".into(),
            position: "00:00:00".into(),
            volume: 25,
            ..RendererState::default()
        }));
        let router = Router::new()
            .route("/description.xml", get(description))
            .route("/transport", post(soap))
            .route("/rendering", post(soap))
            .with_state(renderer.clone());
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let renderer_url = format!(
            "http://127.0.0.1:{}/description.xml",
            listener.local_addr().unwrap().port()
        );
        let renderer_shutdown = CancellationToken::new();
        let token = renderer_shutdown.clone();
        tokio::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(token.cancelled_owned())
                .await
                .unwrap();
        });
        Self {
            directory,
            engine,
            renderer,
            renderer_url,
            renderer_shutdown,
        }
    }

    async fn import(&self, filename: &str, contents: &[u8]) -> String {
        let path = self.directory.path().join(filename);
        std::fs::write(&path, contents).unwrap();
        assert_eq!(self.engine.import(vec![path]).await.unwrap().added, 1);
        self.engine
            .snapshot()
            .await
            .unwrap()
            .tracks
            .iter()
            .find(|track| track.filename == filename)
            .unwrap()
            .id
            .clone()
    }

    async fn connect(&self) {
        let snapshot = self.engine.add_device(&self.renderer_url).await.unwrap();
        assert_eq!(snapshot.devices.len(), 1);
        assert_eq!(snapshot.devices[0].name, "测试小爱音箱 Pro");
        let id = snapshot.devices[0].id.clone();
        let snapshot = self.engine.select(&id).await.unwrap();
        assert_eq!(snapshot.selected_device_id.as_deref(), Some(id.as_str()));
    }

    fn uri(&self) -> String {
        self.renderer.lock().unwrap().uri.clone()
    }
}

#[tokio::test]
async fn import_cast_and_control_use_real_soap_and_fetchable_music() {
    let fixture = Fixture::new().await;
    let audio = b"ID3\0local music with unicode filename";
    let first = fixture.import("夏天 & 海.mp3", audio).await;
    let second = fixture.import("第二首.mp3", b"another song").await;
    fixture.connect().await;
    let snapshot = fixture.engine.play(&first).await.unwrap();
    assert_eq!(snapshot.current_track_id.as_deref(), Some(first.as_str()));
    let uri = fixture.uri();
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let response = client.get(&uri).send().await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["content-type"], "audio/mpeg");
    assert_eq!(response.bytes().await.unwrap().as_ref(), audio);
    let response = client.head(&uri).send().await.unwrap();
    assert_eq!(
        response.headers()["content-length"].to_str().unwrap(),
        audio.len().to_string()
    );
    assert!(response.bytes().await.unwrap().is_empty());
    {
        let renderer = fixture.renderer.lock().unwrap();
        let mutations: Vec<_> = renderer
            .actions
            .iter()
            .filter(|(action, _)| action == "SetAVTransportURI" || action == "Play")
            .collect();
        assert_eq!(mutations[0].0, "SetAVTransportURI");
        assert_eq!(mutations[1].0, "Play");
        let metadata = roxmltree::Document::parse(&mutations[0].1["CurrentURIMetaData"]).unwrap();
        assert_eq!(
            metadata
                .descendants()
                .find(|node| node.is_element() && node.tag_name().name() == "title")
                .unwrap()
                .text(),
            Some("夏天 & 海")
        );
    }
    assert_eq!(
        fixture
            .engine
            .control("pause", Value::Null)
            .await
            .unwrap()
            .transport_state,
        "PAUSED_PLAYBACK"
    );
    fixture.engine.control("resume", Value::Null).await.unwrap();
    assert_eq!(fixture.renderer.lock().unwrap().transport, "PLAYING");
    assert_eq!(
        fixture
            .engine
            .control("seek", json!(42))
            .await
            .unwrap()
            .position,
        42
    );
    assert_eq!(fixture.renderer.lock().unwrap().position, "00:00:42");
    assert_eq!(
        fixture
            .engine
            .control("volume", json!(38))
            .await
            .unwrap()
            .volume,
        Some(38)
    );
    assert_eq!(fixture.renderer.lock().unwrap().volume, 38);
    assert_eq!(
        fixture
            .engine
            .control("next", Value::Null)
            .await
            .unwrap()
            .current_track_id
            .as_deref(),
        Some(second.as_str())
    );
    assert_eq!(
        fixture
            .engine
            .control("previous", Value::Null)
            .await
            .unwrap()
            .current_track_id
            .as_deref(),
        Some(first.as_str())
    );
    fixture
        .engine
        .control("mode", json!("repeat-all"))
        .await
        .unwrap();
    fixture.engine.control("next", Value::Null).await.unwrap();
    assert_eq!(
        fixture
            .engine
            .control("next", Value::Null)
            .await
            .unwrap()
            .current_track_id
            .as_deref(),
        Some(first.as_str())
    );
    let stopped = fixture.engine.control("stop", Value::Null).await.unwrap();
    assert_eq!(stopped.transport_state, "STOPPED");
    assert_eq!(stopped.current_track_id.as_deref(), Some(first.as_str()));
    assert_eq!(fixture.renderer.lock().unwrap().transport, "STOPPED");
}

#[tokio::test]
async fn rename_is_immediate_for_voice_and_does_not_interrupt_current_playback() {
    let fixture = Fixture::new().await;
    let id = fixture.import("视频原标题.mp3", b"original music").await;
    fixture.connect().await;
    let playing = fixture.engine.play(&id).await.unwrap();
    let before = fixture.uri();
    let actions_before = fixture.renderer.lock().unwrap().actions.len();
    let snapshot = fixture
        .engine
        .rename_track(id.clone(), "螺旋 现场版".into(), "视频原标题".into())
        .await
        .unwrap();
    assert_eq!(snapshot.current_track_id.as_deref(), Some(id.as_str()));
    assert_eq!(snapshot.transport_state, playing.transport_state);
    assert_eq!(snapshot.tracks[0].title, "螺旋 现场版");
    assert_eq!(fixture.uri(), before);
    assert_eq!(fixture.renderer.lock().unwrap().transport, "PLAYING");
    assert_eq!(
        fixture.renderer.lock().unwrap().actions.len(),
        actions_before
    );

    let result = fixture
        .engine
        .voice_command("小爱同学，播放本地螺旋现场版", None)
        .await
        .unwrap();
    assert!(result.contains("螺旋 现场版"));
    assert_eq!(
        fixture
            .engine
            .snapshot()
            .await
            .unwrap()
            .current_track_id
            .as_deref(),
        Some(id.as_str())
    );
    {
        let renderer = fixture.renderer.lock().unwrap();
        let metadata = &renderer
            .actions
            .iter()
            .rev()
            .find(|(action, _)| action == "SetAVTransportURI")
            .unwrap()
            .1["CurrentURIMetaData"];
        assert!(metadata.contains("螺旋 现场版"));
    }
    assert!(fixture
        .engine
        .voice_command("播放本地视频原标题", None)
        .await
        .is_err());
}

#[tokio::test]
async fn removing_music_keeps_files_and_only_stops_the_current_track() {
    let fixture = Fixture::new().await;
    let first = fixture.import("first.mp3", b"first audio").await;
    let second = fixture.import("second.mp3", b"second audio").await;
    fixture.connect().await;
    fixture.engine.play(&first).await.unwrap();
    let uri = fixture.uri();
    let before = fixture.renderer.lock().unwrap().actions.len();
    let snapshot = fixture.engine.remove_track(second.clone()).await.unwrap();
    assert_eq!(snapshot.current_track_id.as_deref(), Some(first.as_str()));
    assert_eq!(fixture.renderer.lock().unwrap().actions.len(), before);
    assert_eq!(
        std::fs::read(fixture.directory.path().join("second.mp3")).unwrap(),
        b"second audio"
    );

    fixture.renderer.lock().unwrap().fail_action = Some("Stop".into());
    assert!(fixture
        .engine
        .remove_track(first.clone())
        .await
        .err()
        .unwrap()
        .contains("尚未移除"));
    assert_eq!(fixture.engine.snapshot().await.unwrap().tracks.len(), 1);
    fixture.renderer.lock().unwrap().fail_action = None;
    let removed = fixture.engine.remove_track(first.clone()).await.unwrap();
    assert!(removed.tracks.is_empty());
    assert!(removed.current_track_id.is_none());
    assert_eq!(removed.transport_state, "STOPPED");
    assert_eq!(fixture.renderer.lock().unwrap().transport, "STOPPED");
    assert_eq!(
        reqwest::Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .get(uri)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        std::fs::read(fixture.directory.path().join("first.mp3")).unwrap(),
        b"first audio"
    );
    assert!(fixture.engine.play(&first).await.is_err());
    assert!(fixture.engine.remove_track(first).await.is_err());
    fixture
        .engine
        .import(vec![fixture.directory.path().join("first.mp3")])
        .await
        .unwrap();
    assert_eq!(fixture.engine.snapshot().await.unwrap().tracks.len(), 1);
}

#[tokio::test]
async fn renderer_fault_is_visible_and_does_not_report_playback_success() {
    let fixture = Fixture::new().await;
    let id = fixture.import("song.mp3", b"audio").await;
    fixture.connect().await;
    fixture.renderer.lock().unwrap().fail_action = Some("Play".into());
    assert!(fixture.engine.play(&id).await.is_err());
    let snapshot = fixture.engine.snapshot().await.unwrap();
    assert!(snapshot.status_error.is_some());
    assert!(snapshot.current_track_id.is_none());
    assert_ne!(snapshot.transport_state, "PLAYING");
}

#[tokio::test]
async fn switching_speakers_stops_previous_output_but_reselection_does_not() {
    let first = Fixture::new().await;
    let second = Fixture::new().await;
    let track = first.import("song.mp3", b"audio").await;
    first.connect().await;
    first.engine.play(&track).await.unwrap();
    let original = first
        .engine
        .snapshot()
        .await
        .unwrap()
        .selected_device_id
        .unwrap();
    let stop_count = || {
        first
            .renderer
            .lock()
            .unwrap()
            .actions
            .iter()
            .filter(|(action, _)| action == "Stop")
            .count()
    };
    let before = stop_count();
    first.engine.select(&original).await.unwrap();
    assert_eq!(stop_count(), before);
    assert_eq!(first.renderer.lock().unwrap().transport, "PLAYING");
    let discovered = first.engine.add_device(&second.renderer_url).await.unwrap();
    let other = discovered
        .devices
        .iter()
        .find(|device| device.id != original)
        .unwrap()
        .id
        .clone();
    let selected = first.engine.select(&other).await.unwrap();
    assert_eq!(selected.selected_device_id.as_deref(), Some(other.as_str()));
    assert!(selected.current_track_id.is_none());
    assert_eq!(stop_count(), before + 1);
    assert_eq!(first.renderer.lock().unwrap().transport, "STOPPED");
    assert!(second.renderer.lock().unwrap().actions.is_empty());
    first.engine.play(&track).await.unwrap();
    assert_eq!(second.renderer.lock().unwrap().transport, "PLAYING");
    assert_eq!(first.renderer.lock().unwrap().transport, "STOPPED");
}

#[tokio::test]
async fn replaced_sources_invalid_controls_and_shutdown_are_handled() {
    let fixture = Fixture::new().await;
    let id = fixture.import("song.mp3", b"original").await;
    assert!(fixture.engine.play(&id).await.is_err());
    assert!(fixture.engine.select("unknown").await.is_err());
    fixture.connect().await;
    assert!(fixture.engine.control("volume", json!(101)).await.is_err());
    assert!(fixture.engine.control("seek", json!(-1)).await.is_err());
    assert!(fixture
        .engine
        .control("mode", json!("unsupported"))
        .await
        .is_err());
    let path: PathBuf = fixture.directory.path().join("song.mp3");
    std::fs::rename(&path, path.with_extension("old")).unwrap();
    std::fs::write(path, b"private replacement").unwrap();
    assert!(fixture.engine.play(&id).await.is_err());
    assert!(fixture.renderer.lock().unwrap().actions.is_empty());
    let port = fixture.engine.snapshot().await.unwrap().media_port;
    fixture.engine.shutdown();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port))
                .await
                .is_err()
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn voice_commands_reach_renderer_and_guard_device_binding() {
    let fixture = Fixture::new().await;
    let song = fixture.import("晴天.mp3", b"music").await;
    fixture.import("晴天现场版.mp3", b"live music").await;
    fixture.connect().await;
    let target = fixture
        .engine
        .snapshot()
        .await
        .unwrap()
        .selected_device_id
        .unwrap();
    fixture
        .engine
        .voice_command("小爱同学，播放本地晴天", Some(&target))
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
        Some(song.as_str())
    );
    fixture
        .engine
        .voice_command("本地音量三十五", Some(&target))
        .await
        .unwrap();
    assert_eq!(fixture.renderer.lock().unwrap().volume, 35);
    fixture
        .engine
        .voice_command("本地暂停", Some(&target))
        .await
        .unwrap();
    assert_eq!(
        fixture.renderer.lock().unwrap().transport,
        "PAUSED_PLAYBACK"
    );
    let count = fixture.renderer.lock().unwrap().actions.len();
    assert!(fixture
        .engine
        .voice_command("本地继续", Some("another-speaker"))
        .await
        .unwrap_err()
        .contains("切换"));
    assert!(fixture
        .engine
        .voice_command("播放本地晴", Some(&target))
        .await
        .unwrap_err()
        .contains("2 首"));
    assert!(fixture
        .engine
        .voice_command("播放本地不存在", Some(&target))
        .await
        .unwrap_err()
        .contains("没有找到"));
    assert!(fixture
        .engine
        .voice_command("播放流行音乐", Some(&target))
        .await
        .is_err());
    assert_eq!(fixture.renderer.lock().unwrap().actions.len(), count);
}

#[tokio::test]
async fn imported_download_keeps_source_and_title_after_restart() {
    use xiaomi_dlna_desktop::library::{Library, TrackSource};
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("BV1xx411c7mD.m4a");
    std::fs::write(&path, b"fixture audio").unwrap();
    let source = TrackSource {
        provider: "bilibili".into(),
        id: "BV1xx411c7mD:1".into(),
        url: "https://www.bilibili.com/video/BV1xx411c7mD".into(),
    };
    let mut library = Library::load(directory.path().to_owned()).unwrap();
    let track = library
        .import_download(&path, "自创歌曲", source.clone())
        .unwrap();
    let mut library = Library::load(directory.path().to_owned()).unwrap();
    assert_eq!(library.get(&track.id).unwrap().source, Some(source.clone()));
    assert_eq!(library.get(&track.id).unwrap().title, "自创歌曲");
    let again = library.import_download(&path, "更新标题", source).unwrap();
    assert_eq!(again.id, track.id);
    assert_eq!(library.tracks().len(), 1);
}
