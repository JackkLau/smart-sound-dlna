//! The official page runs in an ephemeral, capability-free webview when its
//! public share HTML has no metadata. No cookies or account sessions are exported.
use crate::douyin::{self, Video};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tauri::Manager;

pub async fn read(
    app: &tauri::AppHandle,
    id: &str,
    shutdown: tokio_util::sync::CancellationToken,
) -> Result<Video, String> {
    let nonce = uuid::Uuid::new_v4().to_string();
    let label = format!("douyin-preview-{nonce}");
    let callback_path = format!("/{nonce}");
    let id_json = serde_json::to_string(id).map_err(|_| "视频编号无效")?;
    let script = include_str!("douyin_webview.js")
        .replace("__EXPECTED_ID__", &id_json)
        .replace(
            "__CALLBACK_PATH__",
            &serde_json::to_string(&callback_path).unwrap(),
        );
    let (send, receive) = tokio::sync::oneshot::channel::<Result<Video, String>>();
    let send = Arc::new(Mutex::new(Some(send)));
    let navigation_send = send.clone();
    let expected = id.to_owned();
    let window = tauri::WebviewWindowBuilder::new(
        app,
        &label,
        tauri::WebviewUrl::External(
            format!("https://www.douyin.com/video/{id}")
                .parse()
                .map_err(|_| "视频地址无效")?,
        ),
    )
    .title("抖音视频解析 · 加载成功后自动关闭")
    .inner_size(960.0, 720.0)
    .incognito(true)
    .initialization_script(&script)
    .on_navigation(move |url| {
        if url.host_str() == Some("douyin-import.invalid") {
            if url.scheme() == "https"
                && url.path() == callback_path
                && url.as_str().len() <= 100_000
            {
                let data = url
                    .query_pairs()
                    .find(|(k, _)| k == "data")
                    .map(|(_, v)| v.into_owned());
                let result = data
                    .ok_or_else(|| "视频信息缺失".to_owned())
                    .and_then(|raw| {
                        serde_json::from_str::<Video>(&raw)
                            .map_err(|_| "视频信息格式无效".to_owned())
                    })
                    .and_then(|video| video.validate(&expected));
                if let Some(tx) = navigation_send
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .take()
                {
                    let _ = tx.send(result);
                }
            }
            return false;
        }
        douyin::page_url(url.as_str()).is_ok()
    })
    .build()
    .map_err(|_| "无法打开抖音官方解析窗口，请重试。")?;
    window.on_window_event(move |event| {
        if matches!(event, tauri::WindowEvent::Destroyed) {
            if let Some(tx) = send.lock().unwrap_or_else(|e| e.into_inner()).take() {
                let _ = tx.send(Err("已关闭抖音解析窗口，可以重新解析。".into()));
            }
        }
    });
    let result = tokio::select! {
        _ = shutdown.cancelled() => Err("客户端正在退出".into()),
        result = tokio::time::timeout(Duration::from_secs(120), receive) => match result {
            Ok(Ok(result)) => result,
            _ => Err("抖音解析超时。请确认该视频可公开播放；若官网要求登录或验证，请在官网完成后重试。".into()),
        },
    };
    let _ = window.close();
    if let Some(main) = app.get_webview_window("main") {
        let _ = main.set_focus();
    }
    result
}
