//! Local, on-demand aria2 process. The renderer never receives RPC credentials.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    io::Write,
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::Duration,
};
use tokio::sync::Mutex;

const LIMIT: usize = 100;
const KEYS: &[&str] = &[
    "status",
    "totalLength",
    "completedLength",
    "downloadSpeed",
    "connections",
    "errorCode",
    "followedBy",
    "bittorrent",
];

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Job {
    pub id: String,
    pub title: String,
    pub phase: String,
    pub downloaded: u64,
    pub total: u64,
    pub speed: u64,
    pub connections: u64,
    pub error: String,
}

#[derive(Serialize)]
pub struct DownloadState {
    pub available: bool,
    pub directory: String,
    pub jobs: Vec<Job>,
    pub error: String,
}

#[derive(Clone, Deserialize, Serialize)]
struct Record {
    uri: String,
    job: Job,
    // GIDs are process-local. Only the magnet and local partial files survive restart.
    #[serde(skip)]
    gid: Option<String>,
}

struct Inner {
    records: Vec<Record>,
    child: Option<Child>,
    endpoint: String,
    secret: String,
}

impl Drop for Inner {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

pub struct Downloader {
    root: PathBuf,
    client: reqwest::Client,
    inner: Mutex<Inner>,
}

/// Canonicalize v1 hashes so hex/base32 spellings cannot create duplicate jobs.
pub fn parse_magnet(input: &str) -> Result<(String, String, String), String> {
    if input.len() > 8192 || input.chars().any(char::is_control) {
        return Err("磁力链接过长或包含无效字符。".into());
    }
    let url = url::Url::parse(input.trim()).map_err(|_| "请粘贴有效的 magnet:? 链接。")?;
    if url.scheme() != "magnet" || url.host_str().is_some() || !url.path().is_empty() {
        return Err("请粘贴有效的 magnet:? 链接。".into());
    }
    let mut hashes = Vec::new();
    let mut title = None;
    let mut trackers = Vec::new();
    for (key, value) in url.query_pairs() {
        match key.as_ref() {
            "xt" => {
                if value
                    .get(..9)
                    .is_some_and(|prefix| prefix.eq_ignore_ascii_case("urn:btih:"))
                {
                    hashes.push(normalize_hash(&value[9..])?);
                }
            }
            "dn" if title.is_none() => {
                title = Some(
                    value
                        .chars()
                        .filter(|c| !c.is_control())
                        .take(240)
                        .collect::<String>(),
                );
            }
            "tr" => {
                if trackers.len() >= 32 {
                    return Err("单个任务最多支持 32 个 Tracker。".into());
                }
                let tracker = url::Url::parse(&value).map_err(|_| "Tracker 地址无效。")?;
                if !matches!(tracker.scheme(), "http" | "https" | "udp")
                    || tracker.host_str().is_none()
                {
                    return Err("Tracker 仅支持 HTTP、HTTPS 或 UDP 地址。".into());
                }
                trackers.push(value.into_owned());
            }
            // Do not accept arbitrary metadata URLs, file paths or aria2 options.
            _ => {}
        }
    }
    let hash = hashes
        .first()
        .cloned()
        .ok_or("链接缺少有效 BTIH；目前支持 BitTorrent v1 和混合种子。")?;
    if hashes.iter().any(|other| other != &hash) {
        return Err("链接包含不同的 BTIH，请使用单个资源的链接。".into());
    }
    let title = title
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| format!("磁力任务 {}…", &hash[..12]));
    let mut pairs = url::form_urlencoded::Serializer::new(String::new());
    pairs.append_pair("xt", &format!("urn:btih:{hash}"));
    pairs.append_pair("dn", &title);
    for tracker in trackers {
        pairs.append_pair("tr", &tracker);
    }
    Ok((hash, title, format!("magnet:?{}", pairs.finish())))
}

