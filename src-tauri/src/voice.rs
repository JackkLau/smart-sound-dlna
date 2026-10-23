//! XiaoAI cloud conversation adapter and a provider-independent command layer.
//! Session cookies stay in memory; only the explicitly bound speaker is read.
use crate::{engine::Engine, library::Track, online};
use serde::Serialize;
use serde_json::Value;
use std::{
    sync::{Arc, Weak},
    time::Duration,
};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

const UA: &str = "MiHome/6.0.103 (com.xiaomi.mihome; build:6.0.103.1; iOS 14.4.0) Alamofire/6.0.103 MICO/iOSApp/appStore/6.0.103";

#[derive(Clone, Debug, Serialize)]
pub struct VoiceDevice {
    pub id: String,
    pub name: String,
    pub hardware: String,
}
#[derive(Clone, Default, Serialize)]
pub struct VoiceState {
    pub connected: bool,
    pub listening: bool,
    pub devices: Vec<VoiceDevice>,
    pub source_id: Option<String>,
    pub target_id: Option<String>,
    pub last_command: Option<String>,
    pub last_result: Option<String>,
    pub error: Option<String>,
    pub last_poll_ms: Option<u64>,
}
struct Inner {
    cookie: String,
    state: VoiceState,
}
pub struct VoiceService {
    inner: Mutex<Inner>,
    lifecycle: Mutex<Option<(CancellationToken, tokio::task::JoinHandle<()>)>>,
    client: reqwest::Client,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Play(String),
    Control(&'static str, Option<u8>),
    Mode(&'static str),
}

fn normalized(text: &str) -> String {
    text.chars()
        .flat_map(char::to_lowercase)
        .filter(|c| c.is_alphanumeric())
        .collect()
}

pub fn parse(text: &str) -> Result<Command, String> {
    if text.chars().count() > 300 {
        return Err("指令过长，请缩短歌名。".into());
    }
    let text = normalized(text);
    let text = text.strip_prefix("小爱同学").unwrap_or(&text);
    for prefix in ["播放本地音乐", "播放本地歌曲", "播放本地", "本地播放"] {
        if let Some(name) = text.strip_prefix(prefix) {
            return if name.is_empty() {
                Err("请说「播放本地 + 歌名」。".into())
            } else {
                Ok(Command::Play(name.into()))
            };
        }
    }
    match text {
        "本地暂停" | "暂停本地音乐" => Ok(Command::Control("pause", None)),
        "本地继续" | "继续本地音乐" => Ok(Command::Control("resume", None)),
        "本地停止" | "停止本地音乐" => Ok(Command::Control("stop", None)),
        "本地下一首" => Ok(Command::Control("next", None)),
        "本地上一首" => Ok(Command::Control("previous", None)),
        "本地单曲循环" => Ok(Command::Mode("repeat-one")),
        "本地列表循环" => Ok(Command::Mode("repeat-all")),
        "本地顺序播放" => Ok(Command::Mode("sequential")),
        _ => {
            if let Some(number) = text.strip_prefix("本地音量") {
                return chinese_number(number)
                    .map(|value| Command::Control("volume", Some(value)))
                    .ok_or_else(|| "请说「本地音量三十」，音量范围为零到一百。".into());
            }
            Err("未识别本地点播指令。可说「播放本地 + 歌名」或「本地下一首」。".into())
        }
    }
}

fn chinese_number(text: &str) -> Option<u8> {
    if let Ok(value) = text.parse::<u8>() {
        return (value <= 100).then_some(value);
    }
    if text == "一百" {
        return Some(100);
    }
    let digit = |s: &str| match s {
        "零" | "〇" => Some(0),
        "一" => Some(1),
        "二" | "两" => Some(2),
        "三" => Some(3),
        "四" => Some(4),
        "五" => Some(5),
        "六" => Some(6),
        "七" => Some(7),
        "八" => Some(8),
        "九" => Some(9),
        _ => None,
    };
    if let Some((tens, ones)) = text.split_once('十') {
        let tens = if tens.is_empty() { 1 } else { digit(tens)? };
        let ones = if ones.is_empty() { 0 } else { digit(ones)? };
        Some(tens * 10 + ones)
    } else {
        digit(text)
    }
}

pub fn match_track<'a>(query: &str, tracks: &'a [Track]) -> Result<&'a Track, String> {
    let query = normalized(query);
    if query.is_empty() {
        return Err("请补充歌名。".into());
    }
    let exact: Vec<_> = tracks
        .iter()
        .filter(|t| normalized(&t.title) == query)
        .collect();
    let found = if exact.is_empty() {
        tracks
            .iter()
            .filter(|t| normalized(&t.title).contains(&query))
            .collect()
    } else {
        exact
    };
    match found.as_slice() {
        [track] => Ok(track),
        [] => Err("曲库里没有找到这首歌，请先导入，或换一个歌名。".into()),
        _ => Err(format!(
            "找到 {} 首匹配的音乐，请说更完整的歌名。",
            found.len()
        )),
    }
}

