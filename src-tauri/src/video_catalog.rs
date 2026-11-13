//! Local video history. It never reads or rewrites the music index.
use crate::video::{SourceStamp, VideoSource};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs,
    io::{Read, Write},
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

const VERSION: u32 = 1;
const MAX_ENTRIES: usize = 100;
const MAX_BYTES: u64 = 4 * 1024 * 1024;
const MAX_POSITION_MS: u64 = 359_999_000;
const UNAVAILABLE: &str = "最近影片索引无法读取或版本不受支持，已暂停记录保存；可继续选择原文件投送。请备份索引并修复后重启。";

pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SavedProgress {
    pub position_ms: u64,
    pub duration_ms: u64,
    pub observed_at_ms: u64,
    // This binding is local-only; it does not authorize Seek or establish compatibility.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) device_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Record {
    id: String,
    source: SourceStamp,
    title: String,
    format: String,
    size: String,
    duration_ms: Option<u64>,
    video_codec: Option<String>,
    last_selected_ms: u64,
    progress: Option<SavedProgress>,
}

#[derive(Clone, Debug, Serialize)]
pub struct RecentVideo {
    pub id: String,
    pub title: String,
    pub format: String,
    pub size: String,
    pub duration_ms: Option<u64>,
    pub video_codec: Option<String>,
    pub last_selected_ms: u64,
    pub progress: Option<SavedProgress>,
}

#[derive(Clone, Default)]
pub(crate) struct CatalogSnapshot {
    pub recent: Vec<RecentVideo>,
    pub warning: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct Index {
    version: u32,
    entries: Vec<Record>,
}

pub(crate) struct VideoCatalog {
    path: PathBuf,
    entries: Vec<Record>,
    expected: Option<Vec<u8>>,
    blocked: bool,
    warning: Option<String>,
}

fn read_index(path: &std::path::Path) -> Result<Option<Vec<u8>>, ()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(()),
    };
    if !metadata.is_file() || crate::video::is_link(&metadata) || metadata.len() > MAX_BYTES {
        return Err(());
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|_| ())?
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ())?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(());
    }
    Ok(Some(bytes))
}

impl VideoCatalog {
    pub fn load(data_dir: PathBuf) -> Self {
        let mut catalog = Self {
            path: data_dir.join("video-library.json"),
            entries: vec![],
            expected: None,
            blocked: false,
            warning: None,
        };
        let loaded = (|| -> Result<(), ()> {
            let Some(bytes) = read_index(&catalog.path)? else {
                return Ok(());
            };
            let index: Index = serde_json::from_slice(&bytes).map_err(|_| ())?;
            if index.version != VERSION || index.entries.len() > MAX_ENTRIES {
                return Err(());
            }
            let mut ids = HashSet::new();
            let mut paths = HashSet::new();
            for entry in &index.entries {
                if uuid::Uuid::parse_str(&entry.id).is_err()
                    || !ids.insert(entry.id.clone())
                    || !paths.insert(entry.source.path.clone())
                    || !entry.source.path.is_absolute()
                    || entry.title.chars().count() > 512
                    || !["mp4", "mkv", "m2ts"].contains(&entry.format.as_str())
                    || entry.size.parse::<u64>().is_err()
                    || entry
                        .video_codec
                        .as_ref()
                        .is_some_and(|codec| codec.chars().count() > 64)
                    || entry
                        .progress
                        .as_ref()
                        .is_some_and(|progress| !valid_progress(progress))
                {
                    return Err(());
                }
            }
            catalog.expected = Some(bytes);
            catalog.entries = index.entries;
            Ok(())
        })();
        if loaded.is_err() {
            catalog.blocked = true;
            catalog.warning = Some(UNAVAILABLE.into());
        }
        catalog
    }

