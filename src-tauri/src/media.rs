//! LAN media endpoint. Control commands are available only through Tauri IPC.

use crate::library::{mime, Library, Track};
use crate::video::VideoSource;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, Method, Response, StatusCode};
use axum::routing::get;
use axum::Router;
use futures_util::StreamExt;
use std::collections::HashMap;
use std::io::SeekFrom;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio_util::io::ReaderStream;
use tokio_util::sync::CancellationToken;

#[path = "media_video.rs"]
mod video_transfer;
pub use video_transfer::{VideoRequestSnapshot, VideoTransferMonitor, VideoTransferSnapshot};

#[derive(Clone)]
struct MediaState {
    library: Arc<Mutex<Library>>,
    token: String,
    shutdown: CancellationToken,
    videos: Arc<Mutex<HashMap<String, VideoRegistration>>>,
}

#[derive(Clone, Debug)]
struct VideoRegistration {
    source: VideoSource,
    cancellation: CancellationToken,
    transfer: VideoTransferMonitor,
}

#[derive(Clone, Debug)]
pub struct VideoGrant {
    pub id: String,
    pub url: String,
    pub transfer: VideoTransferMonitor,
}

#[derive(Clone, Debug)]
pub struct MediaServer {
    pub port: u16,
    pub token: String,
    videos: Arc<Mutex<HashMap<String, VideoRegistration>>>,
    shutdown: CancellationToken,
}

impl MediaServer {
    /// Caller validates the source off the async worker before registering it.
    /// Each HTTP request validates the opened file again; no path comes from the URL.
    pub fn register_video(&self, host: &str, source: VideoSource) -> Result<VideoGrant, String> {
        let address = host
            .trim_matches(['[', ']'])
            .parse::<std::net::IpAddr>()
            .map_err(|_| "视频服务地址无效。")?;
        if address.is_unspecified() || address.is_multicast() || self.shutdown.is_cancelled() {
            return Err("视频服务地址不可用。".into());
        }
        if crate::video::mime(&source.info.format) == "application/octet-stream"
            || uuid::Uuid::parse_str(&source.info.id).is_err()
        {
            return Err("视频资源信息无效。".into());
        }
        let id = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        let authority = match address {
            std::net::IpAddr::V4(ip) => ip.to_string(),
            std::net::IpAddr::V6(ip) => format!("[{ip}]"),
        };
        let url = format!(
            "http://{authority}:{}/video/{id}/{}.{}",
            self.port, source.info.id, source.info.format
        );
        let cancellation = self.shutdown.child_token();
        let transfer = VideoTransferMonitor::new(cancellation.clone());
        self.videos
            .lock()
            .map_err(|_| "视频资源登记暂不可用。")?
            .insert(
                id.clone(),
                VideoRegistration {
                    source,
                    cancellation,
                    transfer: transfer.clone(),
                },
            );
        Ok(VideoGrant { id, url, transfer })
    }

    pub fn revoke_video(&self, id: &str) {
        if let Ok(mut videos) = self.videos.lock() {
            if let Some(registration) = videos.remove(id) {
                registration.cancellation.cancel();
            }
        }
    }

    /// Evidence of bytes yielded to an authorized GET body, not of client receipt or decoding.
    pub fn video_evidence(&self, id: &str) -> bool {
        self.videos
            .lock()
            .ok()
            .and_then(|videos| videos.get(id).map(|r| r.transfer.has_bytes()))
            .unwrap_or(false)
    }

    pub fn url(&self, host: &str, track: &Track) -> String {
        let host = if host.contains(':') && !host.starts_with('[') {
            format!("[{host}]")
        } else {
            host.to_owned()
        };
        format!(
            "http://{host}:{}/media/{}/{}.{}",
            self.port, self.token, track.id, track.format
        )
    }
}