impl VoiceService {
    pub fn new() -> Result<Arc<Self>, String> {
        Ok(Arc::new(Self {
            inner: Mutex::new(Inner {
                cookie: String::new(),
                state: VoiceState::default(),
            }),
            lifecycle: Mutex::new(None),
            client: online::client(UA)?,
        }))
    }
    pub async fn state(&self) -> VoiceState {
        self.inner.lock().await.state.clone()
    }

    pub async fn connect(&self, raw_cookie: &str) -> Result<VoiceState, String> {
        // Serialize login/enable/disable, including their network completion.
        let mut lifecycle = self.lifecycle.lock().await;
        let cookie = online::cookies(raw_cookie, &["userId", "serviceToken"])?;
        if !cookie.starts_with("userId=") || !cookie.contains("; serviceToken=") {
            return Err("需要小爱服务的 userId 和 serviceToken，请先完成小米账号登录。".into());
        }
        let response = self
            .client
            .get("https://api2.mina.mi.com/admin/v2/device_list")
            .query(&[
                ("master", "0"),
                (
                    "requestId",
                    &format!("app_ios_{}", uuid::Uuid::new_v4().simple()),
                ),
            ])
            .header(reqwest::header::COOKIE, &cookie)
            .timeout(Duration::from_secs(20))
            .send()
            .await
            .map_err(|_| "小米服务连接失败，请检查网络后重试。")?;
        let data = online::json(response).await?;
        let devices = parse_devices(&data)?;
        stop_task(&mut lifecycle).await;
        let mut inner = self.inner.lock().await;
        inner.cookie = cookie;
        inner.state = VoiceState {
            connected: true,
            devices,
            ..Default::default()
        };
        Ok(inner.state.clone())
    }

    pub async fn disable(&self, disconnect: bool) -> VoiceState {
        let mut lifecycle = self.lifecycle.lock().await;
        stop_task(&mut lifecycle).await;
        let mut inner = self.inner.lock().await;
        inner.state.listening = false;
        if disconnect {
            inner.cookie.clear();
            inner.state = VoiceState::default();
        }
        inner.state.clone()
    }

    pub async fn enable(
        self: &Arc<Self>,
        engine: Weak<Engine>,
        source_id: &str,
        target_id: &str,
        shutdown: CancellationToken,
    ) -> Result<VoiceState, String> {
        let mut lifecycle = self.lifecycle.lock().await;
        let (cookie, device) = {
            let inner = self.inner.lock().await;
            if !inner.state.connected {
                return Err("请先连接小米账号。".into());
            }
            let device = inner
                .state
                .devices
                .iter()
                .find(|d| d.id == source_id)
                .cloned()
                .ok_or("请选择接收语音的小爱音响。")?;
            (inner.cookie.clone(), device)
        };
        let active = engine.upgrade().ok_or("客户端正在退出")?;
        if active.snapshot().await?.selected_device_id.as_deref() != Some(target_id) {
            return Err("请先在主界面选择要播放的音响。".into());
        }
        stop_task(&mut lifecycle).await;
        let cancel = shutdown.child_token();
        {
            let mut inner = self.inner.lock().await;
            inner.state.listening = true;
            inner.state.source_id = Some(source_id.to_owned());
            inner.state.target_id = Some(target_id.to_owned());
            inner.state.error = None;
            inner.state.last_command = None;
            inner.state.last_result = None;
            inner.state.last_poll_ms = None;
        }
        let service = self.clone();
        let task_cancel = cancel.clone();
        let target_id = target_id.to_owned();
        let since = online::now_ms();
        let task = tokio::spawn(async move {
            service
                .listen(engine, cookie, device, target_id, since, task_cancel)
                .await;
        });
        *lifecycle = Some((cancel, task));
        Ok(self.state().await)
    }

