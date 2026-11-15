use librqbit::{
    AddTorrent, AddTorrentOptions, AddTorrentResponse, ByteBufOwned, DhtSessionConfig, Magnet,
    Session, SessionOptions, ValidatedTorrentMetaV1Info,
};
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    time::Duration,
};
use tokio_util::sync::CancellationToken;

pub struct TransferUpdate {
    pub total_bytes: u64,
    pub downloaded_bytes: u64,
}

pub struct DownloadedFile {
    pub index: usize,
    pub path: PathBuf,
    pub size: u64,
}

// These are filesystem components supplied by strangers. Validate the decoded
// names on every platform, including Windows device names and alternate streams.
fn portable_component(name: &str) -> bool {
    if name.is_empty()
        || name.ends_with([' ', '.'])
        || name
            .chars()
            .any(|c| c.is_control() || r#"<>:"/\|?*"#.contains(c))
    {
        return false;
    }
    let stem = name
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end()
        .to_ascii_uppercase();
    let device_number = stem
        .strip_prefix("COM")
        .or_else(|| stem.strip_prefix("LPT"));
    !matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) && !matches!(
        device_number,
        Some("0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³")
    )
}

fn media_files(
    info: &ValidatedTorrentMetaV1Info<ByteBufOwned>,
) -> Result<Vec<DownloadedFile>, String> {
    let mut files = Vec::new();
    let mut filenames = HashSet::new();
    for (index, file) in info.iter_file_details().enumerate() {
        if index >= 4096 {
            return Err("种子文件数量过多，请使用较小的影视种子。".into());
        }
        if file.attrs().symlink
            || file.symlink_path.is_some()
            || !file
                .filename
                .iter_components()
                .all(|part| portable_component(&part))
        {
            return Err("种子含不安全的文件路径或符号链接。".into());
        }
        let path = file.filename.to_pathbuf();
        if !file.attrs().padding && !filenames.insert(path.to_string_lossy().to_lowercase()) {
            return Err("种子包含重复的文件路径。".into());
        }
        if !file.attrs().padding
            && file.len > 0
            && path
                .extension()
                .and_then(|s| s.to_str())
                .is_some_and(|ext| {
                    ["mp4", "mkv", "m2ts"]
                        .iter()
                        .any(|v| ext.eq_ignore_ascii_case(v))
                })
        {
            files.push(DownloadedFile {
                index,
                path,
                size: file.len,
            });
        }
    }
    if files.is_empty() {
        return Err("磁力资源中没有可下载的 MP4、MKV 或 M2TS 影片。".into());
    }
    Ok(files)
}

/// Download into a newly created, empty per-task directory. No peer-supplied
/// path is used outside the private staging directory until verification ends.
pub async fn download(
    magnet: &str,
    directory: &Path,
    cancel: CancellationToken,
    report: impl Fn(TransferUpdate) + Send + Sync,
) -> Result<Vec<DownloadedFile>, String> {
    let options = SessionOptions {
        dht: Some(DhtSessionConfig {
            persistence: None,
            ..Default::default()
        }),
        ..Default::default()
    };
    download_with_options(magnet, directory, cancel, report, options).await
}

