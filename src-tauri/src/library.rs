//! Persistent allowlist of explicitly imported files, with handle-based identity checks.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use uuid::Uuid;
use walkdir::WalkDir;

const MAX_TRACKS: usize = 20_000;
const MAX_WARNINGS: usize = 20;
const SUPPORTED: &[&str] = &[
    "mp3", "flac", "wav", "m4a", "aac", "ogg", "opus", "wma", "aiff", "aif",
];

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Track {
    pub id: String,
    pub title: String,
    pub filename: String,
    pub format: String,
    pub size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<TrackSource>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct TrackSource {
    pub provider: String,
    pub id: String,
    pub url: String,
}

#[derive(Serialize, Debug)]
pub struct ImportSummary {
    pub added: usize,
    pub total: usize,
    pub skipped: usize,
    pub warnings: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "platform", rename_all = "lowercase")]
enum Identity {
    Unix { device: u64, inode: u64 },
    Windows { volume: u32, index: u64 },
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct Record {
    #[serde(flatten)]
    track: Track,
    path: PathBuf,
    identity: Identity,
}

#[derive(Serialize, Deserialize)]
struct Index {
    version: u32,
    tracks: Vec<Record>,
}

pub struct Library {
    data_dir: PathBuf,
    records: Vec<Record>,
}

pub fn formats() -> Vec<String> {
    SUPPORTED.iter().map(|value| (*value).to_owned()).collect()
}

pub fn mime(format: &str) -> &'static str {
    match format {
        "mp3" => "audio/mpeg",
        "flac" => "audio/flac",
        "wav" => "audio/wav",
        "m4a" => "audio/mp4",
        "aac" => "audio/aac",
        "ogg" | "opus" => "audio/ogg",
        "wma" => "audio/x-ms-wma",
        "aiff" | "aif" => "audio/aiff",
        _ => "application/octet-stream",
    }
}

