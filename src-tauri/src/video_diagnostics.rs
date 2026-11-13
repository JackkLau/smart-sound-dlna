//! Bounded session evidence. No media URLs, file paths or device identifiers enter exports.
use crate::media::{VideoTransferMonitor, VideoTransferSnapshot};
use serde::Serialize;
use std::time::{Duration, Instant};

#[derive(Clone, Debug, Serialize)]
pub struct CommandEvidence {
    pub action: String,
    pub outcome: String,
    pub elapsed_ms: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct VideoDiagnostics {
    pub schema_version: u8,
    pub elapsed_ms: u64,
    pub last_command: Option<CommandEvidence>,
    pub last_poll_age_ms: Option<u64>,
    pub consecutive_poll_failures: u32,
    pub state_source: String,
    pub transfer: VideoTransferSnapshot,
}

pub(crate) struct SessionTrace {
    started: Instant,
    command_started: Option<Instant>,
    command: Option<CommandEvidence>,
    last_poll: Option<Instant>,
    failures: u32,
    source: &'static str,
    pub next_poll: Option<Instant>,
}

fn millis(duration: Duration) -> u64 {
    duration.as_millis().min(u64::MAX as u128) as u64
}

impl SessionTrace {
    pub fn new() -> Self {
        Self {
            started: Instant::now(),
            command_started: None,
            command: None,
            last_poll: None,
            failures: 0,
            source: "unknown",
            next_poll: None,
        }
    }

    pub fn command(&mut self, action: &str, outcome: &str) {
        if outcome == "pending" {
            self.command_started = Some(Instant::now());
        }
        self.command = Some(CommandEvidence {
            action: action.into(),
            outcome: outcome.into(),
            elapsed_ms: self
                .command_started
                .map(|start| millis(start.elapsed()))
                .unwrap_or(0),
        });
        self.source = if outcome == "failed" {
            "unknown"
        } else {
            "command"
        };
        // An explicit control is followed by a fresh read even after disconnection backoff.
        self.next_poll = None;
    }

    pub fn poll(&mut self, success: bool) {
        let now = Instant::now();
        self.last_poll = Some(now);
        if success {
            self.failures = 0;
            self.next_poll = None;
            self.source = "poll";
        } else {
            self.failures = self.failures.saturating_add(1);
            let delay = (2u64 << (self.failures - 1).min(4)).min(30);
            self.next_poll = Some(now + Duration::from_secs(delay));
            self.source = "unknown";
        }
    }

    pub fn snapshot(&self, transfer: &VideoTransferMonitor) -> VideoDiagnostics {
        VideoDiagnostics {
            schema_version: 1,
            elapsed_ms: millis(self.started.elapsed()),
            last_command: self.command.clone(),
            last_poll_age_ms: self.last_poll.map(|last| millis(last.elapsed())),
            consecutive_poll_failures: self.failures,
            state_source: self.source.into(),
            transfer: transfer.snapshot(),
        }
    }
}

/// The destination comes only from the native save dialog. Publish atomically.
#[cfg(feature = "desktop")]
pub(crate) fn write_report(path: &std::path::Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    if !path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
    {
        return Err("请将诊断保存为 .json 文件。".into());
    }
    let parent = path.parent().ok_or("诊断保存位置无效。")?;
    let mut file = tempfile::NamedTempFile::new_in(parent)
        .map_err(|_| "无法创建诊断文件，请选择可写位置。")?;
    file.write_all(bytes)
        .and_then(|_| file.as_file().sync_all())
        .map_err(|_| "诊断文件写入失败，请检查剩余空间。")?;
    file.persist(path)
        .map_err(|_| "诊断文件保存失败，请检查文件是否被占用。")?;
    Ok(())
}

#[cfg(all(test, feature = "desktop"))]
mod tests {
    use super::*;
    #[test]
    fn diagnostic_save_is_atomic_and_refuses_media_extensions() {
        let dir = tempfile::tempdir().unwrap();
        let media = dir.path().join("movie.mkv");
        std::fs::write(&media, b"original movie").unwrap();
        assert!(write_report(&media, b"{}").is_err());
        assert_eq!(std::fs::read(&media).unwrap(), b"original movie");
        let report = dir.path().join("report.json");
        write_report(&report, b"{\"schema_version\":1}").unwrap();
        assert_eq!(std::fs::read(&report).unwrap(), b"{\"schema_version\":1}");
        write_report(&report, b"{\"schema_version\":2}").unwrap();
        assert_eq!(std::fs::read(&report).unwrap(), b"{\"schema_version\":2}");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
    }
}
