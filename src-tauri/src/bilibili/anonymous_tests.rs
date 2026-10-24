use super::*;
use axum::{
    extract::{Query, State},
    http::{HeaderMap, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

#[derive(Default)]
struct Requests {
    visitor: AtomicUsize,
    nav: AtomicUsize,
    media: AtomicUsize,
    detail: AtomicUsize,
    bad_visitor: AtomicBool,
    blocked: AtomicBool,
}

fn audio_bytes() -> Vec<u8> {
    let mut bytes = Vec::new();
    for (kind, body) in [
        (b"ftyp", b"M4A     ".as_slice()),
        (b"moov", b"fixture".as_slice()),
        (b"mdat", b"public audio".as_slice()),
    ] {
        bytes.extend_from_slice(&((body.len() + 8) as u32).to_be_bytes());
        bytes.extend_from_slice(kind);
        bytes.extend_from_slice(body);
    }
    bytes
}

async fn serve(
    State(state): State<Arc<Requests>>,
    uri: Uri,
    Query(query): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let cookie = headers
        .get("cookie")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    let value = match uri.path() {
        "/x/frontend/finger/spi" => {
            state.visitor.fetch_add(1, Ordering::SeqCst);
            assert!(cookie.is_empty());
            if state.bad_visitor.load(Ordering::SeqCst) {
                serde_json::json!({"code":0,"data":{"b_3":"bad\r\nSESSDATA=leak","b_4":"device-four"}})
            } else {
                serde_json::json!({"code":0,"data":{"b_3":"device-three","b_4":"device-four","SESSDATA":"must-not-be-used"}})
            }
        }
        "/x/web-interface/nav" => {
            state.nav.fetch_add(1, Ordering::SeqCst);
            if cookie.contains("SESSDATA=fixture") {
                serde_json::json!({"code":0,"data":{"isLogin":true}})
            } else {
                assert_eq!(cookie, "buvid3=device-three; buvid4=device-four");
                // Anonymous key discovery must accept -101 with public keys.
                serde_json::json!({"code":-101,"data":{"isLogin":false,"wbi_img":{"img_url":"https://i.example/7cd084941338484aae1ad9425b84077c.png","sub_url":"https://i.example/4932caff0ff746eab6f01bf08b70ac45.png"}}})
            }
        }
        "/x/web-interface/view" => return StatusCode::PRECONDITION_FAILED.into_response(),
        "/x/web-interface/wbi/view/detail" => {
            state.detail.fetch_add(1, Ordering::SeqCst);
            assert_eq!(cookie, "buvid3=device-three; buvid4=device-four");
            assert_eq!(headers["origin"], "https://www.bilibili.com");
            assert_eq!(query["platform"], "web");
            assert_eq!(query["w_rid"].len(), 32);
            assert!(query["wts"].parse::<u64>().is_ok());
            if state.blocked.load(Ordering::SeqCst) {
                return StatusCode::PRECONDITION_FAILED.into_response();
            }
            if query["bvid"] == "BV1C54y1b79F" {
                serde_json::json!({"code":-403})
            } else {
                serde_json::json!({"code":0,"data":{"View":{"bvid":query["bvid"],"title":"公开视频","pages":[{"cid":10,"page":1,"part":"第一首"},{"cid":20,"page":2,"part":"第二首"}]}}})
            }
        }
        "/x/player/wbi/playurl" => {
            assert_eq!(cookie, "buvid3=device-three; buvid4=device-four");
            assert_eq!(query["w_rid"].len(), 32);
            assert_eq!(query["cid"], "20");
            serde_json::json!({"code":0,"data":{"dash":{"audio":[{"codecs":"mp4a.40.2","mimeType":"audio/mp4","bandwidth":128000,"baseUrl":"https://cdn.bilivideo.com/public.m4a"}]}}})
        }
        "/public.m4a" => {
            assert!(
                cookie.is_empty(),
                "No account or visitor cookies may reach the CDN"
            );
            state.media.fetch_add(1, Ordering::SeqCst);
            return ([("content-type", "audio/mp4")], audio_bytes()).into_response();
        }
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    axum::Json(value).into_response()
}

async fn fixture() -> (
    tempfile::TempDir,
    Arc<Downloader>,
    Arc<Requests>,
    tokio::task::JoinHandle<()>,
) {
    let directory = tempfile::tempdir().unwrap();
    let state = Arc::new(Requests::default());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = Router::new().fallback(get(serve)).with_state(state.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let mut downloader = Downloader::new(directory.path()).unwrap();
    let owned = Arc::get_mut(&mut downloader).unwrap();
    owned.client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    owned.test_origin = Some(format!("http://{address}").parse().unwrap());
    (directory, downloader, state, server)
}

#[tokio::test]
async fn public_video_parses_and_downloads_without_login_even_after_logout() {
    let (directory, downloader, requests, server) = fixture().await;
    let engine = Engine::new(directory.path().to_owned()).await.unwrap();
    assert!(!downloader.state().await.authenticated);
    let (first, second) = tokio::join!(
        downloader.parse("https://www.bilibili.com/video/BV1DWLJzaEDv/?p=2"),
        downloader.parse("BV1sVtw6JEeq"),
    );
    let preview = first.unwrap();
    assert_eq!(preview.parts.len(), 2);
    assert_eq!(preview.selected_page, 2);
    assert_eq!(second.unwrap().selected_page, 1);
    let job = downloader
        .start(
            Arc::downgrade(&engine),
            &preview.id,
            20,
            engine.cancellation(),
        )
        .await
        .unwrap();
    let completed = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let job = downloader
                .state()
                .await
                .jobs
                .into_iter()
                .find(|j| j.id == job.id)
                .unwrap();
            if !active_phase(&job.phase) {
                break job;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(completed.phase, "completed", "{:?}", completed.error);
    assert_eq!(
        std::fs::read(completed.path.unwrap()).unwrap(),
        audio_bytes()
    );
    assert_eq!(engine.snapshot().await.unwrap().tracks.len(), 1);
    assert!(!downloader.state().await.authenticated);
    assert_eq!(requests.visitor.load(Ordering::SeqCst), 1);
    assert_eq!(requests.nav.load(Ordering::SeqCst), 1);
    assert_eq!(requests.media.load(Ordering::SeqCst), 1);
    // Removing a downloaded track must allow adding the same P again in this
    // process, even though its completed job remains in the download history.
    let track = engine.snapshot().await.unwrap().tracks[0].clone();
    engine.remove_track(track.id).await.unwrap();
    downloader
        .start(
            Arc::downgrade(&engine),
            &preview.id,
            20,
            engine.cancellation(),
        )
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if downloader
                .state()
                .await
                .jobs
                .iter()
                .all(|j| !active_phase(&j.phase))
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(engine.snapshot().await.unwrap().tracks.len(), 1);
    assert!(downloader
        .state()
        .await
        .jobs
        .iter()
        .all(|j| j.phase == "completed"));
    downloader.set_session("SESSDATA=fixture").await.unwrap();
    assert!(downloader.state().await.authenticated);
    downloader.disconnect().await;
    downloader.parse("BV1DWLJzaEDv").await.unwrap();
    assert!(!downloader.state().await.authenticated);
    assert_eq!(requests.visitor.load(Ordering::SeqCst), 1);
    *downloader.wbi.lock().await =
        Some((Instant::now() - Duration::from_secs(31), "expired".into()));
    downloader.parse("BV1DWLJzaEDv").await.unwrap();
    assert_eq!(requests.nav.load(Ordering::SeqCst), 3); // initial, account validation, refresh
    engine.shutdown();
    server.abort();
}

#[tokio::test]
async fn anonymous_requests_preserve_permission_errors_and_do_not_retry_limits() {
    let (_directory, downloader, requests, server) = fixture().await;
    let error = downloader.parse("BV1C54y1b79F").await.unwrap_err();
    assert!(error.contains("访问权限"));
    requests.blocked.store(true, Ordering::SeqCst);
    let error = downloader.parse("BV1DWLJzaEDv").await.unwrap_err();
    assert!(error.contains("受限"));
    assert!(!error.contains("登录"));
    assert!(downloader.previews.lock().await.is_empty());
    assert_eq!(requests.visitor.load(Ordering::SeqCst), 1);
    assert!(downloader.state().await.jobs.is_empty());
    assert_eq!(requests.detail.load(Ordering::SeqCst), 2);
    server.abort();
}

#[tokio::test]
async fn invalid_anonymous_identifier_never_becomes_a_cookie_or_login() {
    let (_directory, downloader, requests, server) = fixture().await;
    requests.bad_visitor.store(true, Ordering::SeqCst);
    let error = downloader.parse("BV1DWLJzaEDv").await.unwrap_err();
    assert!(!error.contains("leak"));
    assert_eq!(requests.nav.load(Ordering::SeqCst), 0);
    assert!(!downloader.state().await.authenticated);
    assert!(downloader.previews.lock().await.is_empty());
    requests.bad_visitor.store(false, Ordering::SeqCst);
    downloader.parse("BV1DWLJzaEDv").await.unwrap();
    assert_eq!(requests.visitor.load(Ordering::SeqCst), 2);
    assert!(!downloader.state().await.authenticated);
    server.abort();
}