    async fn listen(
        &self,
        engine: Weak<Engine>,
        cookie: String,
        device: VoiceDevice,
        target: String,
        mut cursor: u64,
        cancel: CancellationToken,
    ) {
        let mut failures = 0u32;
        loop {
            let result = tokio::select! {
                _ = cancel.cancelled() => break,
                result = self.conversations(&cookie, &device) => result,
            };
            match result {
                Ok(records) => {
                    failures = 0;
                    let now = online::now_ms();
                    let command = newest_command(&records, cursor, now);
                    cursor = records
                        .iter()
                        .filter(|r| r.time <= now + 2000)
                        .map(|r| r.time)
                        .max()
                        .unwrap_or(cursor)
                        .max(cursor);
                    {
                        let mut inner = self.inner.lock().await;
                        inner.state.last_poll_ms = Some(now);
                        inner.state.error = None;
                    }
                    if let Some(text) = command {
                        if cancel.is_cancelled() {
                            break;
                        }
                        let Some(engine) = engine.upgrade() else {
                            break;
                        };
                        // Once a SOAP operation starts, wait for its result. disable()
                        // joins this task so no command can occur after it returns.
                        let outcome = engine.voice_command(&text, Some(&target)).await;
                        let mut inner = self.inner.lock().await;
                        inner.state.last_command = Some(text);
                        inner.state.last_result = Some(match outcome {
                            Ok(message) => message,
                            Err(error) => error,
                        });
                    }
                }
                Err(error) => {
                    failures += 1;
                    self.inner.lock().await.state.error = Some(if failures >= 5 {
                        format!("{error} 连续失败，已停止接收；请重新连接。")
                    } else {
                        error
                    });
                    if failures >= 5 {
                        break;
                    }
                }
            }
            tokio::select! {
                _ = cancel.cancelled() => break,
                _ = tokio::time::sleep(Duration::from_secs((3 * (1 << failures)).min(30))) => {}
            }
        }
        self.inner.lock().await.state.listening = false;
    }

    async fn conversations(
        &self,
        cookie: &str,
        device: &VoiceDevice,
    ) -> Result<Vec<Record>, String> {
        let response = self
            .client
            .get("https://userprofile.mina.mi.com/device_profile/v2/conversation")
            .query(&[
                ("source", "dialogu"),
                ("hardware", &device.hardware),
                ("timestamp", &online::now_ms().to_string()),
                ("limit", "10"),
            ])
            .header(
                reqwest::header::COOKIE,
                format!("{cookie}; deviceId={}", device.id),
            )
            .timeout(Duration::from_secs(15))
            .send()
            .await
            .map_err(|_| "暂时无法读取小爱指令，请检查网络。")?;
        parse_records(&online::json(response).await?)
    }
}

async fn stop_task(lifecycle: &mut Option<(CancellationToken, tokio::task::JoinHandle<()>)>) {
    if let Some((cancel, task)) = lifecycle.take() {
        cancel.cancel();
        let _ = task.await;
    }
}

