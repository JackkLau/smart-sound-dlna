use crate::{
    engine::{Engine, Snapshot},
    library::{self, ImportSummary},
};
use serde::Serialize;
use serde_json::Value;
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use tauri::{Manager, State};
use tauri_plugin_dialog::DialogExt;

struct AppState {
    engine: Arc<Engine>,
    picker_busy: Arc<AtomicBool>,
}

#[tauri::command]
async fn get_state(state: State<'_, AppState>) -> Result<Snapshot, String> {
    state.engine.snapshot().await
}

#[tauri::command]
async fn discover_devices(state: State<'_, AppState>) -> Result<Snapshot, String> {
    state.engine.discover().await
}

#[tauri::command]
async fn add_device(url: String, state: State<'_, AppState>) -> Result<Snapshot, String> {
    state.engine.add_device(&url).await
}

#[tauri::command]
async fn select_device(device_id: String, state: State<'_, AppState>) -> Result<Snapshot, String> {
    state.engine.select(&device_id).await
}

#[tauri::command]
async fn play_track(track_id: String, state: State<'_, AppState>) -> Result<Snapshot, String> {
    state.engine.play(&track_id).await
}

#[tauri::command]
async fn control(
    action: String,
    value: Option<Value>,
    state: State<'_, AppState>,
) -> Result<Snapshot, String> {
    state
        .engine
        .control(&action, value.unwrap_or(Value::Null))
        .await
}

#[derive(Serialize)]
struct Config {
    formats: Vec<String>,
    max_tracks: usize,
    platform: &'static str,
}

#[tauri::command]
fn get_config() -> Config {
    Config {
        formats: library::formats(),
        max_tracks: 20_000,
        platform: std::env::consts::OS,
    }
}

struct PickerGuard(Arc<AtomicBool>);
impl Drop for PickerGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

async fn pick(
    app: tauri::AppHandle,
    state: &AppState,
    folder: bool,
) -> Result<Option<ImportSummary>, String> {
    if state.picker_busy.swap(true, Ordering::AcqRel) {
        return Err("请先完成已打开的文件选择窗口。".into());
    }
    let guard = PickerGuard(state.picker_busy.clone());
    let paths =
        tauri::async_runtime::spawn_blocking(move || -> Result<Option<Vec<PathBuf>>, String> {
            let _guard = guard;
            let dialog = app.dialog().file();
            let selected = if folder {
                dialog
                    .set_title("选择音乐文件夹")
                    .blocking_pick_folder()
                    .map(|path| vec![path])
            } else {
                let formats = library::formats();
                let extensions: Vec<&str> = formats.iter().map(String::as_str).collect();
                dialog
                    .set_title("选择本地音乐")
                    .add_filter("音乐文件", &extensions)
                    .blocking_pick_files()
            };
            selected
                .map(|files| {
                    files
                        .into_iter()
                        .map(|file| {
                            file.into_path()
                                .map_err(|e| format!("文件路径无法读取：{e}"))
                        })
                        .collect()
                })
                .transpose()
        })
        .await
        .map_err(|e| e.to_string())??;
    match paths {
        Some(paths) => state.engine.import(paths).await.map(Some),
        None => Ok(None),
    }
}

#[tauri::command]
async fn pick_music_files(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<ImportSummary>, String> {
    pick(app, &state, false).await
}

#[tauri::command]
async fn pick_music_folder(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<ImportSummary>, String> {
    pick(app, &state, true).await
}

pub fn run() {
    let handler: fn(tauri::ipc::Invoke<tauri::Wry>) -> bool = tauri::generate_handler![
        get_state,
        get_config,
        discover_devices,
        add_device,
        select_device,
        play_track,
        control,
        pick_music_files,
        pick_music_folder,
        get_features,
        parse_bilibili,
        download_bilibili,
        cancel_download,
        retry_download,
        open_provider_login,
        finish_provider_login,
        connect_voice_session,
        disconnect_provider,
        enable_voice,
        disable_voice,
        voice_command
    ];
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            let engine = tauri::async_runtime::block_on(Engine::new(data_dir))
                .map_err(std::io::Error::other)?;
            let running = engine.clone();
            // start() calls tokio::spawn, so enter Tauri's async runtime first.
            tauri::async_runtime::spawn(async move {
                running.start();
            });
            app.manage(AppState {
                engine,
                picker_busy: Arc::new(AtomicBool::new(false)),
            });
            Ok(())
        })
        .invoke_handler(move |invoke| {
            if invoke.message.webview().label() != "main" {
                invoke
                    .resolver
                    .reject("登录窗口不能访问本地音乐或控制音响。");
                return true;
            }
            handler(invoke)
        })
        .build(tauri::generate_context!())
        .expect("无法启动小米音乐投送");
    app.run(|app, event| {
        if matches!(event, tauri::RunEvent::Exit) {
            if let Some(state) = app.try_state::<AppState>() {
                state.engine.shutdown();
            }
        }
    });
}

