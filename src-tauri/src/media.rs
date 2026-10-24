//! LAN media endpoint. Control commands are available only through Tauri IPC.

use crate::library::{mime, Library, Track};
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, Method, Response, StatusCode};
use axum::routing::get;
use axum::Router;
use futures_util::StreamExt;
use std::io::SeekFrom;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio_util::io::ReaderStream;
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
struct MediaState {
    library: Arc<Mutex<Library>>,
    token: String,
    shutdown: CancellationToken,
}

#[derive(Clone, Debug)]
pub struct MediaServer {
    pub port: u16,
    pub token: String,
}

impl MediaServer {
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
    let state = MediaState {
        library,
        token: token.clone(),
        shutdown: shutdown.clone(),
    };
    let router = Router::new()
        .route(
            "/media/{token}/{filename}",
            get(serve_media).head(serve_media),
        )
        .with_state(state);
    tokio::spawn(async move {
        // Active body streams observe the same token, releasing open files
        // before graceful shutdown waits for their connections to finish.
        let _ = axum::serve(listener, router)
            .with_graceful_shutdown(shutdown.cancelled_owned())
            .await;
    });
    Ok(MediaServer { port, token })
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
