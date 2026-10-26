use super::*;
use axum::{extract::Query, http::Uri};
use std::collections::HashMap;

struct Fixture {
    _directory: tempfile::TempDir,
    engine: Arc<Engine>,
    running: Running,
    upstream: tokio::task::JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.engine.shutdown();
        self.upstream.abort();
    }
}
impl Fixture {
    async fn new() -> Self {
        async fn upstream(uri: Uri, Query(query): Query<HashMap<String, String>>) -> Response {
            let value = match uri.path() {
                "/x/frontend/finger/spi" => json!({"code":0,"data":{"b_3":"test","b_4":"test"}}),
                "/x/web-interface/nav" => {
                    json!({"code":-101,"data":{"isLogin":false,"wbi_img":{"img_url":"https://i.example/7cd084941338484aae1ad9425b84077c.png","sub_url":"https://i.example/4932caff0ff746eab6f01bf08b70ac45.png"}}})
                }
                "/x/web-interface/wbi/view/detail" => {
                    json!({"code":0,"data":{"View":{"bvid":query["bvid"],"title":"手机发送音乐","owner":{"name":"测试作者"},"pages": if query["bvid"] == "BV1sVtw6JEeq" { json!([{"cid":30,"page":1,"part":"单 P"}]) } else { json!([{"cid":10,"page":1,"part":"第一首"},{"cid":20,"page":2,"part":"第二首"}]) }}}})
                }
                "/x/player/wbi/playurl" => {
                    json!({"code":0,"data":{"dash":{"audio":[{"codecs":"mp4a.40.2","mimeType":"audio/mp4","bandwidth":128000,"baseUrl":"https://cdn.bilivideo.com/test.m4a"}]}}})
                }
                "/test.m4a" => {
                    let mut bytes = Vec::new();
                    for (kind, body) in [
                        (b"ftyp", b"M4A     ".as_slice()),
                        (b"moov", b"fixture".as_slice()),
                        (b"mdat", b"test audio".as_slice()),
                    ] {
                        bytes.extend_from_slice(&((body.len() + 8) as u32).to_be_bytes());
                        bytes.extend_from_slice(kind);
                        bytes.extend_from_slice(body);
                    }
                    return ([("content-type", "audio/mp4")], bytes).into_response();
                }
                _ => return StatusCode::NOT_FOUND.into_response(),
            };
            Json(value).into_response()
        }
        let directory = tempfile::tempdir().unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap())
            .parse()
            .unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, Router::new().fallback(get(upstream)))
                .await
                .unwrap();
        });
        let mut engine = Engine::new(directory.path().to_owned()).await.unwrap();
        Arc::get_mut(&mut Arc::get_mut(&mut engine).unwrap().downloads)
            .unwrap()
            .use_test_origin(origin);
        let running = start(
            &engine,
            vec![Ipv4Addr::LOCALHOST],
            Ipv4Addr::LOCALHOST,
            0,
            directory.path().join("mobile.sha256"),
        )
        .await
        .unwrap();
        Self {
            _directory: directory,
            engine,
            running,
            upstream: server,
        }
    }
    fn client(&self) -> reqwest::Client {
        reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap()
    }
    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.running.urls[0])
    }
    async fn pair(&self) -> String {
        let body: Value = self
            .client()
            .post(self.url("/api/pair"))
            .json(&json!({"ticket":self.running.state.ticket}))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        body["token"].as_str().unwrap().to_owned()
    }
}

#[test]
fn accepts_android_share_text_without_weakening_url_validation() {
    use crate::bilibili::shared_video_input as parse;
    assert_eq!(parse("【音乐现场】 https://www.bilibili.com/zh-Hans/video/BV1DWLJzaEDv/?p=2&spm_id_from=share").unwrap(), "https://www.bilibili.com/video/BV1DWLJzaEDv/?p=2");
    assert_eq!(
        parse("【歌曲-哔哩哔哩】 https://b23.tv/AbC123").unwrap(),
        "https://b23.tv/AbC123"
    );
    assert!(parse("BV1DWLJzaEDv").is_ok());
    for input in [
        "https://evil.example/video/BV1DWLJzaEDv",
        "https://bilibili.com.evil.example/video/BV1DWLJzaEDv",
        "https://user@www.bilibili.com/video/BV1DWLJzaEDv",
        "http://127.0.0.1/private",
        "BV1DWLJzaEDv BV1sVtw6JEeq",
        "https://www.bilibili.com/video/BV1DWLJzaEDv/?p=0",
    ] {
        assert!(parse(input).is_err(), "{input}");
    }
}

