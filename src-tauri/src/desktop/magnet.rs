//! One user-initiated film download, shared across views of the desktop window.
use crate::torrent_transfer::{self, DownloadedFile, TransferUpdate};
use futures_util::FutureExt;
use serde::Serialize;
use std::{
    panic::AssertUnwindSafe,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;
use url::Url;

pub(super) struct MagnetLink {
    uri: String,
    name: String,
}

impl MagnetLink {
    pub(super) fn parse(raw: &str) -> Result<Self, String> {
        let raw = raw.trim();
        let invalid = "请输入含有效 BTIH 标识的 magnet 链接。";
        if raw.len() > 16384 {
            return Err("磁力链接过长。".into());
        }
        let input = Url::parse(raw).map_err(|_| invalid)?;
        if input.scheme() != "magnet" || input.has_host() || !input.path().is_empty() {
            return Err(invalid.into());
        }
        let mut hash = None;
        let mut name = None;
        let mut trackers = Vec::new();
        for (key, value) in input.query_pairs() {
            match key.as_ref() {
                "xt" => {
                    if let Some((_, value)) = value
                        .split_at_checked(9)
                        .filter(|(prefix, _)| prefix.eq_ignore_ascii_case("urn:btih:"))
                    {
                        let hex = normalize_hash(value).ok_or(invalid)?;
                        if hash.as_ref().is_some_and(|previous| previous != &hex) {
                            return Err("一个下载任务只能包含一个 BTIH 标识。".into());
                        }
                        hash = Some(hex);
                    }
                }
                "dn" if name.is_none() => {
                    let title: String = value
                        .chars()
                        .filter(|c| !c.is_control())
                        .take(240)
                        .collect();
                    if !title.trim().is_empty() {
                        name = Some(title.trim().to_owned());
                    }
                }
                "tr" if trackers.len() < 64 && value.len() <= 2048 => {
                    if let Ok(url) = Url::parse(&value) {
                        if matches!(url.scheme(), "udp" | "http" | "https")
                            && url.has_host()
                            && url.username().is_empty()
                            && url.password().is_none()
                            && !trackers.contains(&url)
                        {
                            trackers.push(url);
                        }
                    }
                }
                _ => {}
            }
        }
        let hash = hash.ok_or(invalid)?;
        let mut params = url::form_urlencoded::Serializer::new(String::new());
        params.append_pair("xt", &format!("urn:btih:{hash}"));
        if let Some(title) = &name {
            params.append_pair("dn", title);
        }
        for tracker in trackers {
            params.append_pair("tr", tracker.as_str());
        }
        let uri = format!("magnet:?{}", params.finish());
        Ok(Self {
            uri,
            name: name.unwrap_or(hash),
        })
    }
}

fn normalize_hash(input: &str) -> Option<String> {
    let valid = match input.len() {
        40 => input.bytes().all(|b| b.is_ascii_hexdigit()),
        32 => input
            .bytes()
            .all(|b| b.is_ascii_alphabetic() || (b'2'..=b'7').contains(&b)),
        _ => false,
    };
    if !valid {
        return None;
    }
    input
        .to_ascii_uppercase()
        .parse::<librqbit::dht::Id20>()
        .ok()
        .map(|id| id.as_string())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
enum Status {
    FetchingMetadata,
    Downloading,
    Complete,
    Cancelled,
    Failed,
}

impl Status {
    fn active(self) -> bool {
        matches!(self, Self::FetchingMetadata | Self::Downloading)
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Film {
    index: usize,
    name: String,
    size: u64,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Snapshot {
    id: String,
    status: Status,
    name: String,
    progress_bytes: u64,
    total_bytes: u64,
    speed_bytes_per_sec: u64,
    error: Option<String>,
    files: Vec<Film>,
}

struct Job {
    state: Snapshot,
    cancel: CancellationToken,
    stopped: CancellationToken,
    directory: PathBuf,
    checked_files: Vec<DownloadedFile>,
    sampled: (Instant, u64),
}

#[derive(Default, Clone)]
pub(super) struct Downloads(Arc<Mutex<Option<Job>>>);

impl Downloads {
    pub(super) fn status(&self) -> Option<Snapshot> {
        self.0.lock().unwrap().as_ref().map(|job| job.state.clone())
    }

    pub(super) fn ensure_available(&self) -> Result<(), String> {
        if self
            .0
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|job| job.state.status.active())
        {
            return Err("已有磁力下载进行中，请先完成或取消当前任务。".into());
        }
        Ok(())
    }

    pub(super) fn start(&self, link: MagnetLink, destination: PathBuf) -> Result<Snapshot, String> {
        self.ensure_available()?;
        let destination = destination.canonicalize().map_err(|_| "保存目录不可用。")?;
        let directory = tempfile::Builder::new()
            .prefix("影视下载-")
            .tempdir_in(destination)
            .map_err(|_| "无法在所选位置创建下载目录。")?;
        let id = uuid::Uuid::new_v4().to_string();
        let cancel = CancellationToken::new();
        let stopped = CancellationToken::new();
        let state = Snapshot {
            id: id.clone(),
            status: Status::FetchingMetadata,
            name: link.name,
            progress_bytes: 0,
            total_bytes: 0,
            speed_bytes_per_sec: 0,
            error: None,
            files: vec![],
        };
        {
            let mut slot = self.0.lock().unwrap();
            if slot.as_ref().is_some_and(|job| job.state.status.active()) {
                return Err("已有磁力下载进行中。".into());
            }
            *slot = Some(Job {
                state: state.clone(),
                cancel: cancel.clone(),
                stopped: stopped.clone(),
                directory: directory.path().to_owned(),
                checked_files: vec![],
                sampled: (Instant::now(), 0),
            });
        }
        let downloads = self.clone();
        tokio::spawn(async move {
            let _signal = stopped.drop_guard();
            let transfer =
                torrent_transfer::download(&link.uri, directory.path(), cancel, |update| {
                    downloads.progress(&id, update)
                });
            let result = AssertUnwindSafe(transfer)
                .catch_unwind()
                .await
                .unwrap_or_else(|_| Err("下载引擎意外停止，请重新下载。".into()));
            downloads.finish(&id, directory, result);
        });
        Ok(state)
    }

    fn progress(&self, id: &str, update: TransferUpdate) {
        let mut slot = self.0.lock().unwrap();
        let Some(job) = slot
            .as_mut()
            .filter(|job| job.state.id == id && job.state.status.active())
        else {
            return;
        };
        job.state.status = Status::Downloading;
        job.state.total_bytes = update.total_bytes;
        job.state.progress_bytes = update.downloaded_bytes.min(update.total_bytes);
        let elapsed = job.sampled.0.elapsed();
        if elapsed >= Duration::from_millis(500) {
            job.state.speed_bytes_per_sec = (job.state.progress_bytes.saturating_sub(job.sampled.1)
                as f64
                / elapsed.as_secs_f64()) as u64;
            job.sampled = (Instant::now(), job.state.progress_bytes);
        }
    }

    fn finish(&self, id: &str, directory: TempDir, result: Result<Vec<DownloadedFile>, String>) {
        // Check the selected files before making this directory permanent.
        let result = result.and_then(|files| {
            if files.is_empty() {
                return Err("该资源没有可投送的影片。".into());
            }
            let mut entries = Vec::with_capacity(files.len());
            for file in &files {
                let path = checked_file(directory.path(), file)?;
                let relative = path
                    .strip_prefix(directory.path())
                    .map_err(|_| "影片路径无效。")?;
                entries.push(Film {
                    index: file.index,
                    name: relative.to_string_lossy().into_owned(),
                    size: file.size,
                });
            }
            Ok((files, entries))
        });
        let mut slot = self.0.lock().unwrap();
        let Some(job) = slot.as_mut().filter(|job| job.state.id == id) else {
            return;
        };
        job.state.speed_bytes_per_sec = 0;
        if job.cancel.is_cancelled() {
            job.state.status = Status::Cancelled;
        } else {
            match result {
                Ok((files, entries)) => {
                    job.directory = directory.keep();
                    job.checked_files = files;
                    job.state.total_bytes = entries.iter().map(|file| file.size).sum();
                    job.state.progress_bytes = job.state.total_bytes;
                    job.state.files = entries;
                    job.state.status = Status::Complete;
                }
                Err(error) => {
                    job.state.status = Status::Failed;
                    job.state.error = Some(error);
                }
            }
        }
    }

    pub(super) async fn cancel(&self, id: &str) -> Result<Option<Snapshot>, String> {
        let done = {
            let slot = self.0.lock().unwrap();
            let job = slot
                .as_ref()
                .filter(|job| job.state.id == id)
                .ok_or("下载任务已经变更。")?;
            if job.state.status.active() {
                job.cancel.cancel();
            }
            job.stopped.clone()
        };
        done.cancelled().await;
        Ok(self.status())
    }

    pub(super) async fn shutdown(&self) {
        let id = self.status().filter(|s| s.status.active()).map(|s| s.id);
        if let Some(id) = id {
            let _ = self.cancel(&id).await;
        }
    }

    pub(super) fn completed_path(&self, id: &str, index: usize) -> Result<PathBuf, String> {
        let slot = self.0.lock().unwrap();
        let job = slot
            .as_ref()
            .filter(|job| job.state.id == id && job.state.status == Status::Complete)
            .ok_or("请先完成当前影片下载。")?;
        let file = job
            .checked_files
            .iter()
            .find(|file| file.index == index)
            .ok_or("找不到所选影片。")?;
        checked_file(&job.directory, file)
    }
}

fn checked_file(directory: &Path, file: &DownloadedFile) -> Result<PathBuf, String> {
    let path = directory.join(&file.path);
    let canonical = path
        .canonicalize()
        .map_err(|_| "下载文件已移动或无法读取。")?;
    let meta = canonical.metadata().map_err(|_| "下载文件不可用。")?;
    if !canonical.starts_with(directory)
        || canonical != path
        || !meta.is_file()
        || meta.len() != file.size
    {
        return Err("下载文件位置或大小已改变。".into());
    }
    Ok(canonical)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parsable_by_the_engine_with_name_and_http_and_udp_trackers() {
        let hash = "000102030405060708090a0b0c0d0e0f10111213";
        let sample = format!("magnet:?xt=urn:btih:{hash}&dn=Concert.2160p.HDR&tr=http%3A%2F%2Ftracker.example%3A80%2Fannounce&tr=udp%3A%2F%2Ftracker.example%3A2720");
        let link = MagnetLink::parse(&sample).unwrap();
        let parsed = librqbit::Magnet::parse(&link.uri).unwrap();
        assert_eq!(parsed.as_id20().unwrap().as_string(), hash);
        assert_eq!(parsed.name.as_deref(), Some("Concert.2160p.HDR"));
        assert_eq!(
            parsed.trackers,
            [
                "http://tracker.example/announce",
                "udp://tracker.example:2720"
            ]
        );
        let base32 =
            MagnetLink::parse("magnet:?xt=urn:btih:aaaqeayeaudaocajbifqydiob4ibceqt").unwrap();
        assert_eq!(base32.name, hash);
        for invalid in [
            "https://example.org",
            "magnet:?dn=nohash",
            "magnet:?xt=urn:btih:1234",
            "magnet:?xt=urn:btih:AAAA============================",
        ] {
            assert!(MagnetLink::parse(invalid).is_err());
        }
    }

    #[test]
    fn completed_movies_cannot_reference_outside_or_truncated_files() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().canonicalize().unwrap();
        let movie = directory.join("影片.mkv");
        std::fs::write(&movie, b"movie").unwrap();
        let mut file = DownloadedFile {
            index: 0,
            path: movie.clone(),
            size: 5,
        };
        assert_eq!(checked_file(&directory, &file).unwrap(), movie);
        let other = tempfile::NamedTempFile::new().unwrap();
        file.path = other.path().to_owned();
        file.size = 0;
        assert!(checked_file(&directory, &file).is_err());
        file.path = movie;
        assert!(checked_file(&directory, &file).is_err());
    }
}