pub async fn start(
    library: Arc<Mutex<Library>>,
    shutdown: CancellationToken,
) -> Result<MediaServer, String> {
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::UNSPECIFIED, 0))
        .await
        .map_err(|error| format!("无法启动音乐推流服务，请检查防火墙和网络权限：{error}"))?;
    let port = listener
        .local_addr()
        .map_err(|error| format!("无法读取推流端口：{error}"))?
        .port();
    let token = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let videos = Arc::new(Mutex::new(HashMap::new()));
    let state = MediaState {
        library,
        token: token.clone(),
        shutdown: shutdown.clone(),
        videos: videos.clone(),
    };
    let router = Router::new()
        .route(
            "/media/{token}/{filename}",
            get(serve_media).head(serve_media),
        )
        .route(
            "/video/{token}/{filename}",
            get(serve_video).head(serve_video),
        )
        .with_state(state);
    let server_shutdown = shutdown.clone();
    tokio::spawn(async move {
        // Active body streams observe the same token, releasing open files
        // before graceful shutdown waits for their connections to finish.
        let _ = axum::serve(listener, router)
            .with_graceful_shutdown(shutdown.cancelled_owned())
            .await;
    });
    Ok(MediaServer {
        port,
        token,
        videos,
        shutdown: server_shutdown,
    })
}

async fn serve_video(
    State(state): State<MediaState>,
    Path((token, filename)): Path<(String, String)>,
    method: Method,
    headers: HeaderMap,
) -> Response<Body> {
    let registration = state
        .videos
        .lock()
        .ok()
        .and_then(|videos| videos.get(&token).cloned());
    let Some(registration) = registration else {
        return empty_response(StatusCode::NOT_FOUND);
    };
    if registration.cancellation.is_cancelled()
        || filename
            != format!(
                "{}.{}",
                registration.source.info.id, registration.source.info.format
            )
    {
        return empty_response(StatusCode::NOT_FOUND);
    }
    let transfer = &registration.transfer;
    transfer.requested(method.as_str(), headers.contains_key(header::RANGE));
    let Some(permit) = transfer.try_read() else {
        transfer.response(method.as_str(), 503, None);
        let mut response = empty_response(StatusCode::SERVICE_UNAVAILABLE);
        response
            .headers_mut()
            .insert(header::RETRY_AFTER, "1".parse().unwrap());
        return response;
    };
    let source = registration.source.clone();
    // The opening task owns the permit if the request is cancelled during disk I/O.
    let (file, permit) =
        match tokio::task::spawn_blocking(move || source.open().map(|file| (file, permit))).await {
            Ok(Ok(opened)) => opened,
            _ => {
                transfer.response(method.as_str(), 404, None);
                return empty_response(StatusCode::NOT_FOUND);
            }
        };
    if registration.cancellation.is_cancelled() {
        transfer.response(method.as_str(), 404, None);
        return empty_response(StatusCode::NOT_FOUND);
    }
    let size = registration.source.length();
    // This route supplies no strong validator. If-Range therefore always falls
    // back to the complete representation, as required for unmatched validators.
    let requested = if method == Method::HEAD
        || headers.contains_key(header::IF_RANGE)
        || headers.get_all(header::RANGE).iter().count() > 1
    {
        None
    } else {
        headers
            .get(header::RANGE)
            .and_then(|value| value.to_str().ok())
    };
    let requested = requested.filter(|value| value.starts_with("bytes=") && !value.contains(','));
    let selected = match parse_range(requested, size) {
        Ok(value) => value,
        Err(()) => {
            transfer.response(method.as_str(), 416, None);
            return video_response(
                StatusCode::RANGE_NOT_SATISFIABLE,
                &registration.source.info.format,
                0,
                Some(format!("bytes */{size}")),
                Body::empty(),
            );
        }
    };
    let (start, length, status, range) = match selected {
        Some((start, end)) => (
            start,
            end - start + 1,
            StatusCode::PARTIAL_CONTENT,
            Some(format!("bytes {start}-{end}/{size}")),
        ),
        None => (0, size, StatusCode::OK, None),
    };
    if method == Method::HEAD {
        transfer.response(method.as_str(), status.as_u16(), None);
        return video_response(
            status,
            &registration.source.info.format,
            length,
            range,
            Body::empty(),
        );
    }
    let mut file = tokio::fs::File::from_std(file);
    if file.seek(SeekFrom::Start(start)).await.is_err() {
        transfer.response(method.as_str(), 404, None);
        return empty_response(StatusCode::NOT_FOUND);
    }
    transfer.response(method.as_str(), status.as_u16(), selected);
    let stream = video_transfer::stream_file(
        file,
        length,
        transfer.clone(),
        permit,
        registration.cancellation,
    );
    video_response(
        status,
        &registration.source.info.format,
        length,
        range,
        Body::from_stream(stream),
    )
}

