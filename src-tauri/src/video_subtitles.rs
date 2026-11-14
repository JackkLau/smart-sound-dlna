//! Create a session-scoped Matroska with one chosen subtitle or a captioned picture.
use crate::video::VideoSource;
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        atomic::{AtomicU8, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubtitleMode {
    Remux,
    Burn,
    BurnHdr10,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SubtitleSelection {
    pub index: u32,
    pub mode: SubtitleMode,
}

pub(crate) fn codec(source: &VideoSource, choice: SubtitleSelection) -> Result<&str, String> {
    let track = source
        .info
        .subtitle_tracks
        .iter()
        .find(|t| t.index == choice.index)
        .ok_or("请选择当前影片的字幕轨。")?;
    if choice.mode == SubtitleMode::BurnHdr10
        && (!source.info.can_burn_hdr10 || track.codec.as_deref() != Some("hdmv_pgs_subtitle"))
    {
        return Err("原分辨率 HDR10 模式需要支持的 HDR10 片源和 PGS 字幕。".into());
    }
    match track.codec.as_deref() {
        Some(
            c @ ("hdmv_pgs_subtitle" | "dvd_subtitle" | "dvb_subtitle" | "subrip" | "ass" | "ssa"
            | "mov_text" | "webvtt" | "text"),
        ) => Ok(c),
        _ => Err("此字幕编码暂不支持处理，可选择其他字幕轨或使用原文件。".into()),
    }
}

fn input(tool: &Path, source: &VideoSource, directory: &Path) -> Command {
    let mut command = Command::new(tool);
    command
        .current_dir(directory)
        .kill_on_drop(true)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .args([
            "-nostdin",
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostats",
            "-progress",
            "pipe:1",
        ]);
    add_media_input(&mut command, source);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    command
}

fn add_media_input(command: &mut Command, source: &VideoSource) {
    command.args([
        "-protocol_whitelist",
        "file",
        "-format_whitelist",
        "mov,matroska,mpegts",
    ]);
    if source.info.format == "mp4" {
        command.args(["-enable_drefs", "0", "-use_absolute_path", "0"]);
    }
    command.arg("-i").arg(source.stamp().path);
}

async fn render(
    mut command: Command,
    cancel: &CancellationToken,
    progress: Option<(&AtomicU8, u64)>,
) -> Result<(), String> {
    if cancel.is_cancelled() {
        return Err("字幕处理已取消。".into());
    }
    let directory = command
        .as_std()
        .get_current_dir()
        .ok_or("缺少字幕工作目录。")?
        .to_owned();
    crate::subtitle_cache::check_space(&directory)?;
    let mut child = command.spawn().map_err(|_| "无法启动包内 FFmpeg。")?;
    let mut lines = BufReader::new(child.stdout.take().ok_or("无法读取字幕处理进度。")?).lines();
    let monitor = async {
        while let Some(line) = lines
            .next_line()
            .await
            .map_err(|_| "字幕处理进度读取失败。")?
        {
            if let (Some((percent, total_ms)), Some(us)) = (
                progress,
                line.strip_prefix("out_time_us=")
                    .and_then(|v| v.parse::<u64>().ok()),
            ) {
                percent.store((us / total_ms.max(1) / 10).min(99) as u8, Ordering::Relaxed);
            }
        }
        match child.wait().await {
            Ok(status) if status.success() => Ok(()),
            _ => Err("字幕版本生成失败，请检查临时磁盘空间或选择其他字幕轨。".to_string()),
        }
    };
    let disk_watch = async {
        loop {
            tokio::time::sleep(Duration::from_secs(2)).await;
            crate::subtitle_cache::check_space(&directory)?;
        }
    };
    let result = tokio::select! {
        biased;
        _ = cancel.cancelled() => Err("字幕处理已取消。".to_string()),
        result = disk_watch => result,
        result = tokio::time::timeout(Duration::from_secs(24 * 3600), monitor) =>
            result.unwrap_or_else(|_| Err("字幕处理超时，请重新准备。".into())),
    };
    if result.is_err() {
        let _ = child.start_kill();
        let _ = child.wait().await;
    }
    result
}

pub(crate) async fn prepare(
    source: &VideoSource,
    choice: SubtitleSelection,
    tools: (PathBuf, PathBuf),
    cancel: &CancellationToken,
    percent: &AtomicU8,
    root: &Path,
) -> Result<VideoSource, String> {
    let (tool, probe) = tools;
    let codec = codec(source, choice)?;
    if !tool.is_absolute() || !tool.is_file() {
        return Err("缺少字幕处理工具，请使用包含影视工具的安装包。".into());
    }
    source.open()?;
    let work = Arc::new(crate::subtitle_cache::Workspace::new(root)?);
    let directory = work.path();
    let track = format!("0:{}", choice.index);
    let mut command = input(&tool, source, directory);
    match choice.mode {
        SubtitleMode::Remux => {
            let target_codec = match codec {
                "mov_text" | "text" => "srt",
                _ => "copy",
            };
            command.args([
                "-map",
                "0:V:0",
                "-map",
                "0:a?",
                "-map",
                &track,
                "-map",
                "0:t?",
                "-c",
                "copy",
                "-c:s",
                target_codec,
                "-disposition:s:0",
                "default+forced",
            ]);
        }
        SubtitleMode::BurnHdr10 => {
            let width = source.info.width.ok_or("缺少原片宽度。")?;
            let height = source.info.height.ok_or("缺少原片高度。")?;
            add_media_input(&mut command, source);
            // Convert only the subtitle plane from sRGB to PQ at 203-nit reference white.
            // Keep its alpha and aspect ratio, and keep the video as the primary input
            // to overlay so its original HDR10 frame metadata reaches the encoder.
            // Register the main input first; FFmpeg 7.1 can otherwise stop the
            // graph at the last sparse subtitle, dropping the video tail.
            let graph = format!(
                "[0:V:0]null[base];[1:{}]format=gbrap,zscale=pin=bt709:tin=iec61966-2-1:min=gbr:rin=full:\
                 p=bt2020:t=smpte2084:m=bt2020nc:r=limited:npl=203,format=yuva420p10le,\
                 scale={width}:{height}:force_original_aspect_ratio=decrease:force_divisible_by=2[sub];\
                 [base][sub]overlay=x=(W-w)/2:y=(H-h)/2:format=yuv420p10:eof_action=pass[out]",
                choice.index
            );
            command.args([
                "-filter_complex",
                &graph,
                "-map",
                "[out]",
                "-map",
                "0:a?",
                "-c:a",
                "copy",
                "-c:v",
                "libx265",
                "-preset",
                "veryfast",
                "-crf",
                "18",
                "-pix_fmt",
                "yuv420p10le",
                "-x265-params",
                "pools=4:frame-threads=3:log-level=error",
                "-colorspace",
                "bt2020nc",
                "-color_trc",
                "smpte2084",
                "-color_primaries",
                "bt2020",
                "-color_range",
                "tv",
            ]);
        }
        SubtitleMode::Burn => {
            let bitmap = matches!(codec, "hdmv_pgs_subtitle" | "dvd_subtitle" | "dvb_subtitle");
            // Fixed intermediate name keeps arbitrary user paths out of libavfilter syntax.
            if !bitmap {
                let mut extract = input(&tool, source, directory);
                extract.args(["-map", &track, "-c:s", "ass", "-n", "captions.ass"]);
                render(extract, cancel, None).await?;
            }
            let mut graph = String::from(
                "[0:V:0]scale=w='max(2,trunc(min(1920,1080*dar)/2)*2)':h='max(2,trunc(min(1080,1920/dar)/2)*2)'",
            );
            if matches!(
                source.info.color_transfer.as_deref(),
                Some("smpte2084" | "arib-std-b67")
            ) {
                graph.push_str(",zscale=t=linear:npl=100,format=gbrpf32le,tonemap=hable,zscale=p=bt709:t=bt709:m=bt709:r=limited");
            }
            graph.push_str(",format=yuv420p,pad=1920:1080:(ow-iw)/2:(oh-ih)/2,setsar=1");
            if bitmap {
                // Read subtitle packets independently so sparse cues do not arrive after video frames.
                add_media_input(&mut command, source);
                let subtitle = format!("1:{}", choice.index);
                graph.push_str(&format!("[base];[{subtitle}]scale=1920:1080,format=rgba[sub];[base][sub]overlay=eof_action=pass[out]"));
            } else {
                graph.push_str(",ass=captions.ass[out]");
            }
            command.args([
                "-filter_complex",
                &graph,
                "-map",
                "[out]",
                "-map",
                "0:a?",
                "-c:v",
                "libx264",
                "-threads:v",
                "4",
                "-preset",
                "veryfast",
                "-crf",
                "18",
                "-pix_fmt",
                "yuv420p",
                "-c:a",
                "copy",
            ]);
            if matches!(
                source.info.color_transfer.as_deref(),
                Some("smpte2084" | "arib-std-b67")
            ) {
                command.args([
                    "-colorspace",
                    "bt709",
                    "-color_trc",
                    "bt709",
                    "-color_primaries",
                    "bt709",
                ]);
            }
        }
    }
    command.args(["-f", "matroska", "-n", "movie.mkv"]);
    render(
        command,
        cancel,
        source.info.duration_ms.map(|ms| (percent, ms)),
    )
    .await?;
    source.open()?;
    let mut output =
        crate::video::probe_file(directory.join("movie.mkv"), probe, cancel.clone()).await?;
    let expected_subtitles = usize::from(choice.mode == SubtitleMode::Remux);
    if output.info.subtitle_tracks.len() != expected_subtitles
        || output.info.audio_tracks.len() != source.info.audio_tracks.len()
    {
        return Err("生成版本的字幕或音轨数量不符，请重新准备。".into());
    }
    if choice.mode == SubtitleMode::BurnHdr10
        && (!output.info.can_burn_hdr10
            || output.info.video_codec.as_deref() != Some("hevc")
            || (output.info.width, output.info.height) != (source.info.width, source.info.height))
    {
        return Err("生成版本未通过原分辨率与 HDR10 格式校验，请重新准备。".into());
    }
    output.info.id.clone_from(&source.info.id);
    output.info.title.clone_from(&source.info.title);

    output.temporary = Some(work);
    Ok(output)
}
