//! Local, explicitly imported video sources. Paths and file identities never leave Rust.

use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
    process::Stdio,
    time::{Duration, SystemTime},
};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct VideoTrackInfo {
    pub index: u32,
    pub codec: Option<String>,
    pub language: Option<String>,
    pub title: Option<String>,
    pub channels: Option<u32>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct VideoInfo {
    pub id: String,
    pub title: String,
    pub format: String,
    pub size: String,
    pub duration_ms: Option<u64>,
    pub video_codec: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub pixel_format: Option<String>,
    pub color_transfer: Option<String>,
    pub can_burn_hdr10: bool,
    pub audio_tracks: Vec<VideoTrackInfo>,
    pub subtitle_tracks: Vec<VideoTrackInfo>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SourceStamp {
    pub path: PathBuf,
    identity: FileIdentity,
    length: u64,
    modified: SystemTime,
}

impl SourceStamp {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if !self.path.is_absolute() {
            return Err("影片记录无效，请重新选择文件。".into());
        }
        let source = VideoSource::open_import(self.path.clone())?;
        if source.stamp() != *self {
            return Err("SOURCE_CHANGED：影片已被替换、修改或移动，请重新选择文件。".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct VideoSource {
    pub(crate) temporary: Option<std::sync::Arc<crate::subtitle_cache::Workspace>>,
    pub info: VideoInfo,
    path: PathBuf,
    identity: FileIdentity,
    length: u64,
    modified: SystemTime,
}

impl VideoSource {
    pub fn open_import(path: PathBuf) -> Result<Self, String> {
        let before = fs::symlink_metadata(&path).map_err(|_| "影片文件无法读取。")?;
        if is_link(&before) || !before.is_file() {
            return Err("请选择本地普通视频文件，不能使用链接或目录。".into());
        }
        let path = fs::canonicalize(path).map_err(|_| "影片路径无法解析。")?;
        let format = path
            .extension()
            .and_then(|v| v.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if !["mp4", "mkv", "m2ts"].contains(&format.as_str()) {
            return Err("当前仅接受 MP4、MKV 和 M2TS 单文件。".into());
        }
        let file = open_regular(&path)?;
        let metadata = file.metadata().map_err(|_| "影片文件信息无法读取。")?;
        if metadata.len() == 0 {
            return Err("影片文件为空。".into());
        }
        let identity = file_identity(&file)?;
        let modified = metadata.modified().map_err(|_| "影片修改时间无法读取。")?;
        let title = path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "影片".into());
        let source = Self {
            temporary: None,
            info: VideoInfo {
                id: uuid::Uuid::new_v4().to_string(),
                title,
                format,
                size: metadata.len().to_string(),
                duration_ms: None,
                video_codec: None,
                width: None,
                height: None,
                pixel_format: None,
                color_transfer: None,
                can_burn_hdr10: false,
                audio_tracks: vec![],
                subtitle_tracks: vec![],
            },
            path,
            identity,
            length: metadata.len(),
            modified,
        };
        source.open()?;
        Ok(source)
    }

    pub fn open(&self) -> Result<File, String> {
        let file = open_regular(&self.path)?;
        let metadata = file.metadata().map_err(|_| "影片文件信息无法读取。")?;
        if file_identity(&file)? != self.identity
            || metadata.len() != self.length
            || metadata.modified().ok() != Some(self.modified)
            || fs::canonicalize(&self.path).ok().as_ref() != Some(&self.path)
        {
            return Err("影片已被替换、修改或移动，请重新选择并分析。".into());
        }
        Ok(file)
    }

    pub(crate) fn stamp(&self) -> SourceStamp {
        SourceStamp {
            path: self.path.clone(),
            identity: self.identity.clone(),
            length: self.length,
            modified: self.modified,
        }
    }

    pub fn length(&self) -> u64 {
        self.length
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum FileIdentity {
    #[cfg(unix)]
    Unix { device: u64, inode: u64 },
    #[cfg(windows)]
    Windows { volume: u32, index: u64 },
}

pub(crate) fn is_link(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes()
            & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
            != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

fn open_regular(path: &Path) -> Result<File, String> {
    let before = fs::symlink_metadata(path).map_err(|_| "影片文件无法读取或已移动。")?;
    if is_link(&before) || !before.is_file() {
        return Err("影片已变为链接或非普通文件。".into());
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = options.open(path).map_err(|_| "无法安全打开影片文件。")?;
    let metadata = file.metadata().map_err(|_| "影片文件信息无法读取。")?;
    if is_link(&metadata) || !metadata.is_file() {
        return Err("影片已变为链接或非普通文件。".into());
    }
    Ok(file)
}

#[cfg(unix)]
fn file_identity(file: &File) -> Result<FileIdentity, String> {
    use std::os::unix::fs::MetadataExt;
    let m = file.metadata().map_err(|_| "无法校验影片身份。")?;
    Ok(FileIdentity::Unix {
        device: m.dev(),
        inode: m.ino(),
    })
}

#[cfg(windows)]
fn file_identity(file: &File) -> Result<FileIdentity, String> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };
    let mut output = std::mem::MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::uninit();
    // File owns a live handle. Win32 initializes the output only on success.
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), output.as_mut_ptr()) } == 0 {
        return Err("无法校验影片身份。".into());
    }
    let m = unsafe { output.assume_init() };
    Ok(FileIdentity::Windows {
        volume: m.dwVolumeSerialNumber,
        index: ((m.nFileIndexHigh as u64) << 32) | m.nFileIndexLow as u64,
    })
}

pub fn mime(format: &str) -> &'static str {
    match format {
        "mp4" => "video/mp4",
        "mkv" => "video/x-matroska",
        "m2ts" => "video/mp2t",
        _ => "application/octet-stream",
    }
}

pub async fn probe_file(
    path: PathBuf,
    tool: PathBuf,
    cancel: CancellationToken,
) -> Result<VideoSource, String> {
    probe_with_limits(path, tool, cancel, Duration::from_secs(30), 1024 * 1024).await
}

async fn probe_with_limits(
    path: PathBuf,
    tool: PathBuf,
    cancel: CancellationToken,
    timeout: Duration,
    output_limit: usize,
) -> Result<VideoSource, String> {
    if cancel.is_cancelled() {
        return Err("影片分析已取消。".into());
    }
    let mut source = tokio::task::spawn_blocking(move || VideoSource::open_import(path))
        .await
        .map_err(|_| "影片授权任务失败。")??;
    if !tool.is_absolute() || !tool.is_file() {
        return Err("FFprobe 不可用，请检查应用内媒体工具。".into());
    }
    let mut command = Command::new(tool);
    command.args([
        "-v", "error", "-protocol_whitelist", "file", "-format_whitelist", "mov,matroska,mpegts",
        "-enable_drefs", "0", "-use_absolute_path", "0", "-print_format", "json",
        "-show_format", "-show_streams", "-show_chapters", "-show_entries",
        "format=format_name,duration:stream=index,codec_type,codec_name,width,height,pix_fmt,color_transfer,color_space,color_range,color_primaries,channels,duration:stream_tags=language,title:stream_disposition=attached_pic:chapter=id,start_time,end_time",
        "-i",
    ]).arg(&source.path).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000); // CREATE_NO_WINDOW: analysis has no console UI.
    let mut child = command
        .spawn()
        .map_err(|_| "无法启动 FFprobe，请检查应用内媒体工具。")?;
    let stdout = child.stdout.take().ok_or("FFprobe 输出不可用。")?;
    let stderr = child.stderr.take().ok_or("FFprobe 错误输出不可用。")?;
    let result = {
        let collect = async {
            let (status, output, _) = tokio::try_join!(
                async {
                    child
                        .wait()
                        .await
                        .map_err(|_| "FFprobe 进程等待失败。".to_owned())
                },
                read_bounded(stdout, output_limit),
                read_bounded(stderr, 64 * 1024),
            )?;
            if !status.success() {
                return Err("FFprobe 无法分析此文件，请检查片源是否完整。".into());
            }
            Ok(output)
        };
        tokio::select! {
            biased;
            _ = cancel.cancelled() => Err("影片分析已取消。".to_owned()),
            result = tokio::time::timeout(timeout, collect) => result.unwrap_or_else(|_| Err("影片分析超时，信息未完成，可重试。".into())),
        }
    };
    if result.is_err() {
        let _ = child.start_kill();
        let _ = child.wait().await;
    }
    let output = result?;
    if cancel.is_cancelled() {
        return Err("影片分析已取消。".into());
    }
    let checked = source.clone();
    tokio::task::spawn_blocking(move || checked.open())
        .await
        .map_err(|_| "影片复核任务失败。")??;
    apply_probe(&mut source, &output)?;
    Ok(source)
}

async fn read_bounded(
    mut reader: impl AsyncRead + Unpin,
    maximum: usize,
) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 8192];
    loop {
        let count = reader
            .read(&mut buffer)
            .await
            .map_err(|_| "FFprobe 输出读取失败。")?;
        if count == 0 {
            return Ok(bytes);
        }
        if bytes.len().saturating_add(count) > maximum {
            return Err("FFprobe 输出过大，已停止分析。".into());
        }
        bytes.extend_from_slice(&buffer[..count]);
    }
}