fn normalize_hash(hash: &str) -> Result<String, String> {
    if hash.len() == 40 && hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Ok(hash.to_ascii_lowercase());
    }
    if hash.len() == 32 {
        let mut buffer = 0u32;
        let mut bits = 0;
        let mut bytes = Vec::with_capacity(20);
        for b in hash.bytes() {
            let value = match b.to_ascii_uppercase() {
                b'A'..=b'Z' => b.to_ascii_uppercase() - b'A',
                b'2'..=b'7' => b - b'2' + 26,
                _ => return Err("BTIH 校验值无效。".into()),
            };
            buffer = (buffer << 5) | u32::from(value);
            bits += 5;
            if bits >= 8 {
                bits -= 8;
                bytes.push((buffer >> bits) as u8);
            }
        }
        return Ok(bytes.iter().map(|b| format!("{b:02x}")).collect());
    }
    Err("BTIH 应为 40 位十六进制或 32 位 Base32 校验值。".into())
}

fn executable() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("TV_DLNA_ARIA2C") {
        let path = PathBuf::from(path);
        return path.is_file().then_some(path);
    }
    let name = if cfg!(windows) {
        "aria2c.exe"
    } else {
        "aria2c"
    };
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            dirs.insert(0, parent.to_path_buf());
        }
    }
    dirs.extend([
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
    ]);
    dirs.into_iter().map(|p| p.join(name)).find(|p| p.is_file())
}

impl Downloader {
    pub fn directory(&self) -> PathBuf {
        self.root.clone()
    }

    pub fn new(data_dir: &Path) -> Result<Self, String> {
        let root = data_dir.join("magnets");
        std::fs::create_dir_all(&root).map_err(|e| format!("无法创建磁力下载目录：{e}"))?;
        let path = root.join("jobs.json");
        let mut records: Vec<Record> = match std::fs::read(&path) {
            Ok(bytes) if bytes.len() <= 2 * 1024 * 1024 => {
                serde_json::from_slice(&bytes).map_err(|e| format!("磁力任务记录无法读取：{e}"))?
            }
            Ok(_) => return Err("磁力任务记录过大。".into()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(format!("磁力任务记录无法读取：{e}")),
        };
        if records.len() > LIMIT {
            return Err("磁力任务数量超过限制。".into());
        }
        for record in &mut records {
            let (hash, _, _) = parse_magnet(&record.uri)?;
            if record.job.id != hash {
                return Err("磁力任务记录的校验值不一致。".into());
            }
            record.job.speed = 0;
            record.job.connections = 0;
            if !matches!(
                record.job.phase.as_str(),
                "completed" | "stopped" | "failed"
            ) {
                record.job.phase = "paused".into();
                record.job.error = "客户端已重启，点击继续可校验已有文件并续传。".into();
            }
        }
        Ok(Self {
            root,
            client: reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(3))
                .build()
                .map_err(|e| e.to_string())?,
            inner: Mutex::new(Inner {
                records,
                child: None,
                endpoint: String::new(),
                secret: String::new(),
            }),
        })
    }

    fn save(&self, inner: &Inner) -> Result<(), String> {
        let mut file = tempfile::NamedTempFile::new_in(&self.root)
            .map_err(|e| format!("无法保存磁力任务：{e}"))?;
        serde_json::to_writer(&mut file, &inner.records).map_err(|e| e.to_string())?;
        file.flush()
            .and_then(|()| file.as_file().sync_all())
            .map_err(|e| e.to_string())?;
        file.persist(self.root.join("jobs.json"))
            .map_err(|e| format!("无法保存磁力任务：{e}"))?;
        Ok(())
    }

