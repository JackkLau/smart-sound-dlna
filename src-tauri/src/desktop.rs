use crate::{
    engine::{Engine, Snapshot},
    library::{self, ImportSummary},
    login::{self, Attempt, LoginFlow, LoginStatus, Phase},
    session_cache::SessionCache,
};
use serde::Serialize;
use serde_json::Value;
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tauri::{Manager, State};
use tauri_plugin_dialog::DialogExt;

struct AppState {
    engine: Arc<Engine>,
    mobile: crate::mobile::Receiver,
    picker_busy: Arc<AtomicBool>,
    xiaomi_login: Arc<LoginFlow>,
    bilibili_login: Arc<LoginFlow>,
    sessions: Arc<SessionCache>,
}

impl AppState {
    fn login(&self, provider: &str) -> Result<Arc<LoginFlow>, String> {
        match provider {
            "xiaomi" => Ok(self.xiaomi_login.clone()),
            "bilibili" => Ok(self.bilibili_login.clone()),
            _ => Err("不支持的登录服务".into()),
        }
    }
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
async fn rename_track(
    track_id: String,
    title: String,
    expected_title: String,
    state: State<'_, AppState>,
) -> Result<Snapshot, String> {
    state
        .engine
        .rename_track(track_id, title, expected_title)
        .await
}

#[tauri::command]
async fn remove_track(track_id: String, state: State<'_, AppState>) -> Result<Snapshot, String> {
    state.engine.remove_track(track_id).await
}

#[tauri::command]
async fn recognize_track_name(
    track_id: String,
    state: State<'_, AppState>,
) -> Result<crate::track_names::Suggestion, String> {
    state.engine.recognize_track_name(track_id).await
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
        rename_track,
        remove_track,
        recognize_track_name,
        control,
        pick_music_files,
        pick_music_folder,
        get_features,
        get_mobile_receiver,
        set_mobile_receiver,
        forget_mobile_pairing,
        parse_bilibili,
        parse_video,
        download_bilibili,
        list_bilibili_folders,
        read_bilibili_favorite,
        download_bilibili_batch,
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
            let mobile = crate::mobile::Receiver::new(data_dir.clone());
            let engine = tauri::async_runtime::block_on(Engine::new(data_dir))
                .map_err(std::io::Error::other)?;
            let running = engine.clone();
            // start() calls tokio::spawn, so enter Tauri's async runtime first.
            tauri::async_runtime::spawn(async move {
                running.start();
            });
            let xiaomi_login = Arc::new(LoginFlow::default());
            let bilibili_login = Arc::new(LoginFlow::default());
            let sessions = Arc::new(SessionCache::default());
            for (provider, flow) in [
                ("xiaomi", xiaomi_login.clone()),
                ("bilibili", bilibili_login.clone()),
            ] {
                let attempt = flow.begin();
                flow.update(&attempt, Phase::Checking, None);
                let engine = engine.clone();
                let sessions = sessions.clone();
                tauri::async_runtime::spawn(async move {
                    let _guard = flow.gate.lock().await;
                    let _ = restore_provider(&engine, &flow, &attempt, &sessions, provider).await;
                });
            }
            app.manage(AppState {
                engine,
                mobile,
                picker_busy: Arc::new(AtomicBool::new(false)),
                xiaomi_login,
                bilibili_login,
                sessions,
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
        .build(crate::desktop_context())
        .expect("无法启动小米音乐投送");
    app.run(|app, event| {
        if matches!(event, tauri::RunEvent::Exit) {
            if let Some(state) = app.try_state::<AppState>() {
                state.xiaomi_login.reset();
                state.bilibili_login.reset();
                state.engine.shutdown();
            }
        }
    });
}

#[tauri::command]
async fn get_mobile_receiver(
    state: State<'_, AppState>,
) -> Result<crate::mobile::ReceiverStatus, String> {
    Ok(state.mobile.status().await)
}

#[tauri::command]
async fn set_mobile_receiver(
    enabled: bool,
    state: State<'_, AppState>,
) -> Result<crate::mobile::ReceiverStatus, String> {
    if enabled {
        state.mobile.enable(&state.engine).await
    } else {
        Ok(state.mobile.disable().await)
    }
}

#[tauri::command]
async fn forget_mobile_pairing(
    state: State<'_, AppState>,
) -> Result<crate::mobile::ReceiverStatus, String> {
    state.mobile.forget().await?;
    state.mobile.enable(&state.engine).await
}

#[derive(Serialize)]
struct Features {
    voice: crate::voice::VoiceState,
    xiaomi_login: LoginStatus,
    bilibili_login: LoginStatus,
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
        xiaomi_login: state.xiaomi_login.status(),
        bilibili_login: state.bilibili_login.status(),
        downloads: state.engine.downloads.state().await,
        selected_device_id: snapshot.selected_device_id,
        selected_device_name: name,
    })
}
#[tauri::command]
async fn parse_video(
    url: String,
    platform: String,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<crate::bilibili::Preview, String> {
    match platform.as_str() {
        "bilibili" => {
            state
                .engine
                .downloads
                .parse(&crate::bilibili::shared_video_input(&url)?)
                .await
        }
        "douyin" => {
            let id = crate::douyin::resolve_input(&url).await?;
            let video = match crate::douyin::fetch(&id).await {
                Ok(video) => video,
                Err(_) => {
                    crate::douyin_webview::read(&app, &id, state.engine.cancellation()).await?
                }
            };
            state.engine.downloads.register_douyin(video).await
        }
        _ => Err("请选择支持的视频平台。".into()),
    }
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
async fn list_bilibili_folders(
    state: State<'_, AppState>,
) -> Result<Vec<crate::bilibili::FavoriteFolder>, String> {
    state.engine.downloads.favorite_folders().await
}
#[tauri::command]
async fn read_bilibili_favorite(
    url: String,
    page: u32,
    state: State<'_, AppState>,
) -> Result<crate::bilibili::FavoritePage, String> {
    state.engine.downloads.favorite_page(&url, page).await
}
#[tauri::command]
async fn download_bilibili_batch(
    selections: Vec<crate::bilibili::BatchSelection>,
    state: State<'_, AppState>,
) -> Result<crate::bilibili::BatchResult, String> {
    state
        .engine
        .downloads
        .start_batch(
            Arc::downgrade(&state.engine),
            selections,
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
// Read the ephemeral webview's native cookie store off the UI thread. Reading
// document.cookie would miss HttpOnly credentials; cookies_for_url in Wry 0.57
// also drops parent-domain cookies on macOS.
enum LoginCredential {
    Service(String),
    XiaomiAccount(String),
}

async fn read_login_cookie(
    window: tauri::WebviewWindow,
    provider: String,
    page: Arc<crate::login_webview::LoginPage>,
) -> Result<Option<LoginCredential>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let observation = page.observe(&window)?;
        let cookies = observation.cookies;
        if let Some(cookie) = login::session_cookie(&provider, &cookies) {
            return Ok(Some(LoginCredential::Service(cookie)));
        }
        if provider == "xiaomi" {
            if let Some(account) = crate::xiaomi_login::account_cookie(&cookies) {
                return Ok(Some(LoginCredential::XiaomiAccount(account)));
            }
            if observation.service_callback {
                return Err(
                    "已返回小爱登录页，但未取得小米账号授权会话。请重新打开登录窗口后重试。".into(),
                );
            }
        }
        Ok(None)
    })
    .await
    .map_err(|_| "无法读取登录会话，请重试。".to_owned())?
}

async fn clear_provider(engine: &Engine, provider: &str) {
    if provider == "xiaomi" {
        engine.voice.disable(true).await;
    } else {
        engine.downloads.disconnect().await;
    }
}

async fn restore_provider(
    engine: &Engine,
    flow: &LoginFlow,
    attempt: &Attempt,
    sessions: &SessionCache,
    provider: &str,
) -> Result<bool, String> {
    let restored = flow
        .restore_cached(attempt, sessions, provider, |raw| async move {
            if provider == "xiaomi" {
                engine.voice.connect(&raw).await.map(|_| ())
            } else {
                engine.downloads.set_session(&raw).await
            }
        })
        .await?;
    match restored {
        Some(login::Completion::Connected) => Ok(true),
        Some(login::Completion::Cancelled {
            clear_session: true,
        }) => {
            clear_provider(engine, provider).await;
            Ok(false)
        }
        _ => Ok(false),
    }
}

// Caller holds the provider gate. Automatic and manual completion use the same
// path; only a server-validated session may close the login window.
async fn complete_login(
    app: &tauri::AppHandle,
    engine: &Engine,
    provider: &str,
    flow: &LoginFlow,
    attempt: &Attempt,
    sessions: &SessionCache,
) -> Result<bool, String> {
    if attempt.cancel.is_cancelled() {
        return Ok(false);
    }
    let (label, _, _) = provider_info(provider)?;
    let Some(window) = app.get_webview_window(label) else {
        flow.window_closed(attempt);
        return Ok(false);
    };
    let read = tokio::select! {
        biased;
        _ = attempt.cancel.cancelled() => return Ok(false),
        result = tokio::time::timeout(Duration::from_secs(10), read_login_cookie(window.clone(), provider.to_owned(), attempt.page.clone())) => result.map_err(|_| "读取登录会话超时，请重试。".to_owned())?,
    };
    let Some(credential) = read? else {
        return Ok(false);
    };
    flow.update(attempt, Phase::Checking, None);
    let raw = tokio::select! {
        _ = attempt.cancel.cancelled() => return Ok(false),
        raw = async { Ok::<String, String>(match credential {
            LoginCredential::Service(cookie) => cookie,
            LoginCredential::XiaomiAccount(account) => {
                crate::xiaomi_login::exchange(&account).await?
            }
        }) } => raw?,
    };
    let mut validated = false;
    let validation = async {
        if provider == "xiaomi" {
            engine.voice.connect(&raw).await?;
        } else {
            engine.downloads.set_session(&raw).await?;
        }
        validated = true;
        Ok(())
    };
    let result = flow
        .complete_cached(attempt, sessions, provider, &raw, validation, || {
            let closed = window
                .close()
                .map_err(|_| "账号已连接，请手动关闭登录窗口。".to_owned());
            if let Some(main) = app.get_webview_window("main") {
                let _ = main.set_focus();
            }
            closed
        })
        .await;
    if result.is_err() && validated {
        clear_provider(engine, provider).await;
    }
    let result = result?;
    match result {
        login::Completion::Connected => Ok(true),
        login::Completion::Cancelled { clear_session } => {
            if clear_session {
                clear_provider(engine, provider).await;
            }
            Ok(false)
        }
    }
}

fn monitor_login(
    app: tauri::AppHandle,
    engine: Arc<Engine>,
    provider: String,
    flow: Arc<LoginFlow>,
    attempt: Attempt,
    sessions: Arc<SessionCache>,
) {
    tauri::async_runtime::spawn(async move {
        let started = tokio::time::Instant::now();
        let deadline = started + Duration::from_secs(600);
        let shutdown = engine.cancellation();
        loop {
            tokio::select! {
                biased;
                _ = shutdown.cancelled() => break,
                _ = attempt.cancel.cancelled() => break,
                _ = tokio::time::sleep(Duration::from_millis(750)) => {}
            }
            let _guard = flow.gate.lock().await;
            if attempt.cancel.is_cancelled() || flow.status().phase == Phase::Connected {
                break;
            }
            let result = if tokio::time::Instant::now() >= deadline {
                Err("登录等待超时，请重新打开登录窗口后重试。".into())
            } else {
                match complete_login(&app, &engine, &provider, &flow, &attempt, &sessions).await {
                    Ok(false) => attempt
                        .page
                        .load_error(started.elapsed())
                        .map_or(Ok(false), |error| Err(error.to_owned())),
                    result => result,
                }
            };
            match result {
                Ok(true) => break,
                Ok(false) => {}
                Err(error) => {
                    flow.update(&attempt, Phase::Failed, Some(error));
                    // Surface errors in the main dialog; keep the original
                    // login webview alive so a manual retry retains its cookies.
                    if let Some(main) = app.get_webview_window("main") {
                        let _ = main.set_focus();
                    }
                    break;
                }
            }
        }
    });
}

#[tauri::command]
async fn open_provider_login(
    provider: String,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let (label, title, url) = provider_info(&provider)?;
    let flow = state.login(&provider)?;
    let _guard = flow.gate.lock().await;
    if let Some(window) = app.get_webview_window(label) {
        window.show().map_err(|e| e.to_string())?;
        window.set_focus().map_err(|e| e.to_string())?;
        if matches!(flow.status().phase, Phase::Waiting | Phase::Checking) {
            return Ok(());
        }
        // Resume monitoring a failed attempt without discarding its session.
        let attempt = flow.retry_in_window();
        watch_login_close(&window, flow.clone(), attempt.clone());
        if !attempt.page.has_loaded()
            && window
                .navigate(url.parse().map_err(|_| "登录地址无效")?)
                .is_err()
        {
            let error = "无法重新加载登录网页，请关闭登录窗口后重试。".to_owned();
            flow.update(&attempt, Phase::Failed, Some(error.clone()));
            return Err(error);
        }
        monitor_login(
            app,
            state.engine.clone(),
            provider,
            flow.clone(),
            attempt,
            state.sessions.clone(),
        );
        return Ok(());
    }
    let attempt = flow.begin();
    let nav_provider = provider.clone();
    let load_provider = provider.clone();
    let load_page = attempt.page.clone();
    let result = tauri::WebviewWindowBuilder::new(
        &app,
        label,
        tauri::WebviewUrl::External(url.parse().map_err(|_| "登录地址无效")?),
    )
    .title(title)
    .inner_size(500.0, 720.0)
    .min_inner_size(400.0, 520.0)
    .incognito(true)
    .disable_drag_drop_handler()
    .on_navigation(move |url| login_host(&nav_provider, url))
    .on_page_load(move |_, payload| {
        if payload.event() == tauri::webview::PageLoadEvent::Finished
            && login_host(&load_provider, payload.url())
        {
            // Use the callback after navigation finishes, so pending redirects
            // and cookies are not mistaken for a failed login.
            load_page
                .loaded(load_provider == "xiaomi" && crate::xiaomi_login::is_sts(payload.url()));
        }
    })
    .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
    .build();
    match result {
        Ok(window) => {
            watch_login_close(&window, flow.clone(), attempt.clone());
            monitor_login(
                app,
                state.engine.clone(),
                provider,
                flow.clone(),
                attempt,
                state.sessions.clone(),
            );
            Ok(())
        }
        Err(_) => {
            let error = "无法打开登录窗口，请重试。".to_owned();
            flow.update(&attempt, Phase::Failed, Some(error.clone()));
            Err(error)
        }
    }
}

fn watch_login_close(window: &tauri::WebviewWindow, flow: Arc<LoginFlow>, attempt: Attempt) {
    window.on_window_event(move |event| {
        if matches!(event, tauri::WindowEvent::Destroyed) {
            flow.window_closed(&attempt);
        }
    });
}

#[tauri::command]
async fn finish_provider_login(
    provider: String,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let flow = state.login(&provider)?;
    let _guard = flow.gate.lock().await;
    if flow.status().phase == Phase::Connected {
        return Ok(());
    }
    let (label, _, _) = provider_info(&provider)?;
    if app.get_webview_window(label).is_none() {
        let attempt = flow.begin();
        return if restore_provider(&state.engine, &flow, &attempt, &state.sessions, &provider)
            .await?
        {
            Ok(())
        } else {
            Err("没有可恢复的登录状态，请打开登录窗口。".into())
        };
    }
    let attempt = flow.current().ok_or("请先打开登录窗口。")?;
    if attempt.cancel.is_cancelled() {
        return Err("登录窗口已关闭，请重新打开并完成登录。".into());
    }
    let result = complete_login(
        &app,
        &state.engine,
        &provider,
        &flow,
        &attempt,
        &state.sessions,
    )
    .await;
    match result {
        Ok(true) => Ok(()),
        Ok(false) => Err("尚未取得完整登录会话，请先在官方窗口完成登录。".into()),
        Err(error) => {
            flow.update(&attempt, Phase::Failed, Some(error.clone()));
            Err(error)
        }
    }
}
#[tauri::command]
async fn connect_voice_session(
    cookie: String,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<crate::voice::VoiceState, String> {
    let flow = state.login("xiaomi")?;
    let _guard = flow.gate.lock().await;
    let result = state.engine.voice.connect(&cookie).await?;
    if let Err(error) = state.sessions.save("xiaomi", &cookie).await {
        state.engine.voice.disable(true).await;
        return Err(error);
    }
    flow.set_cached(true);
    flow.reset();
    if let Some(window) = app.get_webview_window("xiaomi-login") {
        let _ = window.close();
    }
    Ok(result)
}
#[tauri::command]
async fn disconnect_provider(
    provider: String,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let (label, _, _) = provider_info(&provider)?;
    let flow = state.login(&provider)?;
    let _guard = flow.gate.lock().await;
    // If deletion fails, do not claim logout succeeded or restore it silently.
    state.sessions.clear(&provider).await?;
    flow.set_cached(false);
    flow.reset();
    clear_provider(&state.engine, &provider).await;
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