#[derive(Serialize)]
struct Features {
    voice: crate::voice::VoiceState,
    downloads: crate::bilibili::DownloadState,
    selected_device_id: Option<String>,
    selected_device_name: Option<String>,
}
#[tauri::command]
async fn get_features(state: State<'_, AppState>) -> Result<Features, String> {
    let snapshot = state.engine.snapshot().await?;
    let name = snapshot
        .devices
        .iter()
        .find(|d| Some(&d.id) == snapshot.selected_device_id.as_ref())
        .map(|d| d.name.clone());
    Ok(Features {
        voice: state.engine.voice.state().await,
        downloads: state.engine.downloads.state().await,
        selected_device_id: snapshot.selected_device_id,
        selected_device_name: name,
    })
}
#[tauri::command]
async fn parse_bilibili(
    url: String,
    state: State<'_, AppState>,
) -> Result<crate::bilibili::Preview, String> {
    state.engine.downloads.parse(&url).await
}
#[tauri::command]
async fn download_bilibili(
    preview_id: String,
    cid: u64,
    state: State<'_, AppState>,
) -> Result<crate::bilibili::DownloadJob, String> {
    state
        .engine
        .downloads
        .start(
            Arc::downgrade(&state.engine),
            &preview_id,
            cid,
            state.engine.cancellation(),
        )
        .await
}
#[tauri::command]
async fn cancel_download(id: String, state: State<'_, AppState>) -> Result<(), String> {
    state.engine.downloads.cancel(&id).await
}
#[tauri::command]
async fn retry_download(
    id: String,
    state: State<'_, AppState>,
) -> Result<crate::bilibili::DownloadJob, String> {
    state
        .engine
        .downloads
        .retry(
            Arc::downgrade(&state.engine),
            &id,
            state.engine.cancellation(),
        )
        .await
}