    pub fn snapshot(&self) -> CatalogSnapshot {
        let mut recent: Vec<_> = self
            .entries
            .iter()
            .map(|entry| RecentVideo {
                id: entry.id.clone(),
                title: entry.title.clone(),
                format: entry.format.clone(),
                size: entry.size.clone(),
                duration_ms: entry.duration_ms,
                video_codec: entry.video_codec.clone(),
                last_selected_ms: entry.last_selected_ms,
                progress: entry.progress.as_ref().map(|progress| SavedProgress {
                    device_id: None,
                    ..progress.clone()
                }),
            })
            .collect();
        recent.sort_by_key(|entry| std::cmp::Reverse(activity(entry)));
        CatalogSnapshot {
            recent,
            warning: self.warning.clone(),
        }
    }

    pub fn remember(&mut self, source: &mut VideoSource) -> Result<(), String> {
        let stamp = source.stamp();
        let previous = self
            .entries
            .iter()
            .find(|entry| entry.source.path == stamp.path);
        let same = previous.filter(|entry| entry.source == stamp);
        if let Some(entry) = same {
            source.info.id = entry.id.clone();
        }
        let record = Record {
            id: source.info.id.clone(),
            source: stamp,
            title: source.info.title.chars().take(512).collect(),
            format: source.info.format.clone(),
            size: source.info.size.clone(),
            duration_ms: source.info.duration_ms,
            video_codec: source
                .info
                .video_codec
                .as_ref()
                .map(|codec| codec.chars().take(64).collect()),
            last_selected_ms: now_ms(),
            progress: same.and_then(|entry| entry.progress.clone()),
        };
        let mut entries = self.entries.clone();
        entries.retain(|entry| entry.source.path != record.source.path);
        entries.insert(0, record);
        entries.sort_by_key(|entry| {
            std::cmp::Reverse(
                entry
                    .last_selected_ms
                    .max(entry.progress.as_ref().map_or(0, |p| p.observed_at_ms)),
            )
        });
        entries.truncate(MAX_ENTRIES);
        self.commit(entries)
    }

    pub fn source(&self, id: &str) -> Result<SourceStamp, String> {
        if self.blocked {
            return Err(UNAVAILABLE.into());
        }
        self.entries
            .iter()
            .find(|entry| entry.id == id)
            .map(|entry| entry.source.clone())
            .ok_or_else(|| "影片记录已不存在，请重新选择文件。".into())
    }

    pub fn forget(&mut self, id: &str) -> Result<(), String> {
        let mut entries = self.entries.clone();
        entries.retain(|entry| entry.id != id);
        self.commit(entries)
    }

    pub fn progress(&mut self, id: &str, progress: SavedProgress) -> Result<(), String> {
        if !valid_progress(&progress) {
            return Err("播放位置无效，未保存。".into());
        }
        let mut entries = self.entries.clone();
        let Some(entry) = entries.iter_mut().find(|entry| entry.id == id) else {
            return Ok(());
        };
        entry.progress = Some(progress);
        self.commit(entries)
    }

    fn commit(&mut self, entries: Vec<Record>) -> Result<(), String> {
        if self.blocked {
            return Err(UNAVAILABLE.into());
        }
        let result = (|| -> Result<Vec<u8>, String> {
            let current = read_index(&self.path).map_err(|_| "最近影片索引不可写，记录未保存。")?;
            if current != self.expected {
                return Err("最近影片索引已被其他进程修改，未覆盖；请重启应用后再试。".into());
            }
            let bytes = serde_json::to_vec_pretty(&Index {
                version: VERSION,
                entries: entries.clone(),
            })
            .map_err(|_| "最近影片记录无法编码。")?;
            if bytes.len() as u64 > MAX_BYTES {
                return Err("最近影片索引超过容量限制，记录未保存。".into());
            }
            let parent = self.path.parent().ok_or("最近影片保存位置无效。")?;
            let mut temporary = tempfile::NamedTempFile::new_in(parent)
                .map_err(|_| "最近影片记录无法保存，请检查目录权限。")?;
            temporary
                .write_all(&bytes)
                .and_then(|_| temporary.as_file().sync_all())
                .map_err(|_| "最近影片记录写入失败，请检查空间。")?;
            temporary
                .persist(&self.path)
                .map_err(|_| "最近影片索引替换失败，旧记录已保留。")?;
            Ok(bytes)
        })();
        match result {
            Ok(bytes) => {
                self.entries = entries;
                self.expected = Some(bytes);
                self.warning = None;
                Ok(())
            }
            Err(error) => {
                self.warning = Some(error.clone());
                Err(error)
            }
        }
    }
}