fn parse_devices(data: &Value) -> Result<Vec<VoiceDevice>, String> {
    if data["code"].as_i64() != Some(0) {
        return Err("小爱登录会话已失效或无法读取设备，请重新登录。".into());
    }
    let records = data["data"]
        .as_array()
        .ok_or("小米返回的设备列表格式已改变。")?;
    let mut devices = Vec::new();
    for item in records.iter().take(100) {
        let id = item["deviceID"]
            .as_str()
            .or(item["deviceId"].as_str())
            .unwrap_or_default();
        let hardware = item["hardware"].as_str().unwrap_or_default();
        if id.is_empty()
            || id.len() > 200
            || !id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            || hardware.is_empty()
        {
            continue;
        }
        devices.push(VoiceDevice {
            id: id.into(),
            name: item["name"]
                .as_str()
                .unwrap_or(hardware)
                .chars()
                .take(100)
                .collect(),
            hardware: hardware.chars().take(40).collect(),
        });
    }
    if devices.is_empty() {
        return Err("该账号没有可读取的小爱音响，请使用绑定音响的账号。".into());
    }
    Ok(devices)
}
#[derive(Debug)]
struct Record {
    time: u64,
    query: String,
}
fn parse_records(data: &Value) -> Result<Vec<Record>, String> {
    if data["code"].as_i64() != Some(0) {
        return Err("小爱会话已失效或语音记录不可用，请重新登录。".into());
    }
    let body = if let Some(text) = data["data"].as_str() {
        serde_json::from_str::<Value>(text).map_err(|_| "小爱语音记录格式已改变。")?
    } else {
        data["data"].clone()
    };
    let items = body["records"]
        .as_array()
        .ok_or("小爱未返回语音记录列表，请确认账号已开启对话记录。")?;
    Ok(items
        .iter()
        .take(100)
        .filter_map(|r| {
            Some(Record {
                time: r["time"].as_u64()?,
                query: r["query"].as_str()?.chars().take(300).collect(),
            })
        })
        .collect())
}
fn newest_command(records: &[Record], cursor: u64, now: u64) -> Option<String> {
    records
        .iter()
        .filter(|r| {
            r.time > cursor && r.time <= now + 2000 && now.saturating_sub(r.time) <= 30_000 && {
                let query = normalized(&r.query);
                let query = query.strip_prefix("小爱同学").unwrap_or(&query);
                parse(query).is_ok()
                    || ["播放本地", "本地播放", "本地音量"]
                        .iter()
                        .any(|prefix| query.starts_with(prefix))
            }
        })
        .max_by_key(|r| r.time)
        .map(|r| r.query.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn command_namespace_and_chinese_numbers() {
        assert_eq!(
            parse("小爱同学，播放本地 晴天！").unwrap(),
            Command::Play("晴天".into())
        );
        assert_eq!(
            parse("本地音量三十五").unwrap(),
            Command::Control("volume", Some(35))
        );
        assert_eq!(
            parse("本地音量一百").unwrap(),
            Command::Control("volume", Some(100))
        );
        for s in ["明天天气", "播放周杰伦", "本地音量101", "播放本地"] {
            assert!(parse(s).is_err());
        }
    }
    #[test]
    fn conversation_age_dedup_and_latest_intent() {
        let records = parse_records(&json!({"code":0,"data":json!({"records":[
            {"time":100000,"query":"本地暂停"}, {"time":101000,"query":"本地继续"},
            {"time":103000,"query":"今天天气"}, {"time":999999,"query":"本地停止"}
        ]}).to_string()}))
        .unwrap();
        assert_eq!(
            newest_command(&records, 99000, 104000),
            Some("本地继续".into())
        );
        assert!(newest_command(&records, 101000, 104000).is_none());
        assert!(newest_command(&records, 0, 150000).is_none());
        assert!(parse_records(&json!({"code":401,"data":""})).is_err());
        let invalid = [Record {
            time: 104000,
            query: "本地音量一百零一".into(),
        }];
        assert_eq!(
            newest_command(&invalid, 103000, 104000),
            Some("本地音量一百零一".into())
        );
        assert!(parse(&invalid[0].query).is_err());
    }
    #[test]
    fn device_id_cannot_inject_cookie() {
        assert!(parse_devices(
            &json!({"code":0,"data":[{"deviceID":"id; secret=x", "hardware":"LX06"}]})
        )
        .is_err());
        let valid = parse_devices(
            &json!({"code":0,"data":[{"deviceID":"abc-123", "hardware":"LX06","name":"客厅"}]}),
        )
        .unwrap();
        assert_eq!(valid[0].name, "客厅");
    }
}