fn provider_info(provider: &str) -> Result<(&'static str, &'static str, &'static str), String> {
    match provider {
        "xiaomi" => Ok((
            "xiaomi-login",
            "小米账号登录",
            "https://account.xiaomi.com/pass/serviceLogin?sid=micoapi&_locale=zh_CN",
        )),
        "bilibili" => Ok((
            "bilibili-login",
            "Bilibili 登录",
            "https://passport.bilibili.com/login",
        )),
        _ => Err("不支持的登录服务".into()),
    }
}
fn login_host(provider: &str, url: &url::Url) -> bool {
    if url.scheme() != "https" {
        return false;
    }
    let host = url.host_str().unwrap_or_default();
    let domains: &[&str] = if provider == "xiaomi" {
        &["xiaomi.com", "mi.com", "miui.com"]
    } else {
        &["bilibili.com"]
    };
    domains
        .iter()
        .any(|d| host == *d || host.ends_with(&format!(".{d}")))
}
#[tauri::command]
async fn open_provider_login(provider: String, app: tauri::AppHandle) -> Result<(), String> {
    let (label, title, url) = provider_info(&provider)?;
    if let Some(window) = app.get_webview_window(label) {
        window.show().map_err(|e| e.to_string())?;
        return window.set_focus().map_err(|e| e.to_string());
    }
    tauri::WebviewWindowBuilder::new(
        &app,
        label,
        tauri::WebviewUrl::External(url.parse().map_err(|_| "登录地址无效")?),
    )
    .title(title)
    .inner_size(500.0, 720.0)
    .min_inner_size(400.0, 520.0)
    .incognito(true)
    .disable_drag_drop_handler()
    .on_navigation(move |url| login_host(&provider, url))
    .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
    .build()
    .map_err(|e| format!("无法打开登录窗口：{e}"))?;
    Ok(())
}
#[tauri::command]
async fn finish_provider_login(
    provider: String,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let (label, _, _) = provider_info(&provider)?;
    let window = app
        .get_webview_window(label)
        .ok_or("登录窗口已关闭，请重新打开并完成登录。")?;
    let read_window = window.clone();
    let read_provider = provider.clone();
    // WKWebView/WebView2 cookie APIs dispatch onto their UI thread themselves.
    let raw = tauri::async_runtime::spawn_blocking(move || -> Result<String, String> {
        let url = if read_provider == "xiaomi" {
            "https://api2.mina.mi.com/"
        } else {
            "https://www.bilibili.com/"
        };
        let mut cookies = read_window
            .cookies_for_url(url.parse().map_err(|_| "登录地址无效")?)
            .map_err(|_| "无法读取登录会话，请重试。")?;
        // The account cookie can be scoped to xiaomi.com while the service
        // token is scoped to mina.mi.com. Never take a token from another service.
        if read_provider == "xiaomi" && !cookies.iter().any(|c| c.name() == "userId") {
            let account = read_window
                .cookies_for_url(
                    "https://account.xiaomi.com/"
                        .parse()
                        .map_err(|_| "登录地址无效")?,
                )
                .map_err(|_| "无法读取小米账号信息，请重新登录。")?;
            cookies.extend(account.into_iter().filter(|c| c.name() == "userId"));
        }
        let names: &[&str] = if read_provider == "xiaomi" {
            &["userId", "serviceToken"]
        } else {
            &["SESSDATA", "bili_jct", "buvid3", "buvid4", "DedeUserID"]
        };
        Ok(cookies
            .into_iter()
            .filter(|c| names.contains(&c.name()))
            .map(|c| format!("{}={}", c.name(), c.value()))
            .collect::<Vec<_>>()
            .join("; "))
    })
    .await
    .map_err(|_| "无法读取登录会话")??;
    if provider == "xiaomi" {
        state.engine.voice.connect(&raw).await?;
    } else {
        state.engine.downloads.set_session(&raw).await?;
    }
    let _ = window.close();
    Ok(())
}
#[tauri::command]
async fn connect_voice_session(
    cookie: String,
    state: State<'_, AppState>,
) -> Result<crate::voice::VoiceState, String> {
    state.engine.voice.connect(&cookie).await
}
#[tauri::command]
async fn disconnect_provider(
    provider: String,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let (label, _, _) = provider_info(&provider)?;
    if provider == "xiaomi" {
        state.engine.voice.disable(true).await;
    } else {
        state.engine.downloads.disconnect().await;
    }
    if let Some(window) = app.get_webview_window(label) {
        let _ = window.close();
    }
    Ok(())
}
#[tauri::command]
async fn enable_voice(
    source_id: String,
    target_id: String,
    state: State<'_, AppState>,
) -> Result<crate::voice::VoiceState, String> {
    state
        .engine
        .voice
        .enable(
            Arc::downgrade(&state.engine),
            &source_id,
            &target_id,
            state.engine.cancellation(),
        )
        .await
}
#[tauri::command]
async fn disable_voice(state: State<'_, AppState>) -> Result<crate::voice::VoiceState, String> {
    Ok(state.engine.voice.disable(false).await)
}
#[tauri::command]
async fn voice_command(text: String, state: State<'_, AppState>) -> Result<String, String> {
    state.engine.voice_command(&text, None).await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn login_navigation_stays_inside_provider_domains() {
        assert!(login_host(
            "xiaomi",
            &"https://account.xiaomi.com/pass/serviceLogin"
                .parse()
                .unwrap()
        ));
        assert!(login_host(
            "xiaomi",
            &"https://api2.mina.mi.com/sts".parse().unwrap()
        ));
        assert!(login_host(
            "bilibili",
            &"https://passport.bilibili.com/login".parse().unwrap()
        ));
        for address in [
            "http://account.xiaomi.com/",
            "https://account.xiaomi.com.evil.test/",
            "https://example.com/",
            "tauri://localhost",
        ] {
            assert!(!login_host("xiaomi", &address.parse().unwrap()));
        }
        assert!(!login_host(
            "bilibili",
            &"https://account.xiaomi.com/".parse().unwrap()
        ));
    }
}
