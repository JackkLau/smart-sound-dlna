use super::*;

const HEX: &str = "0000000000000000000000000000000000000000";
fn uri() -> String {
    format!("magnet:?xt=urn:btih:{HEX}&dn=测试文件")
}
fn record() -> Record {
    Record {
        uri: uri(),
        gid: Some("metadata".into()),
        job: Job {
            id: HEX.into(),
            title: "测试文件".into(),
            phase: "resolving".into(),
            downloaded: 0,
            total: 0,
            speed: 0,
            connections: 0,
            error: String::new(),
        },
    }
}

#[test]
fn canonical_hashes_and_tracker_encoding() {
    let (id, title, canonical) = parse_magnet(&format!(
        "magnet:?xt=urn:btih:{}&dn=%E6%B5%8B%E8%AF%95&tr=https%3A%2F%2Ftracker.example%2Fa%3Fkey%3D1%26x%3D2&xs=file:///etc/passwd",
        "A".repeat(32),
    )).unwrap();
    assert_eq!(id, HEX);
    assert_eq!(title, "测试");
    let url = url::Url::parse(&canonical).unwrap();
    assert_eq!(
        url.query_pairs().find(|(k, _)| k == "tr").unwrap().1,
        "https://tracker.example/a?key=1&x=2"
    );
    assert!(!canonical.contains("passwd"));
    assert_eq!(parse_magnet(&uri()).unwrap().0, id);
    assert_eq!(
        parse_magnet(&uri().replace("urn:btih:", "URN:BTIH:"))
            .unwrap()
            .0,
        id
    );
    // Independent RFC 4648 all-one vector.
    assert_eq!(normalize_hash(&"7".repeat(32)).unwrap(), "f".repeat(40));
}

#[test]
fn rejects_invalid_or_ambiguous_links() {
    for input in [
        "https://example.com", "magnet:?dn=missing", "magnet:?xt=urn:btih:abcd",
        "magnet:?xt=urn:btmh:1220abcd", "magnet://host?xt=urn:btih:abcd",
        "magnet:?xt=urn:btih:0000000000000000000000000000000000000000&tr=file:///tmp/tracker",
        "magnet:?xt=urn:btih:0000000000000000000000000000000000000000&xt=urn:btih:1111111111111111111111111111111111111111",
    ] { assert!(parse_magnet(input).is_err(), "accepted {input}"); }
    assert!(parse_magnet(&"x".repeat(8193)).is_err());
    assert!(parse_magnet(&(uri() + "\n--dir=/tmp")).is_err());
}

#[test]
fn metadata_completion_is_not_file_completion() {
    let mut r = record();
    update_record(
        &mut r,
        &json!({"status":"complete", "totalLength":"16000", "completedLength":"16000", "followedBy":["payload"]}),
    );
    assert_eq!(r.gid.as_deref(), Some("payload"));
    assert_eq!(r.job.phase, "resolving");
    assert_eq!(r.job.downloaded, 0);
    update_record(
        &mut r,
        &json!({"status":"active", "totalLength":"1024", "completedLength":"256", "downloadSpeed":"128", "connections":"2", "bittorrent":{"info":{"name":"文件.flac"}}}),
    );
    assert_eq!(
        (
            r.job.phase.as_str(),
            r.job.total,
            r.job.downloaded,
            r.job.speed
        ),
        ("downloading", 1024, 256, 128)
    );
    assert_eq!(r.job.title, "文件.flac");
    update_record(&mut r, &json!({"status":"error", "errorCode":"9"}));
    assert_eq!(r.job.phase, "failed");
    assert_eq!(r.job.speed, 0);
    assert!(r.job.error.contains('9'));
}

#[tokio::test]
async fn persists_records_but_requires_explicit_resume_after_restart() {
    let temp = tempfile::tempdir().unwrap();
    let manager = Downloader::new(temp.path()).unwrap();
    {
        let mut inner = manager.inner.lock().await;
        let mut r = record();
        r.job.phase = "downloading".into();
        r.job.downloaded = 128;
        inner.records.push(r);
        manager.save(&inner).unwrap();
    }
    drop(manager);
    let restored = Downloader::new(temp.path()).unwrap();
    let state = restored.state().await;
    assert_eq!(state.jobs[0].phase, "paused");
    assert_eq!(state.jobs[0].downloaded, 128);
    assert!(restored.inner.lock().await.child.is_none());
    assert!(restored.inner.lock().await.records[0].gid.is_none());
    // Public UI projection must not expose the magnet/Tracker credentials or GID.
    let public = serde_json::to_string(&state).unwrap();
    assert!(!public.contains("magnet:?"));
    assert!(!public.contains("metadata"));
    assert!(!public.contains("secret"));
}

#[tokio::test]
async fn clearing_records_preserves_downloads_and_paused_tasks() {
    let temp = tempfile::tempdir().unwrap();
    let manager = Downloader::new(temp.path()).unwrap();
    let file = manager.root.join("retained.flac");
    std::fs::write(&file, b"retained audio").unwrap();
    {
        let mut inner = manager.inner.lock().await;
        for phase in ["completed", "stopped", "paused", "failed"] {
            let mut r = record();
            r.gid = None;
            r.job.phase = phase.into();
            inner.records.push(r);
        }
    }
    manager.clear_finished().await.unwrap();
    assert_eq!(
        manager
            .state()
            .await
            .jobs
            .iter()
            .map(|j| j.phase.as_str())
            .collect::<Vec<_>>(),
        ["paused", "failed"]
    );
    assert_eq!(std::fs::read(file).unwrap(), b"retained audio");
}

