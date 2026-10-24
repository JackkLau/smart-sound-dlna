//! Opt-in live verification against an explicitly supplied public video.
//! Uses an isolated library and never reads or imports a browser/account session.
use std::{path::PathBuf, sync::Arc, time::Duration};
use xiaomi_dlna_desktop::engine::Engine;

#[tokio::main]
async fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let video = args
        .next()
        .ok_or("请提供有权下载的公开视频链接或 BV 号。")?;
    let directory = PathBuf::from(args.next().ok_or("请指定隔离验证目录。")?);
    let engine = Engine::new(directory.clone()).await?;
    let result = async {
        let downloader = &engine.downloads;
        if downloader.state().await.authenticated {
            return Err("验收必须使用未登录会话。".into());
        }
        let preview = downloader.parse(&video).await?;
        println!(
            "Parsed anonymously: {} / {} / {} parts",
            preview.bvid,
            preview.title,
            preview.parts.len()
        );
        let part = preview
            .parts
            .iter()
            .find(|p| p.page == preview.selected_page)
            .ok_or("分 P 不存在")?;
        let job = downloader
            .start(
                Arc::downgrade(&engine),
                &preview.id,
                part.cid,
                engine.cancellation(),
            )
            .await?;
        let completed = tokio::time::timeout(Duration::from_secs(180), async {
            loop {
                let job = downloader
                    .state()
                    .await
                    .jobs
                    .into_iter()
                    .find(|j| j.id == job.id)
                    .ok_or("下载任务不存在")?;
                match job.phase.as_str() {
                    "completed" => return Ok(job),
                    "failed" | "cancelled" => return Err(job.error.unwrap_or("下载未完成".into())),
                    _ => tokio::time::sleep(Duration::from_millis(250)).await,
                }
            }
        })
        .await
        .map_err(|_| "验收下载超时")??;
        if downloader.state().await.authenticated {
            return Err("匿名验收不应变成登录状态。".into());
        }
        let tracks = engine.snapshot().await?.tracks;
        let track = tracks
            .iter()
            .find(|t| Some(&t.id) == completed.track_id.as_ref())
            .ok_or("音轨未入库")?;
        let restored = xiaomi_dlna_desktop::library::Library::load(directory)?;
        if restored.get(&track.id)?.size != track.size {
            return Err("持久化校验失败".into());
        }
        println!(
            "Downloaded and reopened: {} bytes, {}, authenticated=false",
            track.size, track.format
        );
        Ok(())
    }
    .await;
    engine.shutdown();
    result
}