fn apply_probe(source: &mut VideoSource, json: &[u8]) -> Result<(), String> {
    let data: serde_json::Value =
        serde_json::from_slice(json).map_err(|_| "FFprobe 返回了无效的分析结果。")?;
    let streams = data["streams"].as_array().ok_or("分析结果缺少媒体轨道。")?;
    if streams.len() > 512 {
        return Err("媒体轨道数量超出支持范围。".into());
    }
    let video = streams
        .iter()
        .find(|s| {
            s["codec_type"] == "video" && s["disposition"]["attached_pic"].as_u64() != Some(1)
        })
        .ok_or("此文件没有可播放的视频轨道。")?;
    let detected = data["format"]["format_name"]
        .as_str()
        .ok_or("无法识别影片容器。")?;
    let names: Vec<_> = detected.split(',').collect();
    source.info.format = if names.contains(&"matroska") {
        "mkv"
    } else if names.contains(&"mp4") {
        "mp4"
    } else if names.contains(&"mpegts") {
        "m2ts"
    } else {
        return Err("当前直传仅支持 MP4、MKV 和 M2TS 容器。".into());
    }
    .to_owned();
    source.info.duration_ms = data["format"]["duration"]
        .as_str()
        .or_else(|| video["duration"].as_str())
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|n| n.is_finite() && *n >= 0.0 && *n <= 359_999.0)
        .map(|n| (n * 1000.0).round() as u64);
    source.info.video_codec = string_field(video, "codec_name");
    source.info.width = u32_field(video, "width");
    source.info.height = u32_field(video, "height");
    source.info.pixel_format = string_field(video, "pix_fmt");
    source.info.color_transfer = string_field(video, "color_transfer");
    // These packages include the FFmpeg 7.1 10-bit overlay and HDR10 encoder.
    source.info.can_burn_hdr10 = cfg!(any(
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "windows", target_arch = "x86_64")
    )) && source.info.color_transfer.as_deref() == Some("smpte2084")
        && source.info.pixel_format.as_deref() == Some("yuv420p10le")
        && video["color_space"] == "bt2020nc"
        && video["color_primaries"] == "bt2020"
        && video["color_range"] == "tv"
        && [source.info.width, source.info.height]
            .iter()
            .all(|n| n.is_some_and(|v| v >= 16 && v % 2 == 0));
    let tracks = |kind: &str| {
        streams
            .iter()
            .filter(|s| s["codec_type"] == kind)
            .map(|s| VideoTrackInfo {
                index: u32_field(s, "index").unwrap_or(0),
                codec: string_field(s, "codec_name"),
                language: string_field(&s["tags"], "language"),
                title: string_field(&s["tags"], "title"),
                channels: u32_field(s, "channels"),
            })
            .collect()
    };
    source.info.audio_tracks = tracks("audio");
    source.info.subtitle_tracks = tracks("subtitle");
    Ok(())
}