fn video_response(
    status: StatusCode,
    format: &str,
    length: u64,
    range: Option<String>,
    body: Body,
) -> Response<Body> {
    let mut builder = Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, crate::video::mime(format))
        .header(header::CONTENT_LENGTH, length)
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::CACHE_CONTROL, "private, no-store")
        .header("x-content-type-options", "nosniff")
        .header("transferMode.dlna.org", "Streaming")
        .header("contentFeatures.dlna.org", "DLNA.ORG_OP=01;DLNA.ORG_CI=0");
    if let Some(range) = range {
        builder = builder.header(header::CONTENT_RANGE, range);
    }
    builder
        .body(body)
        .unwrap_or_else(|_| empty_response(StatusCode::INTERNAL_SERVER_ERROR))
}

fn token_matches(actual: &str, expected: &str) -> bool {
    if actual.len() != expected.len() {
        return false;
    }
    actual
        .bytes()
        .zip(expected.bytes())
        .fold(0u8, |difference, (a, b)| difference | (a ^ b))
        == 0
}

async fn serve_media(
    State(state): State<MediaState>,
    Path((token, filename)): Path<(String, String)>,
    method: Method,
    headers: HeaderMap,
) -> Response<Body> {
    if !token_matches(&token, &state.token) {
        return empty_response(StatusCode::NOT_FOUND);
    }
    let Some((id, format)) = filename.split_once('.') else {
        return empty_response(StatusCode::NOT_FOUND);
    };
    let id = id.to_owned();
    let format = format.to_owned();
    let library = state.library.clone();
    // Blocking disk operations and the standard Mutex remain off async workers.
    // Only an already-validated File leaves this closure; the lock is released
    // before waiting for network I/O or constructing a response stream.
    let opened = tokio::task::spawn_blocking(move || -> Result<_, String> {
        let library = library
            .lock()
            .map_err(|_| "音乐库暂时无法访问。".to_owned())?;
        let track = library.get(&id)?;
        if track.format != format {
            return Err("音乐链接无效。".into());
        }
        let file = library.open_track(&id)?;
        let size = file
            .metadata()
            .map_err(|error| format!("音乐文件无法读取：{error}"))?
            .len();
        Ok((track, file, size))
    })
    .await;
    let (track, file, size) = match opened {
        Ok(Ok(value)) => value,
        _ => return empty_response(StatusCode::NOT_FOUND),
    };
    let range = if headers.get_all(header::RANGE).iter().count() > 1 {
        Err(())
    } else {
        headers
            .get(header::RANGE)
            .map(|value| value.to_str().map_err(|_| ()))
            .transpose()
            .and_then(|value| parse_range(value, size))
    };
    let selected = match range {
        Ok(range) => range,
        Err(()) => {
            drop(file);
            return media_response(
                StatusCode::RANGE_NOT_SATISFIABLE,
                &track.format,
                0,
                Some(format!("bytes */{size}")),
                Body::empty(),
            );
        }
    };
    let (start, length, status, content_range) = match selected {
        Some((start, end)) => (
            start,
            end - start + 1,
            StatusCode::PARTIAL_CONTENT,
            Some(format!("bytes {start}-{end}/{size}")),
        ),
        None => (0, size, StatusCode::OK, None),
    };
    if method == Method::HEAD || length == 0 {
        drop(file);
        return media_response(status, &track.format, length, content_range, Body::empty());
    }
    let mut file = tokio::fs::File::from_std(file);
    if file.seek(SeekFrom::Start(start)).await.is_err() {
        return empty_response(StatusCode::NOT_FOUND);
    }
    let stream = ReaderStream::with_capacity(file.take(length), 64 * 1024)
        .take_until(state.shutdown.cancelled_owned());
    media_response(
        status,
        &track.format,
        length,
        content_range,
        Body::from_stream(stream),
    )
}