impl Library {
    pub fn load(data_dir: PathBuf) -> Result<Self, String> {
        fs::create_dir_all(&data_dir).map_err(|error| format!("无法创建音乐库目录：{error}"))?;
        let index_path = data_dir.join("library.json");
        let records = match fs::read(&index_path) {
            Ok(bytes) => {
                let index: Index = serde_json::from_slice(&bytes).map_err(|error| {
                    format!(
                        "音乐库索引无法读取，请先备份 {}：{error}",
                        index_path.display()
                    )
                })?;
                if index.version != 1 || index.tracks.len() > MAX_TRACKS {
                    return Err("音乐库版本或大小无效，请备份索引后重新导入。".into());
                }
                let mut ids = HashSet::new();
                let mut paths = HashSet::new();
                for record in &index.tracks {
                    if Uuid::parse_str(&record.track.id).is_err()
                        || !SUPPORTED.contains(&record.track.format.as_str())
                        || !record.path.is_absolute()
                        || !ids.insert(record.track.id.clone())
                        || !paths.insert(record.path.clone())
                    {
                        return Err("音乐库索引包含无效记录，请备份后重新导入。".into());
                    }
                }
                index.tracks
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(error) => return Err(format!("无法读取音乐库索引：{error}")),
        };
        Ok(Self { data_dir, records })
    }

    pub fn tracks(&self) -> Vec<Track> {
        self.records
            .iter()
            .map(|record| record.track.clone())
            .collect()
    }

    pub fn get(&self, id: &str) -> Result<Track, String> {
        self.record(id).map(|record| record.track.clone())
    }

    fn record(&self, id: &str) -> Result<&Record, String> {
        self.records
            .iter()
            .find(|record| record.track.id == id)
            .ok_or_else(|| "这首音乐不在音乐库中，请重新导入。".into())
    }

    pub fn open_track(&self, id: &str) -> Result<File, String> {
        let record = self.record(id)?;
        let file = open_regular(&record.path)?;
        if identity(&file)? != record.identity {
            return Err("音乐文件已被替换，请重新导入后播放。".into());
        }
        // Identity is checked on the open handle, so a subsequent pathname swap
        // cannot redirect the stream. Canonical comparison also rejects moved
        // parent directories replaced by symlinks, even to the original file.
        if fs::canonicalize(&record.path).map_err(|_| "音乐路径已失效，请重新导入。")?
            != record.path
        {
            return Err("音乐路径已改变或包含链接，请重新导入。".into());
        }
        Ok(file)
    }

    pub fn import_paths(&mut self, paths: &[PathBuf]) -> Result<ImportSummary, String> {
        if paths.is_empty() {
            return Err("请选择音乐文件或文件夹。".into());
        }
        let original = self.records.clone();
        let mut known: HashMap<PathBuf, usize> = self
            .records
            .iter()
            .enumerate()
            .map(|(index, record)| (record.path.clone(), index))
            .collect();
        let mut summary = ImportSummary {
            added: 0,
            total: self.records.len(),
            skipped: 0,
            warnings: Vec::new(),
        };
        for selected in paths {
            let metadata = match fs::symlink_metadata(selected) {
                Ok(metadata) => metadata,
                Err(error) => {
                    skip(
                        &mut summary,
                        format!("无法访问 {}：{error}", selected.display()),
                    );
                    continue;
                }
            };
            if is_link(&metadata) {
                skip(
                    &mut summary,
                    format!("已跳过符号链接：{}", selected.display()),
                );
                continue;
            }
            let canonical = match fs::canonicalize(selected) {
                Ok(path) => path,
                Err(error) => {
                    skip(
                        &mut summary,
                        format!("无法读取路径 {}：{error}", selected.display()),
                    );
                    continue;
                }
            };
            if metadata.is_dir() {
                let walker = WalkDir::new(&canonical)
                    .follow_links(false)
                    .sort_by_file_name()
                    .into_iter()
                    .filter_entry(|entry| {
                        !entry.file_type().is_dir()
                            || (!entry
                                .file_name()
                                .to_str()
                                .is_some_and(|name| name.starts_with('.'))
                                && !fs::symlink_metadata(entry.path()).is_ok_and(|metadata| {
                                    is_link(&metadata) || is_hidden(&metadata)
                                }))
                    });
                for entry in walker {
                    let entry = match entry {
                        Ok(entry) => entry,
                        Err(error) => {
                            skip(&mut summary, format!("部分文件无法访问：{error}"));
                            continue;
                        }
                    };
                    if entry.file_type().is_dir() {
                        continue;
                    }
                    if entry
                        .path()
                        .extension()
                        .and_then(|value| value.to_str())
                        .is_some_and(|value| {
                            SUPPORTED.contains(&value.to_ascii_lowercase().as_str())
                        })
                    {
                        self.add_file(entry.path(), Some(&canonical), &mut known, &mut summary);
                    }
                }
            } else {
                self.add_file(&canonical, None, &mut known, &mut summary);
            }
        }
        summary.total = self.records.len();
        if let Err(error) = self.save() {
            self.records = original;
            return Err(error);
        }
        Ok(summary)
    }

    /// Publish downloaded metadata and the file identity in one index transaction.
    pub fn import_download(
        &mut self,
        path: &Path,
        title: &str,
        source: TrackSource,
    ) -> Result<Track, String> {
        let original = self.records.clone();
        let canonical = fs::canonicalize(path).map_err(|e| format!("下载文件无法读取：{e}"))?;
        let mut known = self
            .records
            .iter()
            .enumerate()
            .map(|(i, r)| (r.path.clone(), i))
            .collect();
        let mut summary = ImportSummary {
            added: 0,
            total: 0,
            skipped: 0,
            warnings: vec![],
        };
        self.add_file(&canonical, None, &mut known, &mut summary);
        if summary.skipped > 0 {
            self.records = original;
            return Err(summary.warnings.join("；"));
        }
        let record = self
            .records
            .iter_mut()
            .find(|r| r.path == canonical)
            .ok_or("下载文件未能入库")?;
        record.track.title = title.chars().take(300).collect();
        record.track.source = Some(source);
        let track = record.track.clone();
        if let Err(error) = self.save() {
            self.records = original;
            return Err(error);
        }
        Ok(track)
    }

    fn add_file(
        &mut self,
        path: &Path,
        root: Option<&Path>,
        known: &mut HashMap<PathBuf, usize>,
        summary: &mut ImportSummary,
    ) {
        let result = (|| -> Result<(), String> {
            let metadata =
                fs::symlink_metadata(path).map_err(|error| format!("文件无法读取：{error}"))?;
            if is_link(&metadata) || !metadata.is_file() {
                return Err("已跳过符号链接或非普通文件。".into());
            }
            let canonical =
                fs::canonicalize(path).map_err(|error| format!("文件路径失效：{error}"))?;
            if canonical != path {
                return Err("导入时父文件夹已改变或变为链接，请重试。".into());
            }
            if root.is_some_and(|root| !canonical.starts_with(root)) {
                return Err("文件已移出所选文件夹，已跳过。".into());
            }
            let filename = canonical
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or("文件名无法表示为 Unicode，请重命名后重试。")?
                .to_owned();
            let format = canonical
                .extension()
                .and_then(|extension| extension.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if !SUPPORTED.contains(&format.as_str()) {
                return Err("不是支持的音乐文件格式。".into());
            }
            let file = open_regular(&canonical)?;
            let file_identity = identity(&file)?;
            if fs::canonicalize(&canonical).map_err(|_| "导入时文件已移动，请重试。")? != canonical
            {
                return Err("导入时文件路径发生变化，请重试。".into());
            }
            let size = file
                .metadata()
                .map_err(|error| format!("无法读取文件大小：{error}"))?
                .len();
            if let Some(&index) = known.get(&canonical) {
                // Explicit reimport repairs a replaced source while preserving
                // the existing track ID and queue position.
                self.records[index].identity = file_identity;
                self.records[index].track.size = size;
                return Ok(());
            }
            if self.records.len() >= MAX_TRACKS {
                return Err("音乐库最多支持 20,000 首音乐。".into());
            }
            let title = canonical
                .file_stem()
                .and_then(|name| name.to_str())
                .unwrap_or(&filename)
                .to_owned();
            let track = Track {
                id: Uuid::new_v4().to_string(),
                title,
                filename,
                format,
                size,
                source: None,
            };
            known.insert(canonical.clone(), self.records.len());
            self.records.push(Record {
                track,
                path: canonical,
                identity: file_identity,
            });
            summary.added += 1;
            Ok(())
        })();
        if let Err(error) = result {
            skip(summary, format!("{}：{error}", path.display()));
        }
    }

    fn save(&self) -> Result<(), String> {
        let mut temporary = tempfile::NamedTempFile::new_in(&self.data_dir)
            .map_err(|error| format!("无法创建音乐库临时文件：{error}"))?;
        serde_json::to_writer_pretty(
            &mut temporary,
            &Index {
                version: 1,
                tracks: self.records.clone(),
            },
        )
        .map_err(|error| format!("无法写入音乐库索引：{error}"))?;
        temporary
            .flush()
            .and_then(|_| temporary.as_file().sync_all())
            .map_err(|error| format!("无法保存音乐库索引：{error}"))?;
        temporary
            .persist(self.data_dir.join("library.json"))
            .map_err(|error| {
                format!("无法更新音乐库索引，请检查权限和磁盘空间：{}", error.error)
            })?;
        Ok(())
    }
}

fn skip(summary: &mut ImportSummary, warning: String) {
    summary.skipped += 1;
    if summary.warnings.len() < MAX_WARNINGS {
        summary.warnings.push(warning);
    }
}

fn is_link(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        return metadata.file_attributes()
            & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
            != 0;
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

fn is_hidden(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_HIDDEN
            != 0
    }
    #[cfg(not(windows))]
    {
        let _ = metadata;
        false
    }
}

fn open_regular(path: &Path) -> Result<File, String> {
    let before =
        fs::symlink_metadata(path).map_err(|error| format!("音乐文件无法读取或已移动：{error}"))?;
    if is_link(&before) || !before.is_file() {
        return Err("音乐文件已变为链接或非普通文件，请重新导入。".into());
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // NONBLOCK prevents a concurrently substituted FIFO from hanging.
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = options
        .open(path)
        .map_err(|error| format!("无法安全打开音乐文件：{error}"))?;
    let opened = file
        .metadata()
        .map_err(|error| format!("无法读取音乐文件信息：{error}"))?;
    if !opened.is_file() || is_link(&opened) {
        return Err("音乐文件已变为链接或非普通文件，请重新导入。".into());
    }
    Ok(file)
}

#[cfg(unix)]
fn identity(file: &File) -> Result<Identity, String> {
    use std::os::unix::fs::MetadataExt;
    let metadata = file
        .metadata()
        .map_err(|error| format!("无法校验音乐文件身份：{error}"))?;
    Ok(Identity::Unix {
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}

#[cfg(windows)]
fn identity(file: &File) -> Result<Identity, String> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };
    let mut information = std::mem::MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::uninit();
    // The live File owns a valid handle; Win32 initializes the complete output
    // structure when it returns nonzero. No unstable std Windows APIs are used.
    let success =
        unsafe { GetFileInformationByHandle(file.as_raw_handle(), information.as_mut_ptr()) };
    if success == 0 {
        return Err(format!(
            "无法校验音乐文件身份：{}",
            std::io::Error::last_os_error()
        ));
    }
    let information = unsafe { information.assume_init() };
    Ok(Identity::Windows {
        volume: information.dwVolumeSerialNumber,
        index: ((information.nFileIndexHigh as u64) << 32) | information.nFileIndexLow as u64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn setup() -> (tempfile::TempDir, Library, PathBuf) {
        let directory = tempfile::tempdir().unwrap();
        let music = directory.path().join("music");
        fs::create_dir(&music).unwrap();
        let library = Library::load(directory.path().join("data")).unwrap();
        (directory, library, music)
    }

    #[test]
    fn unicode_import_is_recursive_deduplicated_and_persistent() {
        let (directory, mut library, music) = setup();
        fs::create_dir(music.join("专辑")).unwrap();
        fs::create_dir(music.join(".hidden")).unwrap();
        fs::write(music.join("夏天.MP3"), b"original music").unwrap();
        fs::write(music.join("专辑/second.flac"), b"flac").unwrap();
        fs::write(music.join(".hidden/ignored.mp3"), b"hidden").unwrap();
        fs::write(music.join("notes.txt"), b"notes").unwrap();
        let result = library.import_paths(std::slice::from_ref(&music)).unwrap();
        assert_eq!((result.added, result.total), (2, 2));
        assert_eq!(
            library
                .import_paths(std::slice::from_ref(&music))
                .unwrap()
                .added,
            0
        );
        let expected = library.tracks();
        drop(library);
        let restored = Library::load(directory.path().join("data")).unwrap();
        assert_eq!(restored.tracks(), expected);
        let song = restored
            .tracks()
            .into_iter()
            .find(|track| track.filename == "夏天.MP3")
            .unwrap();
        assert_eq!((song.title.as_str(), song.format.as_str()), ("夏天", "mp3"));
        let mut content = String::new();
        restored
            .open_track(&song.id)
            .unwrap()
            .read_to_string(&mut content)
            .unwrap();
        assert_eq!(content, "original music");
        let public = serde_json::to_string(&song).unwrap();
        assert!(!public.contains("path"));
        assert!(!public.contains("identity"));
        assert_eq!(fs::read(music.join("夏天.MP3")).unwrap(), b"original music");
    }

    #[test]
    fn replaced_regular_file_is_rejected_until_explicit_reimport() {
        let (_directory, mut library, music) = setup();
        let song = music.join("song.mp3");
        fs::write(&song, b"original").unwrap();
        library.import_paths(std::slice::from_ref(&song)).unwrap();
        let id = library.tracks()[0].id.clone();
        fs::rename(&song, music.join("original.mp3")).unwrap();
        fs::write(&song, b"replacement").unwrap();
        assert!(library.open_track(&id).unwrap_err().contains("替换"));
        assert_eq!(library.import_paths(&[song]).unwrap().added, 0);
        let mut contents = String::new();
        library
            .open_track(&id)
            .unwrap()
            .read_to_string(&mut contents)
            .unwrap();
        assert_eq!(contents, "replacement");
    }

    #[test]
    fn malformed_index_is_preserved_and_never_overwritten() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("library.json");
        fs::write(&path, b"not json").unwrap();
        assert!(Library::load(directory.path().to_path_buf()).is_err());
        assert_eq!(fs::read(path).unwrap(), b"not json");
    }

    #[test]
    fn missing_and_unsupported_files_produce_useful_warnings() {
        let (_directory, mut library, music) = setup();
        let text = music.join("notes.txt");
        fs::write(&text, b"notes").unwrap();
        let result = library
            .import_paths(&[music.join("missing.mp3"), text])
            .unwrap();
        assert_eq!((result.added, result.skipped), (0, 2));
        assert_eq!(result.warnings.len(), 2);
        assert!(library.get("missing").is_err());
    }

    #[test]
    fn failed_persistence_rolls_back_in_memory_import() {
        let (_directory, mut library, music) = setup();
        fs::write(music.join("song.mp3"), b"audio").unwrap();
        fs::create_dir(library.data_dir.join("library.json")).unwrap();
        assert!(library.import_paths(&[music]).is_err());
        assert!(library.tracks().is_empty());
    }

    #[test]
    fn downloaded_metadata_rolls_back_when_index_cannot_be_saved() {
        let (_directory, mut library, music) = setup();
        let path = music.join("download.m4a");
        fs::write(&path, b"audio").unwrap();
        fs::create_dir(library.data_dir.join("library.json")).unwrap();
        let source = TrackSource {
            provider: "bilibili".into(),
            id: "fixture:1".into(),
            url: "https://www.bilibili.com/".into(),
        };
        assert!(library.import_download(&path, "下载歌曲", source).is_err());
        assert!(library.tracks().is_empty());
        assert!(path.exists());
    }

    #[cfg(unix)]
    #[test]
    fn symlink_files_and_replaced_parent_directories_cannot_be_streamed() {
        use std::os::unix::fs::symlink;
        let (directory, mut library, music) = setup();
        let secret = directory.path().join("secret.mp3");
        fs::write(&secret, b"private").unwrap();
        symlink(&secret, music.join("linked.mp3")).unwrap();
        fs::write(music.join("song.mp3"), b"audio").unwrap();
        let result = library.import_paths(std::slice::from_ref(&music)).unwrap();
        assert_eq!((result.added, result.skipped), (1, 1));
        let id = library.tracks()[0].id.clone();
        fs::rename(&music, directory.path().join("old-music")).unwrap();
        fs::create_dir(directory.path().join("replacement")).unwrap();
        fs::write(directory.path().join("replacement/song.mp3"), b"private").unwrap();
        symlink(directory.path().join("replacement"), &music).unwrap();
        assert!(library.open_track(&id).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn replaced_file_symlink_is_rejected() {
        use std::os::unix::fs::symlink;
        let (_directory, mut library, music) = setup();
        let song = music.join("song.mp3");
        fs::write(&song, b"audio").unwrap();
        library.import_paths(std::slice::from_ref(&song)).unwrap();
        let id = library.tracks()[0].id.clone();
        fs::rename(&song, music.join("original.mp3")).unwrap();
        symlink(music.join("original.mp3"), &song).unwrap();
        assert!(library.open_track(&id).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn replaced_fifo_is_rejected_without_blocking() {
        use std::os::unix::ffi::OsStrExt;
        let (_directory, mut library, music) = setup();
        let song = music.join("song.mp3");
        fs::write(&song, b"audio").unwrap();
        library.import_paths(std::slice::from_ref(&song)).unwrap();
        let id = library.tracks()[0].id.clone();
        fs::remove_file(&song).unwrap();
        let name = std::ffi::CString::new(song.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        assert!(library.open_track(&id).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn windows_hidden_attribute_directories_are_skipped() {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::{
            SetFileAttributesW, FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_NORMAL,
        };
        let (_directory, mut library, music) = setup();
        let hidden = music.join("Windows hidden folder");
        fs::create_dir(&hidden).unwrap();
        fs::write(hidden.join("private.mp3"), b"private").unwrap();
        fs::write(music.join("song.mp3"), b"audio").unwrap();
        let path: Vec<u16> = hidden.as_os_str().encode_wide().chain(Some(0)).collect();
        assert_ne!(
            unsafe { SetFileAttributesW(path.as_ptr(), FILE_ATTRIBUTE_HIDDEN) },
            0
        );
        let result = library.import_paths(std::slice::from_ref(&music)).unwrap();
        assert_eq!(result.added, 1);
        assert_eq!(library.tracks()[0].filename, "song.mp3");
        assert_ne!(
            unsafe { SetFileAttributesW(path.as_ptr(), FILE_ATTRIBUTE_NORMAL) },
            0
        );
    }
}