fn string_field(value: &serde_json::Value, key: &str) -> Option<String> {
    value[key]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(|s| s.chars().take(512).collect())
}

fn u32_field(value: &serde_json::Value, key: &str) -> Option<u32> {
    value[key].as_u64().and_then(|n| n.try_into().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imported_source_rejects_replacement_and_changed_length() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("电影 空格.mkv");
        std::fs::write(&path, b"original").unwrap();
        let source = VideoSource::open_import(path.clone()).unwrap();
        assert_eq!(source.info.size, "8");
        assert!(source.open().is_ok());
        let public = serde_json::to_string(&source.info).unwrap();
        assert!(!public.contains(directory.path().to_str().unwrap()));
        std::fs::rename(&path, directory.path().join("old.mkv")).unwrap();
        std::fs::write(&path, b"replaced").unwrap();
        assert!(source.open().is_err());
        let updated = VideoSource::open_import(path.clone()).unwrap();
        std::fs::write(&path, b"changed length").unwrap();
        assert!(updated.open().is_err());
    }

    #[test]
    fn probe_extracts_real_streams_and_rejects_audio_only() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("电影.mkv");
        std::fs::write(&path, b"sample").unwrap();
        let mut source = VideoSource::open_import(path).unwrap();
        apply_probe(&mut source, br#"{"format":{"format_name":"matroska,webm","duration":"12.345"},"streams":[{"index":0,"codec_type":"video","codec_name":"hevc","width":3840,"height":2160,"pix_fmt":"yuv420p10le","color_transfer":"smpte2084"},{"index":1,"codec_type":"audio","codec_name":"truehd","channels":8,"tags":{"language":"eng"}},{"index":2,"codec_type":"subtitle","codec_name":"hdmv_pgs_subtitle","tags":{"language":"zho"}}]}"#).unwrap();
        assert_eq!(source.info.duration_ms, Some(12345));
        assert_eq!(source.info.video_codec.as_deref(), Some("hevc"));
        assert_eq!(source.info.width, Some(3840));
        assert_eq!(source.info.audio_tracks[0].channels, Some(8));
        assert_eq!(
            source.info.subtitle_tracks[0].language.as_deref(),
            Some("zho")
        );
        assert!(apply_probe(
            &mut source,
            br#"{"format":{"format_name":"matroska,webm"},"streams":[{"codec_type":"audio"}]}"#
        )
        .is_err());
        assert!(apply_probe(&mut source, b"not json").is_err());
    }

    #[test]
    fn sources_accept_only_supported_regular_files() {
        let directory = tempfile::tempdir().unwrap();
        assert!(VideoSource::open_import(directory.path().to_path_buf()).is_err());
        let path = directory.path().join("music.mp3");
        std::fs::write(&path, b"sample").unwrap();
        assert!(VideoSource::open_import(path).is_err());
        #[cfg(unix)]
        {
            let source = directory.path().join("source.mkv");
            let link = directory.path().join("link.mkv");
            std::fs::write(&source, b"sample").unwrap();
            std::os::unix::fs::symlink(&source, &link).unwrap();
            assert!(VideoSource::open_import(link).is_err());
        }
    }

    #[cfg(unix)]
    fn fake_probe(directory: &Path, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let tool = directory.join("fake-ffprobe");
        std::fs::write(&tool, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o700)).unwrap();
        tool
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn real_probe_process_is_bounded_and_uses_fixed_arguments() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("电影.mkv");
        std::fs::write(&path, b"sample").unwrap();
        let tool = fake_probe(
            directory.path(),
            r#"
test "$1" = '-v' || exit 2
test "$3" = '-protocol_whitelist' || exit 3
test "$4" = 'file' || exit 4
test "$5" = '-format_whitelist' || exit 5
test "$6" = 'mov,matroska,mpegts' || exit 6
printf '%s' '{"format":{"format_name":"matroska,webm","duration":"1.25"},"streams":[{"index":0,"codec_type":"video","codec_name":"hevc"}]}'
"#,
        );
        let result = probe_file(path.clone(), tool, CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(result.info.duration_ms, Some(1250));
        let tool = fake_probe(directory.path(), "printf '12345678901234567890'");
        let result = probe_with_limits(
            path.clone(),
            tool,
            CancellationToken::new(),
            Duration::from_secs(3),
            8,
        )
        .await;
        assert!(result.unwrap_err().contains("过大"));
        let tool = fake_probe(directory.path(), "exec /bin/sleep 30");
        let result = probe_with_limits(
            path,
            tool,
            CancellationToken::new(),
            Duration::from_millis(100),
            1024,
        )
        .await;
        assert!(result.unwrap_err().contains("超时"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cancelling_probe_kills_and_reaps_its_process() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("电影.mkv");
        std::fs::write(&path, b"sample").unwrap();
        let tool = fake_probe(
            directory.path(),
            "printf '%s' \"$$\" > \"$0.pid\"\nexec /bin/sleep 30",
        );
        let pid_file = directory.path().join("fake-ffprobe.pid");
        let cancel = CancellationToken::new();
        let task = tokio::spawn(probe_file(path, tool, cancel.clone()));
        let pid = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if let Ok(text) = tokio::fs::read_to_string(&pid_file).await {
                    if let Ok(pid) = text.parse::<i32>() {
                        break pid;
                    }
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        cancel.cancel();
        let result = tokio::time::timeout(Duration::from_secs(3), task)
            .await
            .unwrap()
            .unwrap();
        assert!(result.unwrap_err().contains("取消"));
        // A reaped child is no longer a live process or a zombie owned by us.
        assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ESRCH)
        );
    }
}