// Exercise the real JSON-RPC boundary over loopback, including authentication,
// duplicate protection, pause/resume/stop and uncertain add responses.
#[cfg(unix)]
#[tokio::test]
async fn rpc_download_controls_and_duplicate_protection() {
    use axum::{extract::State, routing::post, Json, Router};
    use std::{collections::HashMap, sync::Arc};
    #[derive(Default)]
    struct Fixture {
        tasks: HashMap<String, Value>,
        calls: Vec<Value>,
        lose_add_response: bool,
    }
    async fn handler(
        State(state): State<Arc<Mutex<Fixture>>>,
        Json(request): Json<Value>,
    ) -> Json<Value> {
        let mut fixture = state.lock().await;
        fixture.calls.push(request.clone());
        let params = request["params"].as_array().unwrap();
        let method = request["method"].as_str().unwrap();
        if method != "system.multicall" {
            assert_eq!(params[0], "token:test-secret");
        }
        let result = match method {
            "aria2.addUri" => {
                let gid = params[2]["gid"].as_str().unwrap().to_string();
                assert!(params[1][0].as_str().unwrap().starts_with("magnet:?"));
                assert_eq!(params[2]["seed-time"], "0");
                fixture.tasks.insert(
                    gid.clone(),
                    json!({"status":"active","totalLength":"0","completedLength":"0"}),
                );
                json!(gid)
            }
            "system.multicall" => json!(params[0]
                .as_array()
                .unwrap()
                .iter()
                .map(|call| {
                    assert_eq!(call["params"][0], "token:test-secret");
                    fixture
                        .tasks
                        .get(call["params"][1].as_str().unwrap())
                        .map(|s| json!([s]))
                        .unwrap_or_else(|| json!({"code":1,"message":"not found"}))
                })
                .collect::<Vec<_>>()),
            "aria2.forcePause" | "aria2.unpause" | "aria2.forceRemove" => {
                let gid = params[1].as_str().unwrap();
                fixture.tasks.get_mut(gid).unwrap()["status"] = json!(match method {
                    "aria2.forcePause" => "paused",
                    "aria2.unpause" => "active",
                    _ => "removed",
                });
                json!(gid)
            }
            "aria2.removeDownloadResult" => {
                fixture.tasks.remove(params[1].as_str().unwrap());
                json!("OK")
            }
            _ => panic!("unexpected method {method}"),
        };
        if method == "aria2.addUri" && fixture.lose_add_response {
            fixture.lose_add_response = false;
            return Json(json!({"jsonrpc":"2.0","id":"tv-dlna"}));
        }
        Json(json!({"jsonrpc":"2.0","id":"tv-dlna","result":result}))
    }
    let fixture = Arc::new(Mutex::new(Fixture::default()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/jsonrpc", listener.local_addr().unwrap());
    let router = Router::new()
        .route("/jsonrpc", post(handler))
        .with_state(fixture.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let temp = tempfile::tempdir().unwrap();
    let manager = Downloader::new(temp.path()).unwrap();
    {
        let mut inner = manager.inner.lock().await;
        inner.endpoint = endpoint;
        inner.secret = "test-secret".into();
        // A benign child supplies liveness; network behavior is the real local RPC.
        inner.child = Some(Command::new("/bin/sleep").arg("30").spawn().unwrap());
    }
    let job = manager.add(&uri()).await.unwrap();
    assert!(manager
        .add(&uri())
        .await
        .unwrap_err()
        .contains("已在任务列表"));
    manager.control(&job.id, "pause").await.unwrap();
    assert_eq!(manager.state().await.jobs[0].phase, "paused");
    manager.control(&job.id, "resume").await.unwrap();
    assert_eq!(manager.state().await.jobs[0].phase, "resolving");
    manager.control(&job.id, "stop").await.unwrap();
    assert_eq!(manager.state().await.jobs[0].phase, "stopped");
    manager.control(&job.id, "resume").await.unwrap();
    assert_eq!(manager.state().await.jobs[0].phase, "resolving");
    assert!(manager.control(&job.id, "delete-files").await.is_err());
    manager.control(&job.id, "stop").await.unwrap();
    manager.clear_finished().await.unwrap();
    fixture.lock().await.lose_add_response = true;
    assert!(manager.add(&uri()).await.is_err());
    // Refresh recovers an accepted request whose response was lost. Resume must
    // not submit another addUri or download the resource twice.
    manager.control(HEX, "resume").await.unwrap();
    {
        let mut fixture = fixture.lock().await;
        for status in fixture.tasks.values_mut() {
            status["status"] = json!("error");
            status["errorCode"] = json!("9");
        }
    }
    manager.control(HEX, "stop").await.unwrap();
    assert_eq!(manager.state().await.jobs[0].phase, "stopped");
    manager.clear_finished().await.unwrap();
    assert!(manager.state().await.jobs.is_empty());
    let calls = &fixture.lock().await.calls;
    assert_eq!(
        calls
            .iter()
            .filter(|c| c["method"] == "aria2.addUri")
            .count(),
        3
    );
    server.abort();
}