    async fn rpc(&self, inner: &Inner, method: &str, params: Vec<Value>) -> Result<Value, String> {
        let mut authenticated = vec![json!(format!("token:{}", inner.secret))];
        authenticated.extend(params);
        // system.multicall authenticates its individual calls instead.
        if method == "system.multicall" {
            authenticated.remove(0);
        }
        let mut response = self
            .client
            .post(&inner.endpoint)
            .json(&json!({
                "jsonrpc": "2.0", "id": "tv-dlna", "method": method, "params": authenticated,
            }))
            .send()
            .await
            .map_err(|_| "下载引擎未响应，请稍后刷新任务状态。")?
            .error_for_status()
            .map_err(|_| "下载引擎返回异常响应。")?;
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| "下载引擎响应中断。")? {
            if bytes.len() + chunk.len() > 1024 * 1024 {
                return Err("下载引擎响应过大。".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        let value: Value = serde_json::from_slice(&bytes).map_err(|_| "下载引擎响应无法解析。")?;
        if value.get("error").is_some() {
            return Err("下载引擎未接受操作，请刷新后重试。".into());
        }
        value
            .get("result")
            .cloned()
            .ok_or_else(|| "下载引擎未返回操作结果。".into())
    }

    async fn ensure_started(&self, inner: &mut Inner) -> Result<(), String> {
        if let Some(child) = inner.child.as_mut() {
            if child.try_wait().map_err(|e| e.to_string())?.is_none() {
                return Ok(());
            }
        }
        inner.child = None;
        for record in &mut inner.records {
            record.gid = None;
            if matches!(
                record.job.phase.as_str(),
                "resolving" | "downloading" | "queued"
            ) {
                record.job.phase = "paused".into();
                record.job.speed = 0;
                record.job.connections = 0;
                record.job.error = "下载引擎已退出，点击继续可恢复。".into();
            }
        }
        let executable = executable()
            .ok_or("尚未找到 aria2。安装 aria2c 后重试，或通过 TV_DLNA_ARIA2C 指定完整路径。")?;
        let socket = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
        let port = socket.local_addr().map_err(|e| e.to_string())?.port();
        drop(socket);
        inner.secret = uuid::Uuid::new_v4().simple().to_string();
        inner.endpoint = format!("http://127.0.0.1:{port}/jsonrpc");
        let mut command = Command::new(executable);
        command
            .args([
                "--no-conf=true",
                "--enable-rpc=true",
                "--rpc-listen-all=false",
                "--rpc-allow-origin-all=false",
                "--seed-time=0",
                "--continue=true",
                "--check-integrity=true",
                "--auto-file-renaming=false",
                "--allow-overwrite=false",
                "--max-concurrent-downloads=3",
                "--max-download-result=256",
                "--file-allocation=none",
                "--summary-interval=0",
                "--console-log-level=error",
                "--auto-save-interval=5",
            ])
            .arg(format!("--rpc-listen-port={port}"))
            .arg(format!("--rpc-secret={}", inner.secret))
            .arg(format!("--stop-with-process={}", std::process::id()))
            .arg(format!(
                "--dht-file-path={}",
                self.root.join("dht.dat").display()
            ))
            .arg(format!(
                "--dht-file-path6={}",
                self.root.join("dht6.dat").display()
            ))
            .current_dir(&self.root)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }
        inner.child = Some(
            command
                .spawn()
                .map_err(|e| format!("无法启动 aria2：{e}"))?,
        );
        let ready = tokio::time::timeout(Duration::from_secs(4), async {
            loop {
                if let Ok(version) = self.rpc(inner, "aria2.getVersion", vec![]).await {
                    return version["enabledFeatures"]
                        .as_array()
                        .is_some_and(|features| {
                            features.iter().any(|feature| feature == "BitTorrent")
                        });
                }
                if inner
                    .child
                    .as_mut()
                    .and_then(|child| child.try_wait().ok())
                    .flatten()
                    .is_some()
                {
                    return false;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .unwrap_or(false);
        if ready {
            return Ok(());
        }
        if let Some(mut child) = inner.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        Err("aria2 启动失败，请确认安装的 aria2c 支持 BitTorrent 和 JSON-RPC。".into())
    }

    async fn refresh(&self, inner: &mut Inner) -> Result<(), String> {
        let Some(child) = inner.child.as_mut() else {
            return Ok(());
        };
        if child.try_wait().map_err(|e| e.to_string())?.is_some() {
            inner.child = None;
            for record in &mut inner.records {
                record.gid = None;
                if matches!(
                    record.job.phase.as_str(),
                    "resolving" | "downloading" | "queued"
                ) {
                    record.job.phase = "paused".into();
                    record.job.speed = 0;
                    record.job.error = "下载引擎已退出，点击继续可恢复。".into();
                }
            }
            return self.save(inner);
        }
        // One bounded RPC per poll, including metadata -> payload GID transitions.
        let calls: Vec<Value> = inner.records.iter().filter_map(|r| r.gid.as_ref()).map(|gid| json!({
            "methodName": "aria2.tellStatus", "params": [format!("token:{}", inner.secret), gid, KEYS],
        })).collect();
        if calls.is_empty() {
            return Ok(());
        }
        let result = self
            .rpc(inner, "system.multicall", vec![json!(calls)])
            .await?;
        let statuses = result.as_array().ok_or("下载任务状态无法解析。")?;
        let mut statuses = statuses.iter();
        let before = serde_json::to_vec(&inner.records).map_err(|e| e.to_string())?;
        for record in inner.records.iter_mut().filter(|r| r.gid.is_some()) {
            let status = statuses.next().ok_or("下载任务状态不完整。")?;
            if let Some(status) = status.get(0) {
                update_record(record, status);
            } else {
                record.gid = None;
                record.job.phase = "paused".into();
                record.job.speed = 0;
                record.job.error = "任务已中断，点击继续可校验已有文件并续传。".into();
            }
        }
        if serde_json::to_vec(&inner.records).map_err(|e| e.to_string())? != before {
            self.save(inner)?;
        }
        Ok(())
    }

    pub async fn state(&self) -> DownloadState {
        let mut inner = self.inner.lock().await;
        let error = self.refresh(&mut inner).await.err().unwrap_or_default();
        DownloadState {
            available: executable().is_some(),
            directory: self.root.display().to_string(),
            jobs: inner.records.iter().map(|r| r.job.clone()).collect(),
            error,
        }
    }

    pub async fn add(&self, uri: &str) -> Result<Job, String> {
        let (id, title, uri) = parse_magnet(uri)?;
        let mut inner = self.inner.lock().await;
        if inner.records.iter().any(|r| r.job.id == id) {
            return Err("此资源已在任务列表中，请继续或重试已有任务。".into());
        }
        if inner.records.len() >= LIMIT {
            return Err("最多保留 100 个任务，请先清除已结束的记录。".into());
        }
        self.ensure_started(&mut inner).await?;
        inner.records.push(Record {
            uri,
            gid: None,
            job: Job {
                id: id.clone(),
                title,
                phase: "paused".into(),
                downloaded: 0,
                total: 0,
                speed: 0,
                connections: 0,
                error: String::new(),
            },
        });
        if let Err(error) = self.save(&inner) {
            inner.records.pop();
            return Err(error);
        }
        let index = inner.records.len() - 1;
        self.start_record(&mut inner, index).await?;
        Ok(inner.records[index].job.clone())
    }

    async fn start_record(&self, inner: &mut Inner, index: usize) -> Result<(), String> {
        let record = &mut inner.records[index];
        let directory = self.root.join(&record.job.id);
        std::fs::create_dir_all(&directory).map_err(|e| format!("无法创建任务目录：{e}"))?;
        let gid = uuid::Uuid::new_v4().simple().to_string()[..16].to_string();
        record.gid = Some(gid.clone());
        record.job.phase = "resolving".into();
        record.job.error.clear();
        let uri = record.uri.clone();
        self.save(inner)?;
        let result = self
            .rpc(
                inner,
                "aria2.addUri",
                vec![
                    json!([uri]),
                    json!({ "dir": directory, "gid": gid, "seed-time": "0" }),
                ],
            )
            .await
            .and_then(|result| {
                if result.as_str() == Some(gid.as_str()) {
                    Ok(())
                } else {
                    Err("下载引擎未返回匹配的任务编号，请刷新任务状态后重试。".into())
                }
            });
        if let Err(error) = result {
            inner.records[index].job.error = error.clone();
            // Preserve the GID: a timed-out response may still have created the task.
            self.save(inner)?;
            return Err(error);
        }
        Ok(())
    }

    pub async fn control(&self, id: &str, action: &str) -> Result<(), String> {
        if !matches!(action, "pause" | "resume" | "stop") {
            return Err("不支持的下载操作。".into());
        }
        let mut inner = self.inner.lock().await;
        self.refresh(&mut inner).await?;
        let index = inner
            .records
            .iter()
            .position(|r| r.job.id == id)
            .ok_or("下载任务不存在。")?;
        if inner.records[index].job.phase == "completed" {
            return Err("此任务已经下载完成。".into());
        }
        if action == "resume" {
            self.ensure_started(&mut inner).await?;
            // Refresh follows a metadata GID before a subsequent control operation.
            self.refresh(&mut inner).await?;
            if let Some(gid) = inner.records[index].gid.clone() {
                match inner.records[index].job.phase.as_str() {
                    "paused" => {
                        self.rpc(&inner, "aria2.unpause", vec![json!(gid)]).await?;
                    }
                    "failed" | "stopped" => {
                        self.rpc(&inner, "aria2.removeDownloadResult", vec![json!(gid)])
                            .await?;
                        self.start_record(&mut inner, index).await?;
                    }
                    _ => return Ok(()),
                }
            } else {
                self.start_record(&mut inner, index).await?;
            }
            inner.records[index].job.phase = "resolving".into();
            inner.records[index].job.error.clear();
        } else {
            // Pausing first prevents the metadata task spawning a payload after stop.
            self.refresh(&mut inner).await?;
            if let Some(gid) = inner.records[index].gid.clone() {
                if action == "stop" && inner.records[index].job.phase == "failed" {
                    self.rpc(
                        &inner,
                        "aria2.removeDownloadResult",
                        vec![json!(gid.clone())],
                    )
                    .await?;
                    inner.records[index].gid = None;
                }
                if !matches!(
                    inner.records[index].job.phase.as_str(),
                    "failed" | "stopped"
                ) {
                    if inner.records[index].job.phase != "paused" {
                        self.rpc(&inner, "aria2.forcePause", vec![json!(gid)])
                            .await?;
                    }
                    if action == "stop" {
                        self.rpc(&inner, "aria2.forceRemove", vec![json!(gid)])
                            .await?;
                    }
                }
            }
            inner.records[index].job.phase = if action == "stop" {
                "stopped"
            } else {
                "paused"
            }
            .into();
            inner.records[index].job.speed = 0;
            inner.records[index].job.error.clear();
        }
        self.save(&inner)
    }

    /// Remove only ended records, retaining every downloaded/partial file.
    pub async fn clear_finished(&self) -> Result<(), String> {
        let mut inner = self.inner.lock().await;
        self.refresh(&mut inner).await?;
        let before = inner.records.clone();
        inner
            .records
            .retain(|r| !matches!(r.job.phase.as_str(), "completed" | "stopped"));
        if let Err(error) = self.save(&inner) {
            inner.records = before;
            return Err(error);
        }
        Ok(())
    }
}

fn update_record(record: &mut Record, status: &Value) {
    if let Some(next) = status["followedBy"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(Value::as_str)
    {
        record.gid = Some(next.into());
        record.job.phase = "resolving".into();
        return;
    }
    let number = |key: &str| {
        status[key]
            .as_str()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0)
    };
    let phase = status["status"].as_str().unwrap_or("error");
    record.job.phase = match phase {
        "active" if status["bittorrent"]["info"]["name"].as_str().is_none() => "resolving",
        "active" => "downloading",
        "waiting" => "queued",
        "paused" => "paused",
        "complete" => "completed",
        "removed" => "stopped",
        _ => "failed",
    }
    .into();
    if let Some(title) = status["bittorrent"]["info"]["name"].as_str() {
        record.job.title = title
            .chars()
            .filter(|c| !c.is_control())
            .take(240)
            .collect();
    }
    record.job.downloaded = number("completedLength");
    record.job.total = number("totalLength");
    record.job.speed = if phase == "active" {
        number("downloadSpeed")
    } else {
        0
    };
    record.job.connections = number("connections");
    record.job.error = if phase == "error" {
        format!(
            "下载失败（aria2 错误码 {}），请检查剩余空间和网络后重试。",
            status["errorCode"].as_str().unwrap_or("未知")
        )
    } else {
        String::new()
    };
}

#[cfg(test)]
mod tests;
