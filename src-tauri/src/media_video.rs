//! Per-session transfer evidence and bounded file readers. No source identifiers are retained here.
use axum::body::Bytes;
use futures_util::Stream;
use serde::Serialize;
use std::{
    io,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
};
use tokio::{io::AsyncReadExt, sync::mpsc};
use tokio_util::sync::CancellationToken;

const MAX_READERS: u32 = 8;
const CHUNK_SIZE: usize = 64 * 1024;
const QUEUED_CHUNKS: usize = 2;

#[derive(Clone, Debug, Serialize)]
pub struct VideoRequestSnapshot {
    pub method: String,
    pub status: u16,
    pub range_start: Option<String>,
    pub range_end: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct VideoTransferSnapshot {
    pub get_requests: u64,
    pub head_requests: u64,
    pub range_requests: u64,
    pub rejected_requests: u64,
    pub active_streams: u32,
    pub peak_active_streams: u32,
    /// Bytes yielded to the HTTP body, not acknowledged by the peer or decoded frames.
    pub bytes_streamed: String,
    pub last_request: Option<VideoRequestSnapshot>,
    pub revoked: bool,
}

#[derive(Debug, Default)]
struct Counters {
    get_requests: u64,
    head_requests: u64,
    range_requests: u64,
    rejected_requests: u64,
    active_streams: u32,
    peak_active_streams: u32,
    bytes_streamed: u64,
    last_request: Option<VideoRequestSnapshot>,
}

#[derive(Clone, Debug)]
pub struct VideoTransferMonitor {
    counters: Arc<Mutex<Counters>>,
    cancellation: CancellationToken,
}

impl VideoTransferMonitor {
    pub(super) fn new(cancellation: CancellationToken) -> Self {
        Self {
            counters: Arc::new(Mutex::new(Counters::default())),
            cancellation,
        }
    }

    pub fn snapshot(&self) -> VideoTransferSnapshot {
        let counters = self
            .counters
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        VideoTransferSnapshot {
            get_requests: counters.get_requests,
            head_requests: counters.head_requests,
            range_requests: counters.range_requests,
            rejected_requests: counters.rejected_requests,
            active_streams: counters.active_streams,
            peak_active_streams: counters.peak_active_streams,
            bytes_streamed: counters.bytes_streamed.to_string(),
            last_request: counters.last_request.clone(),
            revoked: self.cancellation.is_cancelled(),
        }
    }

    pub(super) fn has_bytes(&self) -> bool {
        self.counters
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .bytes_streamed
            > 0
    }

    pub(super) fn requested(&self, method: &str, range: bool) {
        let mut counters = self
            .counters
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match method {
            "GET" => counters.get_requests = counters.get_requests.saturating_add(1),
            "HEAD" => counters.head_requests = counters.head_requests.saturating_add(1),
            _ => {}
        }
        if range {
            counters.range_requests = counters.range_requests.saturating_add(1);
        }
    }

    pub(super) fn response(&self, method: &str, status: u16, range: Option<(u64, u64)>) {
        let mut counters = self
            .counters
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if status >= 400 {
            counters.rejected_requests = counters.rejected_requests.saturating_add(1);
        }
        counters.last_request = Some(VideoRequestSnapshot {
            method: method.to_owned(),
            status,
            range_start: range.map(|(start, _)| start.to_string()),
            range_end: range.map(|(_, end)| end.to_string()),
        });
    }

    pub(super) fn try_read(&self) -> Option<VideoReadPermit> {
        let mut counters = self
            .counters
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if self.cancellation.is_cancelled() || counters.active_streams >= MAX_READERS {
            return None;
        }
        counters.active_streams += 1;
        counters.peak_active_streams = counters.peak_active_streams.max(counters.active_streams);
        Some(VideoReadPermit {
            transfer: self.clone(),
        })
    }