fn media_response(
    status: StatusCode,
    format: &str,
    length: u64,
    range: Option<String>,
    body: Body,
) -> Response<Body> {
    let mut builder = Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, mime(format))
        .header(header::CONTENT_LENGTH, length)
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::CACHE_CONTROL, "private, no-store")
        .header("x-content-type-options", "nosniff")
        .header("transferMode.dlna.org", "Streaming")
        .header(
            "contentFeatures.dlna.org",
            "DLNA.ORG_OP=01;DLNA.ORG_CI=0;DLNA.ORG_FLAGS=01700000000000000000000000000000",
        );
    if let Some(range) = range {
        builder = builder.header(header::CONTENT_RANGE, range);
    }
    builder
        .body(body)
        .unwrap_or_else(|_| empty_response(StatusCode::INTERNAL_SERVER_ERROR))
}

fn empty_response(status: StatusCode) -> Response<Body> {
    let mut response = Response::new(Body::empty());
    *response.status_mut() = status;
    response
}

fn parse_range(value: Option<&str>, size: u64) -> Result<Option<(u64, u64)>, ()> {
    let Some(value) = value else {
        return Ok(None);
    };
    if size == 0 {
        return Err(());
    }
    let value = value.strip_prefix("bytes=").ok_or(())?;
    let (left, right) = value.split_once('-').ok_or(())?;
    if left.is_empty() {
        let suffix = unsigned_decimal(right)?;
        if suffix == 0 {
            return Err(());
        }
        return Ok(Some((size.saturating_sub(suffix), size - 1)));
    }
    let start = unsigned_decimal(left)?;
    let end = if right.is_empty() {
        size - 1
    } else {
        unsigned_decimal(right)?.min(size - 1)
    };
    if start >= size || end < start {
        return Err(());
    }
    Ok(Some((start, end)))
}