fn valid_progress(progress: &SavedProgress) -> bool {
    progress.duration_ms > 0
        && progress.duration_ms <= MAX_POSITION_MS
        && progress.position_ms <= progress.duration_ms
        && progress
            .device_id
            .as_ref()
            .is_some_and(|id| !id.is_empty() && id.len() <= 4096)
}

fn activity(video: &RecentVideo) -> u64 {
    video.last_selected_ms.max(
        video
            .progress
            .as_ref()
            .map_or(0, |progress| progress.observed_at_ms),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn source(dir: &std::path::Path) -> VideoSource {
        let movie = dir.join("测试.mkv");
        fs::write(&movie, b"original video bytes").unwrap();
        VideoSource::open_import(movie).unwrap()
    }
    fn progress() -> SavedProgress {
        SavedProgress {
            position_ms: 45_000,
            duration_ms: 120_000,
            observed_at_ms: now_ms(),
            device_id: Some("uuid:test-tv".into()),
        }
    }

    #[test]
    fn reload_keeps_identity_and_progress_but_public_state_has_no_paths_or_device_ids() {
        let dir = tempfile::tempdir().unwrap();
        let mut source = source(dir.path());
        let mut catalog = VideoCatalog::load(dir.path().into());
        catalog.remember(&mut source).unwrap();
        catalog.progress(&source.info.id, progress()).unwrap();
        let mut restored = VideoCatalog::load(dir.path().into());
        restored
            .source(&source.info.id)
            .unwrap()
            .validate()
            .unwrap();
        let mut same = VideoSource::open_import(dir.path().join("测试.mkv")).unwrap();
        restored.remember(&mut same).unwrap();
        assert_eq!(same.info.id, source.info.id);
        assert_eq!(
            restored.snapshot().recent[0]
                .progress
                .as_ref()
                .unwrap()
                .position_ms,
            45_000
        );
        let output = serde_json::to_string(&restored.snapshot().recent).unwrap();
        assert!(!output.contains(dir.path().to_str().unwrap()));
        assert!(!output.contains("uuid:test-tv"));
        assert!(!output.contains("device_id"));
    }

    #[test]
    fn changed_file_is_rejected_and_explicit_reimport_resets_history() {
        let dir = tempfile::tempdir().unwrap();
        let mut source = source(dir.path());
        let mut catalog = VideoCatalog::load(dir.path().into());
        catalog.remember(&mut source).unwrap();
        catalog.progress(&source.info.id, progress()).unwrap();
        let old = catalog.source(&source.info.id).unwrap();
        fs::write(&old.path, b"replacement with a different size").unwrap();
        assert!(old.validate().is_err());
        let mut replacement = VideoSource::open_import(old.path).unwrap();
        catalog.remember(&mut replacement).unwrap();
        assert_ne!(replacement.info.id, source.info.id);
        assert!(catalog.snapshot().recent[0].progress.is_none());
    }

    #[test]
    fn corrupt_or_future_index_never_overwrites_itself_or_the_music_index() {
        for bytes in [
            b"broken".as_slice(),
            b"{\"version\":999,\"entries\":[]}".as_slice(),
        ] {
            let dir = tempfile::tempdir().unwrap();
            fs::write(dir.path().join("library.json"), b"music sentinel").unwrap();
            fs::write(dir.path().join("video-library.json"), bytes).unwrap();
            let mut catalog = VideoCatalog::load(dir.path().into());
            assert!(catalog.snapshot().warning.is_some());
            assert!(catalog.remember(&mut source(dir.path())).is_err());
            assert_eq!(
                fs::read(dir.path().join("video-library.json")).unwrap(),
                bytes
            );
            assert_eq!(
                fs::read(dir.path().join("library.json")).unwrap(),
                b"music sentinel"
            );
        }
    }

    #[test]
    fn external_changes_and_failed_publication_preserve_committed_state() {
        let dir = tempfile::tempdir().unwrap();
        let mut source = source(dir.path());
        let mut catalog = VideoCatalog::load(dir.path().into());
        catalog.remember(&mut source).unwrap();
        let index = dir.path().join("video-library.json");
        fs::write(&index, b"external edit").unwrap();
        assert!(catalog.forget(&source.info.id).is_err());
        assert_eq!(catalog.snapshot().recent.len(), 1);
        assert_eq!(fs::read(&index).unwrap(), b"external edit");
        fs::remove_file(&index).unwrap();
        fs::create_dir(&index).unwrap();
        assert!(catalog.progress(&source.info.id, progress()).is_err());
        assert!(catalog.snapshot().recent[0].progress.is_none());
    }

    #[test]
    fn forgetting_does_not_delete_the_movie_and_late_progress_does_not_restore_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut source = source(dir.path());
        let mut catalog = VideoCatalog::load(dir.path().into());
        catalog.remember(&mut source).unwrap();
        catalog.forget(&source.info.id).unwrap();
        catalog.progress(&source.info.id, progress()).unwrap();
        assert!(catalog.snapshot().recent.is_empty());
        source.open().unwrap();
    }
    #[test]
    fn bounds_history_and_refuses_oversized_indexes() {
        let dir = tempfile::tempdir().unwrap();
        let mut catalog = VideoCatalog::load(dir.path().into());
        for index in 0..101 {
            let file = dir.path().join(format!("movie-{index}.mkv"));
            fs::write(&file, b"movie").unwrap();
            let mut source = VideoSource::open_import(file).unwrap();
            catalog.remember(&mut source).unwrap();
        }
        let restored = VideoCatalog::load(dir.path().into());
        assert_eq!(restored.snapshot().recent.len(), 100);
        assert!(
            restored
                .snapshot()
                .recent
                .iter()
                .any(|entry| entry.title == "movie-100.mkv"),
            "the newest import must survive a full index, including timestamp ties"
        );
        let index = dir.path().join("video-library.json");
        fs::OpenOptions::new()
            .write(true)
            .open(&index)
            .unwrap()
            .set_len(MAX_BYTES + 1)
            .unwrap();
        let mut oversized = VideoCatalog::load(dir.path().into());
        assert!(oversized.snapshot().warning.is_some());
        assert!(oversized.remember(&mut source(dir.path())).is_err());
        assert_eq!(fs::metadata(index).unwrap().len(), MAX_BYTES + 1);
    }

    #[test]
    fn invalid_progress_does_not_overwrite_confirmed_progress() {
        let dir = tempfile::tempdir().unwrap();
        let mut source = source(dir.path());
        let mut catalog = VideoCatalog::load(dir.path().into());
        catalog.remember(&mut source).unwrap();
        catalog.progress(&source.info.id, progress()).unwrap();
        let index = dir.path().join("video-library.json");
        let before = fs::read(&index).unwrap();
        for (position_ms, duration_ms) in [(1, 0), (121_000, 120_000), (0, u64::MAX)] {
            assert!(catalog
                .progress(
                    &source.info.id,
                    SavedProgress {
                        position_ms,
                        duration_ms,
                        ..progress()
                    }
                )
                .is_err());
            assert_eq!(fs::read(&index).unwrap(), before);
        }
    }
}