    fn provided(&self, bytes: usize) {
        let mut counters = self
            .counters
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        counters.bytes_streamed = counters.bytes_streamed.saturating_add(bytes as u64);
    }
}

pub(super) struct VideoReadPermit {
    transfer: VideoTransferMonitor,
}

impl Drop for VideoReadPermit {
    fn drop(&mut self) {
        let mut counters = self
            .transfer
            .counters
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        counters.active_streams -= 1;
    }
}

type Lease = Arc<Mutex<Option<VideoReadPermit>>>;

fn release(lease: &Lease) {
    lease
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take();
}

/// The producer owns the file and observes cancellation independently of HTTP backpressure.
/// Two queued 64 KiB chunks plus one in-progress chunk bound application read-ahead per reader.
/// The permit normally stays with the HTTP body until EOF/drop; cancellation releases it only
/// after the file reader has stopped, even if the client never drains its socket.
pub(super) fn stream_file(
    mut file: tokio::fs::File,
    length: u64,
    transfer: VideoTransferMonitor,
    permit: VideoReadPermit,
    cancellation: CancellationToken,
) -> VideoBodyStream {
    let cancellation = cancellation.child_token();
    let lease = Arc::new(Mutex::new(Some(permit)));
    let (sender, receiver) = mpsc::channel(QUEUED_CHUNKS);
    let producer_cancel = cancellation.clone();
    let producer_lease = lease.clone();
    tokio::spawn(async move {
        let mut remaining = length;
        let completed = async {
            while remaining > 0 {
                // Reserve capacity before allocating/reading another chunk.
                let slot = sender.reserve().await.map_err(|_| ())?;
                let mut chunk = vec![0u8; remaining.min(CHUNK_SIZE as u64) as usize];
                match file.read(&mut chunk).await {
                    Ok(0) => {
                        slot.send(Err(io::Error::new(
                            io::ErrorKind::UnexpectedEof,
                            "video source ended before its declared length",
                        )));
                        return Ok::<(), ()>(());
                    }
                    Ok(size) => {
                        remaining -= size as u64;
                        chunk.truncate(size);
                        slot.send(Ok(Bytes::from(chunk)));
                    }
                    Err(error) => {
                        slot.send(Err(error));
                        return Ok(());
                    }
                }
            }
            Ok(())
        };
        let aborted = tokio::select! {
            biased;
            _ = producer_cancel.cancelled() => true,
            _ = sender.closed() => true,
            result = completed => result.is_err(),
        };
        drop(file);
        drop(sender);
        if !aborted {
            // The file may finish before a backpressured HTTP body. Keep watching
            // cancellation without keeping the file open, so revocation also frees
            // a completed-but-not-yet-drained response's slot.
            producer_cancel.cancelled().await;
        }
        release(&producer_lease);
    });
    VideoBodyStream {
        receiver,
        transfer,
        lease,
        cancellation,
    }
}

pub(super) struct VideoBodyStream {
    receiver: mpsc::Receiver<Result<Bytes, io::Error>>,
    transfer: VideoTransferMonitor,
    lease: Lease,
    cancellation: CancellationToken,
}

impl Stream for VideoBodyStream {
    type Item = Result<Bytes, io::Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.cancellation.is_cancelled() {
            self.receiver.close();
            while self.receiver.try_recv().is_ok() {}
            // The producer drops the file and releases the permit on its cancellation branch.
            return Poll::Ready(None);
        }
        match self.receiver.poll_recv(cx) {
            Poll::Ready(Some(Ok(chunk))) => {
                self.transfer.provided(chunk.len());
                Poll::Ready(Some(Ok(chunk)))
            }
            Poll::Ready(None) => {
                release(&self.lease);
                self.cancellation.cancel();
                Poll::Ready(None)
            }
            other => other,
        }
    }
}

impl Drop for VideoBodyStream {
    fn drop(&mut self) {
        self.cancellation.cancel();
        // An active producer retains the shared lease until its file is dropped.
        // If it has already finished, dropping this final Arc releases the lease.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::StreamExt;

    #[tokio::test]
    async fn queued_read_ahead_is_not_counted_and_cancellation_drops_undelivered_chunks() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("queued.mkv");
        std::fs::write(&path, vec![7; CHUNK_SIZE * 8]).unwrap();
        let cancellation = CancellationToken::new();
        let transfer = VideoTransferMonitor::new(cancellation.clone());
        let file = tokio::fs::File::open(&path).await.unwrap();
        let permit = transfer.try_read().unwrap();
        let mut stream = stream_file(
            file,
            (CHUNK_SIZE * 8) as u64,
            transfer.clone(),
            permit,
            cancellation.clone(),
        );
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while stream.receiver.len() < QUEUED_CHUNKS {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(transfer.snapshot().bytes_streamed, "0");
        assert!(!transfer.has_bytes());
        assert_eq!(stream.receiver.max_capacity(), QUEUED_CHUNKS);
        cancellation.cancel();
        assert!(
            stream.next().await.is_none(),
            "queued data must not be yielded after cancellation"
        );
        assert_eq!(transfer.snapshot().bytes_streamed, "0");
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while transfer.snapshot().active_streams != 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }
}

#[cfg(test)]
mod completed_reader_tests {
    use super::*;
    use futures_util::StreamExt;

    #[tokio::test]
    async fn cancellation_releases_a_completed_reader_before_body_is_drained() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("finished.mkv");
        std::fs::write(&path, vec![9; CHUNK_SIZE * 2]).unwrap();
        let cancellation = CancellationToken::new();
        let transfer = VideoTransferMonitor::new(cancellation.clone());
        let permit = transfer.try_read().unwrap();
        let file = tokio::fs::File::open(path).await.unwrap();
        let mut stream = stream_file(
            file,
            (CHUNK_SIZE * 2) as u64,
            transfer.clone(),
            permit,
            cancellation.clone(),
        );
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while !stream.receiver.is_closed() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(stream.receiver.len(), 2);
        assert_eq!(transfer.snapshot().active_streams, 1);
        assert_eq!(transfer.snapshot().bytes_streamed, "0");
        cancellation.cancel();
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while transfer.snapshot().active_streams != 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(stream.next().await.is_none());
        assert_eq!(transfer.snapshot().bytes_streamed, "0");
    }
}