async fn download_with_options(
    magnet: &str,
    directory: &Path,
    cancel: CancellationToken,
    report: impl Fn(TransferUpdate) + Send + Sync,
    mut options: SessionOptions,
) -> Result<Vec<DownloadedFile>, String> {
    // The manager passes a normalized v1 URI. Never give add_torrent a file
    // path, HTTP URL, or an unbounded BEP53 selection range from a raw URI.
    let uri = url::Url::parse(magnet).map_err(|_| "磁力链接无效。")?;
    if uri.scheme() != "magnet" || uri.query_pairs().any(|(key, _)| key == "so") {
        return Err("磁力链接无效。".into());
    }
    let expected_hash = Magnet::parse(magnet)
        .ok()
        .and_then(|m| m.as_id20())
        .ok_or("需要包含 BTIH 的磁力链接。")?;
    let folder_error = || "下载任务需要独占的空目录。".to_owned();
    let meta = fs::symlink_metadata(directory).map_err(|_| folder_error())?;
    if crate::video::is_link(&meta)
        || !meta.file_type().is_dir()
        || fs::read_dir(directory)
            .map_err(|_| folder_error())?
            .next()
            .is_some()
    {
        return Err(folder_error());
    }
    let directory = fs::canonicalize(directory).map_err(|_| folder_error())?;
    // The library may write boundary pieces into unselected files. Keep these
    // and every partially written file in a private, newly allocated staging area.
    let mut staging = tempfile::Builder::new();
    staging.prefix(".torrent-");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        staging.permissions(fs::Permissions::from_mode(0o700));
    }
    let stage = staging
        .tempdir_in(&directory)
        .map_err(|_| "无法创建下载临时目录。")?;
    options.cancellation_token = Some(cancel.child_token());
    let session = tokio::select! {
        biased;
        _ = cancel.cancelled() => return Err("磁力下载已取消。".into()),
        session = Session::new_with_opts(stage.path().to_owned(), options) =>
            session.map_err(|_| "无法启动 BitTorrent 网络，请检查网络连接后重试。")?,
    };
    let transfer = async {
        let lookup = session.add_torrent(
            AddTorrent::from_url(magnet),
            Some(AddTorrentOptions {
                list_only: true,
                sub_folder: Some(String::new()),
                ..Default::default()
            }),
        );
        let response = tokio::time::timeout(Duration::from_secs(180), lookup)
            .await
            .map_err(|_| "获取种子信息超时，资源可能暂时没有可连接的节点。")?
            .map_err(|_| "未能取得有效的种子信息，请检查磁力链接和网络。")?;
        let AddTorrentResponse::ListOnly(list) = response else {
            return Err("无法读取磁力资源文件列表。".to_owned());
        };
        if list.info_hash != expected_hash {
            return Err("种子信息与磁力链接的哈希不匹配。".into());
        }
        let files = media_files(&list.info)?;
        let total_bytes = files
            .iter()
            .try_fold(0u64, |n, f| n.checked_add(f.size))
            .ok_or("种子文件大小无效。")?;
        let available = crate::subtitle_cache::available_space(&directory)
            .map_err(|_| "无法查询所选下载目录的可用空间。")?;
        if available < total_bytes {
            const MIB: u64 = 1024 * 1024;
            return Err(format!(
                "影视文件需要 {} MiB，下载目录可用 {} MiB。请释放空间或更换目录。",
                total_bytes.div_ceil(MIB),
                available / MIB
            ));
        }
        report(TransferUpdate {
            downloaded_bytes: 0,
            total_bytes,
        });
        let response = session
            .add_torrent(
                AddTorrent::from_bytes(list.torrent_bytes),
                Some(AddTorrentOptions {
                    only_files: Some(files.iter().map(|f| f.index).collect()),
                    sub_folder: Some(String::new()),
                    initial_peers: Some(list.seen_peers),
                    ..Default::default()
                }),
            )
            .await
            .map_err(|_| "无法开始影视下载，请检查下载目录和剩余空间。")?;
        let handle = response.into_handle().ok_or("无法启动下载任务。")?;
        loop {
            let stats = handle.stats();
            if stats.error.is_some() {
                return Err("影视下载失败，请检查磁盘空间或重新尝试。".into());
            }
            let downloaded_bytes = files
                .iter()
                .map(|f| {
                    stats
                        .file_progress
                        .get(f.index)
                        .copied()
                        .unwrap_or(0)
                        .min(f.size)
                })
                .sum();
            report(TransferUpdate {
                downloaded_bytes,
                total_bytes,
            });
            if stats.finished {
                if downloaded_bytes != total_bytes {
                    return Err("影视文件的完整性检查未通过。".to_owned());
                }
                return Ok(files);
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    };
    let result = tokio::select! {
        biased;
        _ = cancel.cancelled() => Err("磁力下载已取消。".to_owned()),
        result = transfer => result,
    };
    session.stop().await;
    drop(session);
    if cancel.is_cancelled() {
        return Err("磁力下载已取消。".to_owned());
    }
    let mut files = result?;
    // Preserve torrent subdirectories (e.g. seasons of a series) without
    // exposing unselected files in the final directory.
    for file in &mut files {
        let src = stage.path().join(&file.path);
        let dest = directory.join(&file.path);
        let metadata = fs::symlink_metadata(&src).map_err(|_| "下载后的影片无法读取。")?;
        if crate::video::is_link(&metadata) || !metadata.is_file() || metadata.len() != file.size {
            return Err("下载后的影片大小或类型不符。".into());
        }
        let mut parent = directory.clone();
        for component in file.path.parent().ok_or("下载文件名无效。")?.components() {
            parent.push(component);
            match fs::create_dir(&parent) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    let metadata =
                        fs::symlink_metadata(&parent).map_err(|_| "无法读取影片目录。")?;
                    if !metadata.is_dir() || crate::video::is_link(&metadata) {
                        return Err("影片目录已变更，无法保存。".into());
                    }
                }
                Err(_) => return Err("无法创建影片目录。".into()),
            }
        }
        tempfile::TempPath::try_from_path(src)
            .map_err(|_| "找不到已下载的影片。")?
            .persist_noclobber(&dest)
            .map_err(|_| "无法保存影片，目标可能已存在。")?;
        file.path = dest;
    }
    stage.close().map_err(|_| "无法清理下载临时文件。")?;
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use librqbit::{CreateTorrentOptions, ListenerOptions};
    use std::sync::Mutex;

    #[test]
    fn unsafe_torrent_components_are_rejected_before_opening_any_files() {
        for value in [
            "",
            ".",
            "..",
            "../clip.mkv",
            r"C:\film.mkv",
            r"foo\bar",
            "AUX.mkv",
            "COM¹.mp4",
            "NUL .mp4",
            "clip.mkv:stream",
            "film.",
        ] {
            assert!(!portable_component(value), "{value}");
        }
        assert!(portable_component("影片 01.mkv"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn magnet_downloads_only_verified_movies_from_loopback_seed() {
        let seed_root = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        let source = seed_root.path().join("series");
        fs::create_dir(&source).unwrap();
        let movie = include_bytes!("../tests/fixtures/generated-video.mkv");
        let bonus = vec![0x85; 145_500]; // Exercise multiple verified pieces.
        fs::write(source.join("Episode 1.MKV"), movie).unwrap();
        fs::write(source.join("notes.txt"), b"not a movie").unwrap();
        fs::write(source.join("花絮.mp4"), &bonus).unwrap();
        let offline = || SessionOptions {
            dht: None,
            disable_trackers: true,
            disable_local_service_discovery: true,
            ipv4_only: true,
            ..Default::default()
        };
        let mut options = offline();
        options.listen = Some(ListenerOptions {
            listen_addr: ([127, 0, 0, 1], 0).into(),
            ipv4_only: true,
            ..Default::default()
        });
        let seeder = Session::new_with_opts(seed_root.path().into(), options)
            .await
            .unwrap();
        // The client learns the seeder address only from the magnet's local HTTP tracker.
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let url = format!("http://{}/announce", listener.local_addr().unwrap());
        let mut compact = b"d8:intervali60e5:peers6:".to_vec();
        compact.extend([127, 0, 0, 1]);
        compact.extend(seeder.listen_addr().unwrap().port().to_be_bytes());
        compact.push(b'e');
        let service = axum::Router::new().route(
            "/announce",
            axum::routing::get(move || {
                let bytes = compact.clone();
                async move { bytes }
            }),
        );
        let tracker = tokio::spawn(async move { axum::serve(listener, service).await.unwrap() });
        let (torrent, seed_handle) = seeder
            .create_and_serve_torrent(
                &source,
                CreateTorrentOptions {
                    piece_length: Some(16384),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        seed_handle.wait_until_completed().await.unwrap();
        let mut uri = url::Url::parse(&torrent.as_magnet().to_string()).unwrap();
        uri.query_pairs_mut().append_pair("tr", &url);
        let received = Mutex::new(Vec::new());
        let files = tokio::time::timeout(
            Duration::from_secs(35),
            download_with_options(
                uri.as_str(),
                dst.path(),
                CancellationToken::new(),
                |u| {
                    received
                        .lock()
                        .unwrap()
                        .push((u.downloaded_bytes, u.total_bytes))
                },
                SessionOptions {
                    disable_trackers: false,
                    ..offline()
                },
            ),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(fs::read(dst.path().join("Episode 1.MKV")).unwrap(), movie);
        assert_eq!(fs::read(dst.path().join("花絮.mp4")).unwrap(), bonus);
        assert_eq!(fs::read_dir(dst.path()).unwrap().count(), 2);
        assert!(files.iter().all(|f| f.path.starts_with(dst.path())
            || f.path.starts_with(fs::canonicalize(dst.path()).unwrap())));
        let expected = movie.len() as u64 + bonus.len() as u64;
        assert_eq!(
            received.lock().unwrap().last().copied(),
            Some((expected, expected))
        );
        let film = dst.path().join("Episode 1.MKV");
        assert!(crate::video::VideoSource::open_import(film.clone()).is_ok());
        if let Ok(probe) = crate::video_tools::ffprobe_path() {
            let imported = crate::video::probe_file(film, probe, CancellationToken::new())
                .await
                .unwrap();
            assert_eq!(imported.info.duration_ms, Some(2000));
            assert_eq!(imported.info.width, Some(16));
        }

        let cancelled = tempfile::tempdir().unwrap();
        let token = CancellationToken::new();
        let result = tokio::time::timeout(
            Duration::from_secs(35),
            download_with_options(
                uri.as_str(),
                cancelled.path(),
                token.clone(),
                |_| token.cancel(),
                SessionOptions {
                    disable_trackers: false,
                    ..offline()
                },
            ),
        )
        .await
        .unwrap();
        assert_eq!(result.err().as_deref(), Some("磁力下载已取消。"));
        assert_eq!(fs::read_dir(cancelled.path()).unwrap().count(), 0);
        seeder.stop().await;
        tracker.abort();
    }
}