fn unsigned_decimal(value: &str) -> Result<u64, ()> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(());
    }
    value.parse().map_err(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use tokio::io::AsyncWriteExt;

    fn sparse_file(path: &std::path::Path, length: u64) -> std::fs::File {
        let file = std::fs::File::create(path).unwrap();
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            use windows_sys::Win32::System::{Ioctl::FSCTL_SET_SPARSE, IO::DeviceIoControl};
            let mut returned = 0;
            // The test owns this ordinary file. Mark it sparse before extending
            // it so the >4 GiB offset tests do not allocate gigabytes on NTFS.
            let ok = unsafe {
                DeviceIoControl(
                    file.as_raw_handle(),
                    FSCTL_SET_SPARSE,
                    std::ptr::null(),
                    0,
                    std::ptr::null_mut(),
                    0,
                    &mut returned,
                    std::ptr::null_mut(),
                )
            };
            assert_ne!(
                ok,
                0,
                "cannot create sparse test file: {}",
                std::io::Error::last_os_error()
            );
        }
        file.set_len(length).unwrap();
        file
    }

    struct Fixture {
        _directory: tempfile::TempDir,
        library: Arc<Mutex<Library>>,
        server: MediaServer,
        track: Track,
        source: std::path::PathBuf,
        shutdown: CancellationToken,
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            self.shutdown.cancel();
        }
    }

    async fn fixture(content: &[u8]) -> Fixture {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("歌曲.mp3");
        std::fs::write(&source, content).unwrap();
        let mut library = Library::load(directory.path().join("data")).unwrap();
        library.import_paths(std::slice::from_ref(&source)).unwrap();
        let track = library.tracks()[0].clone();
        let library = Arc::new(Mutex::new(library));
        let shutdown = CancellationToken::new();
        let server = start(library.clone(), shutdown.clone()).await.unwrap();
        Fixture {
            _directory: directory,
            library,
            server,
            track,
            source,
            shutdown,
        }
    }

    impl Fixture {
        fn path(&self) -> String {
            format!("/media/{}/{}.mp3", self.server.token, self.track.id)
        }

        async fn request(
            &self,
            method: &str,
            path: &str,
            headers: &str,
        ) -> (u16, HashMap<String, String>, Vec<u8>) {
            let mut connection =
                tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, self.server.port))
                    .await
                    .unwrap();
            connection.write_all(format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n{headers}\r\n", self.server.port).as_bytes()).await.unwrap();
            let mut response = Vec::new();
            tokio::time::timeout(
                std::time::Duration::from_secs(5),
                connection.read_to_end(&mut response),
            )
            .await
            .unwrap()
            .unwrap();
            let split = response
                .windows(4)
                .position(|window| window == b"\r\n\r\n")
                .unwrap();
            let text = std::str::from_utf8(&response[..split]).unwrap();
            let mut lines = text.split("\r\n");
            let status = lines
                .next()
                .unwrap()
                .split_whitespace()
                .nth(1)
                .unwrap()
                .parse()
                .unwrap();
            let headers = lines
                .map(|line| {
                    let (name, value) = line.split_once(':').unwrap();
                    (name.to_ascii_lowercase(), value.trim().to_owned())
                })
                .collect();
            (status, headers, response[split + 4..].to_vec())
        }
    }

    #[test]
    fn range_parser_rejects_invalid_and_overflowing_inputs() {
        assert_eq!(parse_range(None, 0), Ok(None));
        for invalid in [
            "bytes=",
            "bytes=-0",
            "bytes=16-",
            "bytes=8-2",
            "bytes=0-1,4-5",
            "bytes=+1-2",
            "items=1-2",
            "bytes=18446744073709551616-",
        ] {
            assert_eq!(parse_range(Some(invalid), 16), Err(()), "{invalid}");
        }
        assert_eq!(parse_range(Some("bytes=0-"), 0), Err(()));
        assert_eq!(parse_range(Some("bytes=-99"), 16), Ok(Some((0, 15))));
    }

    #[tokio::test]
    async fn video_http_bytes_head_and_revocation_are_independent_of_music() {
        let fixture = fixture(b"music").await;
        let path = fixture._directory.path().join("电影.mkv");
        std::fs::write(&path, b"0123456789abcdef").unwrap();
        let source = crate::video::VideoSource::open_import(path.clone()).unwrap();
        let grant = fixture.server.register_video("127.0.0.1", source).unwrap();
        let video_path = url::Url::parse(&grant.url).unwrap().path().to_owned();
        let (status, headers, body) = fixture
            .request("GET", &video_path, "Range: bytes=2-5\r\n")
            .await;
        assert_eq!((status, body), (206, b"2345".to_vec()));
        assert_eq!(headers["content-type"], "video/x-matroska");
        assert!(fixture.server.video_evidence(&grant.id));
        let (status, headers, body) = fixture
            .request("HEAD", &video_path, "Range: bytes=2-5\r\n")
            .await;
        assert_eq!(
            (status, body.len(), headers["content-length"].as_str()),
            (200, 0, "16")
        );
        for range in ["bytes=0-1,4-5", "items=1-2"] {
            let (status, _, body) = fixture
                .request("GET", &video_path, &format!("Range: {range}\r\n"))
                .await;
            assert_eq!((status, body), (200, b"0123456789abcdef".to_vec()));
        }
        let (status, headers, _) = fixture
            .request("GET", &video_path, "Range: bytes=16-\r\n")
            .await;
        assert_eq!(status, 416);
        assert_eq!(headers["content-range"], "bytes */16");
        let (status, _, body) = fixture
            .request(
                "GET",
                &video_path,
                "Range: bytes=2-5\r\nIf-Range: \"unknown\"\r\n",
            )
            .await;
        assert_eq!((status, body), (200, b"0123456789abcdef".to_vec()));
        fixture.server.revoke_video(&grant.id);
        assert_eq!(fixture.request("GET", &video_path, "").await.0, 404);
        assert_eq!(
            fixture.request("GET", &fixture.path(), "").await.2,
            b"music"
        );
        let replacement = fixture
            .server
            .register_video(
                "127.0.0.1",
                crate::video::VideoSource::open_import(path.clone()).unwrap(),
            )
            .unwrap();
        std::fs::rename(&path, path.with_extension("old")).unwrap();
        std::fs::write(&path, b"private").unwrap();
        assert_eq!(
            fixture
                .request("GET", url::Url::parse(&replacement.url).unwrap().path(), "")
                .await
                .0,
            404
        );
    }

    #[tokio::test]
    async fn video_http_reads_bytes_beyond_four_gib_without_buffering_file() {
        use std::io::{Seek, Write};
        let fixture = fixture(b"music").await;
        let path = fixture._directory.path().join("large.mkv");
        let offset = (1u64 << 32) + 7;
        let mut file = sparse_file(&path, offset + 8);
        file.seek(SeekFrom::Start(offset)).unwrap();
        file.write_all(b"TAILBYTE").unwrap();
        drop(file);
        let grant = fixture
            .server
            .register_video(
                "127.0.0.1",
                crate::video::VideoSource::open_import(path).unwrap(),
            )
            .unwrap();
        let url = url::Url::parse(&grant.url).unwrap();
        let (status, headers, body) = fixture
            .request("GET", url.path(), &format!("Range: bytes={offset}-\r\n"))
            .await;
        assert_eq!((status, body), (206, b"TAILBYTE".to_vec()));
        assert_eq!(
            headers["content-range"],
            format!("bytes {offset}-{}/{total}", offset + 7, total = offset + 8)
        );
        assert_eq!(headers["content-length"], "8");
        let stats = grant.transfer.snapshot();
        assert_eq!(stats.bytes_streamed, "8");
        let range = stats.last_request.unwrap();
        assert_eq!(range.range_start, Some(offset.to_string()));
        assert_eq!(range.range_end, Some((offset + 7).to_string()));
    }

    #[tokio::test]
    async fn video_transfer_diagnostics_count_real_bytes_and_normalized_ranges_only() {
        let fixture = fixture(b"music").await;
        let path = fixture._directory.path().join("private movie.mkv");
        let content: Vec<u8> = (0..70_000).map(|n| (n % 251) as u8).collect();
        std::fs::write(&path, &content).unwrap();
        let grant = fixture
            .server
            .register_video(
                "127.0.0.1",
                crate::video::VideoSource::open_import(path.clone()).unwrap(),
            )
            .unwrap();
        let video_path = url::Url::parse(&grant.url).unwrap().path().to_owned();
        assert_eq!(
            fixture
                .request("HEAD", &video_path, "Range: bytes=2-5\r\n")
                .await
                .0,
            200
        );
        let initial = grant.transfer.snapshot();
        assert_eq!(initial.head_requests, 1);
        assert_eq!(initial.bytes_streamed, "0");
        assert!(!fixture.server.video_evidence(&grant.id));
        for _ in 0..4 {
            let requests =
                (0..4).map(|_| fixture.request("GET", &video_path, "Range: bytes=65530-65550\r\n"));
            for (status, _, body) in futures_util::future::join_all(requests).await {
                assert_eq!(status, 206);
                assert_eq!(body, content[65530..=65550]);
            }
        }
        let served = grant.transfer.snapshot();
        assert_eq!(served.get_requests, 16);
        assert_eq!(served.range_requests, 17);
        assert_eq!(served.bytes_streamed, (16 * 21).to_string());
        assert!(fixture.server.video_evidence(&grant.id));
        assert_eq!(served.active_streams, 0);
        assert!(served.peak_active_streams <= 8);
        let last = served.last_request.unwrap();
        assert_eq!(last.method, "GET");
        assert_eq!(last.status, 206);
        assert_eq!(last.range_start.as_deref(), Some("65530"));
        assert_eq!(last.range_end.as_deref(), Some("65550"));
        assert_eq!(
            fixture
                .request("GET", &video_path, "Range: bytes=70000-\r\n")
                .await
                .0,
            416
        );
        fixture.server.revoke_video(&grant.id);
        let stopped = grant.transfer.snapshot();
        assert!(stopped.revoked);
        assert_eq!(stopped.rejected_requests, 1);
        assert_eq!(stopped.bytes_streamed, (16 * 21).to_string());
        assert_eq!(stopped.last_request.as_ref().unwrap().status, 416);
        assert!(stopped.last_request.as_ref().unwrap().range_start.is_none());
        let report = serde_json::to_string(&stopped).unwrap();
        for sensitive in [
            grant.id.as_str(),
            grant.url.as_str(),
            path.to_str().unwrap(),
            "private movie",
        ] {
            assert!(
                !report.contains(sensitive),
                "transfer reports must not include source or access identifiers"
            );
        }
    }

    #[tokio::test]
    async fn cancelled_backpressured_video_closes_reader_without_waiting_for_client() {
        for shutdown in [false, true] {
            let fixture = fixture(b"music").await;
            let path = fixture._directory.path().join("unread.mkv");
            drop(sparse_file(&path, 1u64 << 33));
            let grant = fixture
                .server
                .register_video(
                    "127.0.0.1",
                    crate::video::VideoSource::open_import(path.clone()).unwrap(),
                )
                .unwrap();
            let video_path = url::Url::parse(&grant.url).unwrap().path().to_owned();
            let mut reader = tokio::net::TcpStream::connect((
                std::net::Ipv4Addr::LOCALHOST,
                fixture.server.port,
            ))
            .await
            .unwrap();
            reader
                .write_all(
                    format!(
                        "GET {video_path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            let mut headers = Vec::new();
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                while !headers.ends_with(b"\r\n\r\n") {
                    headers.push(reader.read_u8().await.unwrap());
                }
            })
            .await
            .unwrap();
            assert!(headers.starts_with(b"HTTP/1.1 200"));
            assert_eq!(grant.transfer.snapshot().active_streams, 1);
            // Keep the client connected and never drain the response body.
            if shutdown {
                fixture.shutdown.cancel();
            } else {
                fixture.server.revoke_video(&grant.id);
            }
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                while grant.transfer.snapshot().active_streams != 0 {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            let stopped = grant.transfer.snapshot();
            assert!(stopped.revoked);
            assert!(stopped.bytes_streamed.parse::<u64>().unwrap() < 1u64 << 33);
            #[cfg(windows)]
            {
                use std::os::windows::fs::OpenOptionsExt;
                // A new exclusive handle can only open after the old reader releases its handle.
                tokio::time::timeout(std::time::Duration::from_secs(5), async {
                    loop {
                        if std::fs::OpenOptions::new()
                            .read(true)
                            .share_mode(0)
                            .open(&path)
                            .is_ok()
                        {
                            break;
                        }
                        tokio::task::yield_now().await;
                    }
                })
                .await
                .unwrap();
            }
            std::fs::rename(&path, path.with_extension("closed")).unwrap();
            drop(reader);
        }
    }

    #[tokio::test]
    async fn revoking_video_cancels_an_active_large_response() {
        let fixture = fixture(b"music").await;
        let path = fixture._directory.path().join("stream.mkv");
        let length = 1u64 << 33;
        let file = sparse_file(&path, length);
        drop(file);
        let grant = fixture
            .server
            .register_video(
                "127.0.0.1",
                crate::video::VideoSource::open_import(path).unwrap(),
            )
            .unwrap();
        let response = reqwest::Client::new().get(&grant.url).send().await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let mut stream = response.bytes_stream();
        let first = stream.next().await.unwrap().unwrap();
        assert!(!first.is_empty());
        fixture.server.revoke_video(&grant.id);
        let total = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            let mut total = first.len() as u64;
            while let Some(Ok(chunk)) = stream.next().await {
                total += chunk.len() as u64;
            }
            total
        })
        .await
        .unwrap();
        assert!(
            total < length,
            "revoked stream must not deliver the whole source"
        );
    }

    #[tokio::test]
    async fn slow_video_readers_have_a_bounded_per_grant_limit_without_blocking_music() {
        let fixture = fixture(b"music").await;
        let path = fixture._directory.path().join("slow.mkv");
        drop(sparse_file(&path, 1u64 << 33));
        let grant = fixture
            .server
            .register_video(
                "127.0.0.1",
                crate::video::VideoSource::open_import(path).unwrap(),
            )
            .unwrap();
        let video_path = url::Url::parse(&grant.url).unwrap().path().to_owned();
        let mut readers = Vec::new();
        for _ in 0..8 {
            let mut connection = tokio::net::TcpStream::connect((
                std::net::Ipv4Addr::LOCALHOST,
                fixture.server.port,
            ))
            .await
            .unwrap();
            connection
                .write_all(
                    format!(
                        "GET {video_path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            let mut headers = Vec::new();
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                while !headers.ends_with(b"\r\n\r\n") {
                    headers.push(connection.read_u8().await.unwrap());
                }
            })
            .await
            .unwrap();
            assert!(headers.starts_with(b"HTTP/1.1 200"));
            readers.push(connection);
        }
        let (status, headers, _) = fixture.request("HEAD", &video_path, "").await;
        assert_eq!(status, 503, "a ninth read must not allocate another stream");
        assert_eq!(headers["retry-after"], "1");
        assert_eq!(
            fixture.request("GET", &fixture.path(), "").await.2,
            b"music"
        );
        drop(readers);
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if fixture.request("HEAD", &video_path, "").await.0 == 200 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn real_http_stream_head_and_ranges_match_original_bytes() {
        let fixture = fixture(b"0123456789abcdef").await;
        let path = fixture.path();
        assert_eq!(
            fixture.server.url("192.168.1.20", &fixture.track),
            format!("http://192.168.1.20:{}{path}", fixture.server.port)
        );
        let (status, headers, body) = fixture
            .request("GET", &path, "getcontentFeatures.dlna.org: 1\r\n")
            .await;
        assert_eq!((status, body), (200, b"0123456789abcdef".to_vec()));
        assert_eq!(headers["content-type"], "audio/mpeg");
        assert_eq!(headers["content-length"], "16");
        assert_eq!(headers["accept-ranges"], "bytes");
        assert!(headers["contentfeatures.dlna.org"].contains("DLNA.ORG_OP=01"));
        assert!(fixture.library.try_lock().is_ok());
        let (status, headers, body) = fixture.request("HEAD", &path, "").await;
        assert_eq!(
            (status, body.len(), headers["content-length"].as_str()),
            (200, 0, "16")
        );
        for (value, expected, expected_range) in [
            ("bytes=2-5", "2345", "bytes 2-5/16"),
            ("bytes=12-", "cdef", "bytes 12-15/16"),
            ("bytes=-3", "def", "bytes 13-15/16"),
            ("bytes=14-99", "ef", "bytes 14-15/16"),
        ] {
            let (status, headers, body) = fixture
                .request("GET", &path, &format!("Range: {value}\r\n"))
                .await;
            assert_eq!(status, 206);
            assert_eq!(body, expected.as_bytes());
            assert_eq!(headers["content-range"], expected_range);
            assert_eq!(headers["content-length"], expected.len().to_string());
        }
        let (status, headers, body) = fixture.request("HEAD", &path, "Range: bytes=2-5\r\n").await;
        assert_eq!(
            (status, body.len(), headers["content-length"].as_str()),
            (206, 0, "4")
        );
    }

    #[tokio::test]
    async fn invalid_ranges_and_unlisted_paths_are_rejected() {
        let fixture = fixture(b"0123456789abcdef").await;
        for range in ["bytes=16-", "bytes=8-2", "bytes=-0", "bytes=0-1,4-5"] {
            let (status, headers, body) = fixture
                .request("GET", &fixture.path(), &format!("Range: {range}\r\n"))
                .await;
            assert_eq!(status, 416);
            assert_eq!(headers["content-range"], "bytes */16");
            assert!(body.is_empty());
        }
        for path in [
            "/api/config".to_owned(),
            "/library.json".to_owned(),
            "/".to_owned(),
            format!("/media/wrong/{}.mp3", fixture.track.id),
            fixture.path().replace(".mp3", ".flac"),
            format!("/media/{}/%2e%2e%2flibrary.json", fixture.server.token),
        ] {
            assert_eq!(fixture.request("GET", &path, "").await.0, 404, "{path}");
        }
        assert_eq!(
            fixture
                .request("POST", &fixture.path(), "Content-Length: 0\r\n")
                .await
                .0,
            405
        );
    }

    #[tokio::test]
    async fn replaced_source_is_not_served_and_shutdown_closes_listener() {
        let fixture = fixture(b"original").await;
        std::fs::rename(&fixture.source, fixture.source.with_extension("original")).unwrap();
        std::fs::write(&fixture.source, b"private replacement").unwrap();
        assert_eq!(fixture.request("GET", &fixture.path(), "").await.0, 404);
        fixture.shutdown.cancel();
        // Windows can take several seconds to report a refused loopback
        // connection. Keep checking the real socket, with room for that OS
        // delay; a listener that stays open must still fail the test.
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                if tokio::net::TcpStream::connect((
                    std::net::Ipv4Addr::LOCALHOST,
                    fixture.server.port,
                ))
                .await
                .is_err()
                {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
    }
}