#[tokio::test]
async fn lan_boundary_requires_pairing_and_revokes_access() {
    let f = Fixture::new().await;
    let client = f.client();
    assert_eq!(
        client
            .get(f.url("/api/jobs"))
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    assert_eq!(
        client
            .post(f.url("/api/parse"))
            .json(&json!({"text":"BV1DWLJzaEDv"}))
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    assert_eq!(
        client
            .post(f.url("/api/pair"))
            .header("origin", "https://evil.example")
            .json(&json!({"ticket":f.running.state.ticket}))
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    assert_eq!(
        client
            .get(f.url("/"))
            .header("host", "evil.example")
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    let page = client.get(f.url("/")).send().await.unwrap();
    assert_eq!(page.headers()["cache-control"], "no-store");
    assert!(page.text().await.unwrap().contains("发送音乐到电脑"));
    let token = f.pair().await;
    assert_eq!(
        client
            .post(f.url("/api/pair"))
            .json(&json!({"ticket":f.running.state.ticket}))
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    let empty: Value = client
        .get(f.url("/api/jobs"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(empty, json!({"jobs":[]}));
    assert_eq!(
        client
            .post(f.url("/api/download"))
            .bearer_auth(&token)
            .json(&json!({"preview_id":"desktop-preview", "cids":[10]}))
            .send()
            .await
            .unwrap()
            .status(),
        400
    );
    assert_eq!(
        client
            .post(f.url("/api/parse"))
            .bearer_auth(&token)
            .json(&json!({"text":"x".repeat(20000)}))
            .send()
            .await
            .unwrap()
            .status(),
        413
    );
    f.running.state.shutdown.cancel();
    let after = client
        .get(f.url("/api/jobs"))
        .bearer_auth(&token)
        .send()
        .await;
    assert!(after.is_err() || after.unwrap().status() == 410);
    let fresh = start(
        &f.engine,
        vec![Ipv4Addr::LOCALHOST],
        Ipv4Addr::LOCALHOST,
        0,
        f.running.state.cache.clone(),
    )
    .await
    .unwrap();
    assert_eq!(
        fresh.state.pairing.lock().await.hash.as_deref(),
        Some(crate::mobile_pairing::digest(&token).as_str())
    );
    assert_eq!(
        client
            .get(format!("{}/api/jobs", fresh.urls[0]))
            .bearer_auth(token)
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
}

#[tokio::test]
async fn wrong_pairing_ticket_is_bounded() {
    let f = Fixture::new().await;
    for _ in 0..5 {
        assert_eq!(
            f.client()
                .post(f.url("/api/pair"))
                .json(&json!({"ticket":"incorrect"}))
                .send()
                .await
                .unwrap()
                .status(),
            401
        );
    }
    assert_eq!(
        f.client()
            .post(f.url("/api/pair"))
            .json(&json!({"ticket":f.running.state.ticket}))
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
}

fn offline_session(directory: &std::path::Path, created: Instant) -> Arc<Session> {
    let cache = directory.join("mobile.sha256");
    let hash = crate::mobile_pairing::load(&cache).unwrap();
    Arc::new(Session {
        engine: Weak::new(),
        downloader: Downloader::new(directory).unwrap(),
        shutdown: CancellationToken::new(),
        hosts: HashSet::new(),
        ticket: "a".repeat(64),
        cache,
        created,
        pairing: Mutex::new(Pairing { attempts: 0, hash }),
        operation: Mutex::new(()),
        previews: Mutex::new(HashSet::new()),
        jobs: Mutex::new(HashSet::new()),
    })
}
fn offline_receiver(session: &Arc<Session>) -> Receiver {
    Receiver {
        running: Mutex::new(Some(Running {
            state: session.clone(),
            urls: vec![format!("http://192.168.1.2:{MOBILE_PORT}")],
            server: None,
        })),
        cache: session.cache.clone(),
    }
}
async fn redeem(session: &Arc<Session>, ticket: &str) -> ApiResult {
    pair(
        State(session.clone()),
        Json(PairRequest {
            ticket: ticket.into(),
        }),
    )
    .await
}

#[tokio::test]
async fn pairing_lifecycle_without_network() {
    let directory = tempfile::tempdir().unwrap();
    let session = offline_session(directory.path(), Instant::now());
    let receiver = offline_receiver(&session);
    let initial = receiver.status().await;
    assert!(initial.pairing_available);
    assert_eq!(
        initial.entries[0].url,
        format!("http://192.168.1.2:8998/#pair={}", session.ticket)
    );
    assert_eq!(
        redeem(&session, "wrong").await.unwrap_err().0,
        StatusCode::UNAUTHORIZED
    );
    let body = redeem(&session, &session.ticket).await.unwrap().0;
    let token = body["token"].as_str().unwrap();
    assert_eq!(token.len(), 64);
    assert_ne!(token, session.ticket);
    let on_disk = std::fs::read_to_string(&session.cache).unwrap();
    assert_eq!(on_disk, crate::mobile_pairing::digest(token));
    assert!(!on_disk.contains(token));
    assert_eq!(
        redeem(&session, &session.ticket).await.unwrap_err().0,
        StatusCode::FORBIDDEN
    );
    let status = receiver.status().await;
    assert!(status.paired);
    assert!(!status.pairing_available);
    assert_eq!(status.entries[0].url, "http://192.168.1.2:8998/");
    assert!(!receiver.disable().await.enabled);
    assert!(session.shutdown.is_cancelled());
    // Reconstructed process state restores only the digest and accepts the phone.
    let resumed = offline_session(directory.path(), Instant::now());
    assert_eq!(
        resumed.pairing.lock().await.hash.as_deref(),
        Some(on_disk.as_str())
    );
    let receiver = offline_receiver(&resumed);
    assert!(receiver.status().await.paired);
    receiver.forget().await.unwrap();
    assert!(resumed.shutdown.is_cancelled());
    let fresh = offline_session(directory.path(), Instant::now());
    assert!(fresh.pairing.lock().await.hash.is_none());
    assert!(offline_receiver(&fresh).status().await.pairing_available);
}

#[tokio::test]
async fn expired_and_locked_pairing_without_network() {
    let directory = tempfile::tempdir().unwrap();
    let expired = offline_session(directory.path(), Instant::now() - Duration::from_secs(601));
    assert_eq!(
        redeem(&expired, &expired.ticket).await.unwrap_err().0,
        StatusCode::FORBIDDEN
    );
    let locked = offline_session(directory.path(), Instant::now());
    for _ in 0..5 {
        assert_eq!(
            redeem(&locked, "wrong").await.unwrap_err().0,
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(
        redeem(&locked, &locked.ticket).await.unwrap_err().0,
        StatusCode::FORBIDDEN
    );
    assert!(!offline_receiver(&locked).status().await.pairing_available);
}

#[tokio::test]
async fn pairing_save_failure_without_network() {
    let directory = tempfile::tempdir().unwrap();
    let session = offline_session(directory.path(), Instant::now());
    // A directory at the destination makes atomic file publication fail reliably.
    std::fs::create_dir(&session.cache).unwrap();
    assert_eq!(
        redeem(&session, &session.ticket).await.unwrap_err().0,
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert!(session.pairing.lock().await.hash.is_none());
    assert!(crate::mobile_pairing::load(&session.cache).is_err());
    let receiver = offline_receiver(&session);
    assert!(receiver.forget().await.is_err());
    assert!(!receiver.status().await.enabled);
}

#[tokio::test]
async fn phone_single_and_multi_part_downloads_reach_the_pc_library() {
    let f = Fixture::new().await;
    let token = f.pair().await;
    let client = f.client();
    for (text, cids, parts) in [
        (
            "分享 https://www.bilibili.com/video/BV1DWLJzaEDv/?p=2",
            vec![10, 20],
            2,
        ),
        ("BV1sVtw6JEeq", vec![30], 1),
    ] {
        let preview: Value = client
            .post(f.url("/api/parse"))
            .bearer_auth(&token)
            .json(&json!({"text":text}))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(preview["parts"].as_array().unwrap().len(), parts);
        assert_eq!(preview["selected_page"], if parts == 2 { 2 } else { 1 });
        let payload = json!({"preview_id":preview["id"], "cids":cids});
        let result: Value = client
            .post(f.url("/api/download"))
            .bearer_auth(&token)
            .json(&payload)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(result["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|i| i["accepted"] == true));
        let again: Value = client
            .post(f.url("/api/download"))
            .bearer_auth(&token)
            .json(&payload)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(again["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|i| i["accepted"] == false));
    }
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if f.engine
                .downloads
                .state()
                .await
                .jobs
                .iter()
                .filter(|j| j.phase == "completed")
                .count()
                == 3
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let jobs: Value = client
        .get(f.url("/api/jobs"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(jobs["jobs"].as_array().unwrap().len(), 3);
    assert!(!jobs.to_string().contains("path"));
    assert!(!jobs.to_string().contains("cookie"));
    f.running.state.shutdown.cancel();
    assert_eq!(f.engine.snapshot().await.unwrap().tracks.len(), 3);
    assert!(!f.engine.cancellation().is_cancelled());
}
