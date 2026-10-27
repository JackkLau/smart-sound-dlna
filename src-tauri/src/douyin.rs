//! Public Douyin video metadata. Only official share pages and media CDNs are
//! accepted; account cookies, challenge solvers and third-party parsers are not used.
use crate::online;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    io::{Read, Seek, SeekFrom, Write},
    time::Duration,
};
use url::Url;

pub const UA: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 Version/17.0 Mobile/15E148 Safari/604.1";
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Video {
    pub id: String,
    pub title: String,
    pub author: String,
    pub duration: u64,
    pub urls: Vec<String>,
}
fn valid_id(id: &str) -> bool {
    (15..=22).contains(&id.len()) && id.bytes().all(|c| c.is_ascii_digit())
}
pub fn page_url(raw: &str) -> Result<Url, String> {
    let url = Url::parse(raw).map_err(|_| "请粘贴完整的抖音视频链接。")?;
    if url.scheme() != "https"
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || !matches!(
            url.host_str(),
            Some(
                "www.douyin.com"
                    | "douyin.com"
                    | "v.douyin.com"
                    | "www.iesdouyin.com"
                    | "iesdouyin.com"
            )
        )
    {
        return Err("仅支持抖音官方 HTTPS 视频链接。".into());
    }
    Ok(url)
}
fn video_id(url: &Url) -> Option<String> {
    let parts: Vec<_> = url.path_segments()?.filter(|s| !s.is_empty()).collect();
    let id = match parts.as_slice() {
        ["video", id] | ["share", "video", id] => *id,
        _ => return None,
    };
    valid_id(id).then(|| id.to_owned())
}
pub fn shared_input(text: &str) -> Result<Url, String> {
    if text.len() > 8192 {
        return Err("分享内容过长，请只粘贴一个视频链接。".into());
    }
    let mut found = Vec::new();
    for word in
        text.split(|c: char| c.is_whitespace() || "【】「」《》<>\"，。；（）()".contains(c))
    {
        let word = word.trim_matches(|c| matches!(c, ',' | '.' | ';' | '!' | '！'));
        if let Ok(mut url) = page_url(word) {
            url.set_fragment(None);
            if !found.contains(&url) {
                found.push(url);
            }
        }
    }
    if found.len() != 1 {
        return Err("请粘贴一个抖音视频链接或单条视频的分享内容。".into());
    }
    Ok(found.remove(0))
}
pub async fn resolve_input(input: &str) -> Result<String, String> {
    let mut url = shared_input(input)?;
    let client = online::client(UA)?;
    for _ in 0..5 {
        if let Some(id) = video_id(&url) {
            return Ok(id);
        }
        if url.host_str() != Some("v.douyin.com") {
            return Err("暂不支持抖音图集、直播或主页，请复制单个视频的链接。".into());
        }
        let response = client
            .get(url.clone())
            .timeout(Duration::from_secs(15))
            .send()
            .await
            .map_err(|_| "抖音短链接无法打开，请检查网络。")?;
        if !response.status().is_redirection() {
            return Err("短链接未指向视频，请复制完整的视频地址。".into());
        }
        let next = response
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .ok_or("抖音短链接缺少跳转地址。")?;
        url = page_url(url.join(next).map_err(|_| "抖音跳转无效")?.as_str())?;
    }
    Err("抖音跳转过多，请使用完整视频链接。".into())
}
pub fn media_url(raw: &str) -> Result<Url, String> {
    let raw = if raw.starts_with("//") {
        format!("https:{raw}")
    } else {
        raw.to_owned()
    };
    let mut url = Url::parse(&raw).map_err(|_| "抖音媒体地址无效")?;
    // Some official response variants still advertise HTTP; never send plaintext.
    if url.scheme() == "http" {
        let _ = url.set_scheme("https");
    }
    let host = url.host_str().ok_or("抖音媒体地址缺少域名")?;
    if url.scheme() != "https"
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || ![
            "douyinvod.com",
            "douyin.com",
            "iesdouyin.com",
            "amemv.com",
            "bytecdn.cn",
        ]
        .iter()
        .any(|root| host == *root || host.ends_with(&format!(".{root}")))
    {
        return Err("抖音返回了不支持的媒体地址。".into());
    }
    Ok(url)
}
impl Video {
    pub fn validate(mut self, expected: &str) -> Result<Self, String> {
        if !valid_id(&self.id) || self.id != expected {
            return Err("视频信息与链接不一致，请重新解析。".into());
        }
        self.title = self
            .title
            .trim()
            .chars()
            .filter(|c| !c.is_control())
            .take(300)
            .collect();
        self.author = self
            .author
            .chars()
            .filter(|c| !c.is_control())
            .take(100)
            .collect();
        if self.title.is_empty() {
            self.title = format!("抖音视频 {}", self.id);
        }
        self.duration = self.duration.min(86400);
        self.urls = self
            .urls
            .iter()
            .take(8)
            .filter(|s| s.len() <= 8192)
            .filter_map(|s| media_url(s).ok().map(Into::into))
            .collect();
        if self.urls.is_empty() {
            return Err("这个视频未提供可下载的公开音视频，暂时无法导入。".into());
        }
        Ok(self)
    }
}
pub fn from_item(item: &Value, expected: &str) -> Result<Video, String> {
    // Images/slideshows may have background music; that music is not this video's audio.
    if item["images"].as_array().is_some_and(|a| !a.is_empty()) {
        return Err("暂不支持抖音图集，请选择视频。".into());
    }
    let urls = item["video"]["play_addr"]["url_list"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect();
    Video {
        id: item["aweme_id"].as_str().unwrap_or_default().into(),
        title: item["desc"].as_str().unwrap_or_default().into(),
        author: item["author"]["nickname"]
            .as_str()
            .unwrap_or("抖音作者")
            .into(),
        duration: item["video"]["duration"].as_u64().unwrap_or(0) / 1000,
        urls,
    }
    .validate(expected)
}
pub fn parse_page(html: &str, id: &str) -> Result<Video, String> {
    if let Some((_, rest)) = html.split_once("window._ROUTER_DATA") {
        if let Some((_, json)) = rest.split_once('=') {
            let mut stream =
                serde_json::Deserializer::from_str(json.trim_start()).into_iter::<Value>();
            if let Some(Ok(data)) = stream.next() {
                if let Some(loaders) = data["loaderData"].as_object() {
                    for value in loaders.values() {
                        if let Some(items) = value["videoInfoRes"]["item_list"].as_array() {
                            if let Some(item) =
                                items.iter().find(|v| v["aweme_id"].as_str() == Some(id))
                            {
                                return from_item(item, id);
                            }
                        }
                    }
                }
            }
        }
    }
    Err("抖音需要加载官方网页后才能读取此视频，请在解析窗口中等待。".into())
}
pub async fn fetch(id: &str) -> Result<Video, String> {
    if !valid_id(id) {
        return Err("抖音视频编号无效".into());
    }
    let mut response = online::client(UA)?
        .get(format!("https://www.iesdouyin.com/share/video/{id}/"))
        .timeout(Duration::from_secs(25))
        .send()
        .await
        .map_err(|_| "抖音连接失败，请检查网络。")?;
    if !response.status().is_success() {
        return Err("抖音暂时无法读取此视频，请稍后重试。".into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| "读取抖音页面失败")? {
        if bytes.len() + chunk.len() > 2 * 1024 * 1024 {
            return Err("抖音页面过大，已停止读取。".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    parse_page(&String::from_utf8_lossy(&bytes), id)
}

/// Copy AAC samples, without decoding/re-encoding or retaining video tracks.
pub fn extract_audio(
    input: &std::path::Path,
    directory: &std::path::Path,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<tempfile::NamedTempFile, String> {
    let operation = || -> Result<tempfile::NamedTempFile, String> {
        let mut file = std::fs::File::open(input).map_err(|_| "无法读取已下载视频")?;
        let size = file.metadata().map_err(|_| "无法读取视频大小")?.len();
        // Bound metadata before the third-party parser allocates sample tables.
        let mut offset = 0;
        while offset < size {
            let mut header = [0u8; 8];
            file.read_exact(&mut header).map_err(|_| "视频文件不完整")?;
            let mut len = u32::from_be_bytes(header[..4].try_into().unwrap()) as u64;
            let mut header_size = 8;
            if len == 1 {
                let mut ext = [0u8; 8];
                file.read_exact(&mut ext).map_err(|_| "视频文件不完整")?;
                len = u64::from_be_bytes(ext);
                header_size = 16;
            }
            if len == 0 {
                len = size - offset;
            }
            if len < header_size || len > size - offset {
                return Err("视频容器损坏".into());
            }
            if &header[4..] != b"mdat" && len > 16 * 1024 * 1024 {
                return Err("视频元数据过大".into());
            }
            if &header[4..] == b"moof" {
                return Err("暂不支持分片视频音轨，请换一个视频。".into());
            }
            offset += len;
            file.seek(SeekFrom::Start(offset))
                .map_err(|_| "无法读取视频")?;
        }
        file.rewind().map_err(|_| "无法读取视频")?;
        let mut reader =
            mp4::Mp4Reader::read_header(file, size).map_err(|_| "无法读取 MP4 视频音轨")?;
        let tracks: Vec<_> = reader
            .tracks()
            .values()
            .filter(|t| t.track_type().ok() == Some(mp4::TrackType::Audio))
            .collect();
        if tracks.len() != 1 {
            return Err("视频没有唯一音轨，暂时无法导入。".into());
        }
        let track = tracks[0];
        let profile = track.audio_profile().map_err(|_| "暂仅支持 AAC 音轨")?;
        let timescale = track.timescale();
        if timescale == 0 || track.sample_count() == 0 || track.sample_count() > 2_000_000 {
            return Err("视频音轨长度无效".into());
        }
        let id = track.track_id();
        let description = audio_description(input, id)?;
        let count = track.sample_count();
        // The crate exposes its parsed sample table; bound every allocation.
        let stsz = &track.trak.mdia.minf.stbl.stsz;
        if stsz.sample_size > 1024 * 1024 || stsz.sample_sizes.iter().any(|s| *s > 1024 * 1024) {
            return Err("音频采样块过大".into());
        }
        let config = mp4::TrackConfig {
            track_type: mp4::TrackType::Audio,
            timescale,
            language: "und".into(),
            media_conf: mp4::MediaConfig::AacConfig(mp4::AacConfig {
                bitrate: track.bitrate(),
                profile,
                freq_index: track
                    .sample_freq_index()
                    .unwrap_or(mp4::SampleFreqIndex::Freq48000),
                chan_conf: track.channel_config().unwrap_or(mp4::ChannelConfig::Stereo),
            }),
        };
        let temp = tempfile::Builder::new()
            .prefix(".audio-")
            .suffix(".part")
            .tempfile_in(directory)
            .map_err(|_| "无法创建音频文件")?;
        let output = temp.reopen().map_err(|_| "无法写入音频")?;
        let mut writer = mp4::Mp4Writer::write_start(
            output,
            &mp4::Mp4Config {
                major_brand: "M4A ".parse().unwrap(),
                minor_version: 0,
                compatible_brands: vec!["isom".parse().unwrap(), "mp42".parse().unwrap()],
                timescale,
            },
        )
        .map_err(|_| "无法创建 M4A 音频")?;
        writer.add_track(&config).map_err(|_| "无法创建音轨")?;
        let mut audio_bytes = 0u64;
        for index in 1..=count {
            if cancel.is_cancelled() {
                return Err("已取消".into());
            }
            let sample = reader
                .read_sample(id, index)
                .map_err(|_| "音频数据损坏")?
                .ok_or("音频数据不完整")?;
            audio_bytes += sample.bytes.len() as u64;
            if audio_bytes > 1024 * 1024 * 1024 {
                return Err("音频超过 1 GB，已停止提取".into());
            }
            writer
                .write_sample(1, &sample)
                .map_err(|_| "保存音频失败，请检查磁盘空间")?;
        }
        writer.write_end().map_err(|_| "保存音轨索引失败")?;
        drop(writer.into_writer());
        restore_description(temp.path(), &description)?;
        Ok(temp)
    };
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(operation))
        .map_err(|_| "视频容器异常，无法提取音轨。".to_owned())?
}

// mp4's writer rebuilds a minimal AudioSpecificConfig and drops HE-AAC SBR/PS
// extensions. Copy the original stsd verbatim after writing new audio-only sample
// tables. The generated moov is last, so mdat/chunk offsets remain unchanged.
fn atom_ranges(bytes: &[u8]) -> Result<Vec<(usize, usize, usize)>, String> {
    let mut ranges = Vec::new();
    let mut start = 0;
    while start < bytes.len() {
        if bytes.len() - start < 8 {
            return Err("音轨元数据不完整".into());
        }
        let mut size = u32::from_be_bytes(bytes[start..start + 4].try_into().unwrap()) as usize;
        let mut header = 8;
        if size == 1 {
            if bytes.len() - start < 16 {
                return Err("音轨元数据不完整".into());
            }
            size = usize::try_from(u64::from_be_bytes(
                bytes[start + 8..start + 16].try_into().unwrap(),
            ))
            .map_err(|_| "音轨元数据过大")?;
            header = 16;
        }
        if size < header || size > bytes.len() - start {
            return Err("音轨元数据尺寸无效".into());
        }
        ranges.push((start, start + size, header));
        start += size;
    }
    Ok(ranges)
}
fn moov_bytes(path: &std::path::Path) -> Result<(u64, Vec<u8>), String> {
    let mut file = std::fs::File::open(path).map_err(|_| "无法读取音轨")?;
    let total = file.metadata().map_err(|_| "无法读取音轨")?.len();
    let mut start = 0;
    while start + 8 <= total {
        let mut head = [0u8; 8];
        file.read_exact(&mut head).map_err(|_| "音轨文件不完整")?;
        let mut size = u32::from_be_bytes(head[..4].try_into().unwrap()) as u64;
        let mut header = 8;
        if size == 1 {
            let mut large = [0u8; 8];
            file.read_exact(&mut large).map_err(|_| "音轨文件不完整")?;
            size = u64::from_be_bytes(large);
            header = 16;
        }
        if size == 0 {
            size = total - start;
        }
        if size < header || size > total - start {
            return Err("音轨文件尺寸无效".into());
        }
        if &head[4..] == b"moov" {
            if size > 16 * 1024 * 1024 {
                return Err("音轨元数据过大".into());
            }
            let mut bytes = vec![0; (size - header) as usize];
            file.read_exact(&mut bytes).map_err(|_| "音轨索引不完整")?;
            return Ok((start, bytes));
        }
        start += size;
        file.seek(SeekFrom::Start(start))
            .map_err(|_| "无法读取音轨")?;
    }
    Err("缺少音轨索引".into())
}
fn child<'a>(bytes: &'a [u8], name: &[u8; 4]) -> Result<&'a [u8], String> {
    let ranges = atom_ranges(bytes)?;
    let (start, end, header) = ranges
        .into_iter()
        .find(|(s, _, _)| &bytes[s + 4..s + 8] == name)
        .ok_or("音轨缺少必要元数据")?;
    Ok(&bytes[start + header..end])
}
fn audio_description(path: &std::path::Path, track_id: u32) -> Result<Vec<u8>, String> {
    let (_, moov) = moov_bytes(path)?;
    for (start, end, header) in atom_ranges(&moov)? {
        if &moov[start + 4..start + 8] != b"trak" {
            continue;
        }
        let track = &moov[start + header..end];
        let tkhd = child(track, b"tkhd")?;
        let offset = match tkhd.first() {
            Some(0) => 12,
            Some(1) => 20,
            _ => return Err("不支持的音轨索引版本".into()),
        };
        let id = tkhd.get(offset..offset + 4).ok_or("音轨编号缺失")?;
        if u32::from_be_bytes(id.try_into().unwrap()) != track_id {
            continue;
        }
        let stbl = child(child(child(track, b"mdia")?, b"minf")?, b"stbl")?;
        let stsd = child(stbl, b"stsd")?;
        if stsd.len() < 16
            || u32::from_be_bytes(stsd[4..8].try_into().unwrap()) != 1
            || &stsd[12..16] != b"mp4a"
        {
            return Err("暂仅支持未加密的单一 AAC 音轨".into());
        }
        return Ok(stsd.to_vec());
    }
    Err("找不到目标音轨配置".into())
}
fn replace_description(bytes: &[u8], path: &[[u8; 4]], stsd: &[u8]) -> Result<Vec<u8>, String> {
    let mut result = Vec::new();
    let mut replaced = 0;
    for (start, end, header) in atom_ranges(bytes)? {
        if bytes[start + 4..start + 8] == path[0] {
            replaced += 1;
            let payload = if path.len() == 1 {
                stsd.to_vec()
            } else {
                replace_description(&bytes[start + header..end], &path[1..], stsd)?
            };
            let len = u32::try_from(payload.len() + 8).map_err(|_| "音轨元数据过大")?;
            result.extend_from_slice(&len.to_be_bytes());
            result.extend_from_slice(&path[0]);
            result.extend_from_slice(&payload);
        } else {
            result.extend_from_slice(&bytes[start..end]);
        }
    }
    if replaced != 1 {
        return Err("音轨配置不唯一".into());
    }
    Ok(result)
}
fn restore_description(path: &std::path::Path, stsd: &[u8]) -> Result<(), String> {
    let (offset, moov) = moov_bytes(path)?;
    let next = replace_description(
        &moov,
        &[*b"trak", *b"mdia", *b"minf", *b"stbl", *b"stsd"],
        stsd,
    )?;
    let size = u32::try_from(next.len() + 8).map_err(|_| "音轨元数据过大")?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .map_err(|_| "无法保存音轨配置")?;
    file.seek(SeekFrom::Start(offset))
        .map_err(|_| "无法保存音轨配置")?;
    file.write_all(&size.to_be_bytes())
        .and_then(|_| file.write_all(b"moov"))
        .and_then(|_| file.write_all(&next))
        .and_then(|_| file.set_len(offset + u64::from(size)))
        .and_then(|_| file.sync_all())
        .map_err(|_| "保存音轨配置失败，请检查磁盘空间".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    const ID: &str = "7518247213001592121";
    fn item() -> Value {
        json!({"aweme_id": ID, "desc": "歌手《曲名》", "author": {"nickname": "作者"}, "video": {"duration": 330800, "play_addr": {"url_list": ["https://v3-test.douyinvod.com/sample.mp4"]}}})
    }
    #[test]
    fn official_links_and_share_text_only() {
        let url = shared_input(&format!(
            "分享音乐 https://www.douyin.com/video/{ID} 复制打开抖音"
        ))
        .unwrap();
        assert_eq!(video_id(&url).as_deref(), Some(ID));
        assert!(shared_input("https://v.douyin.com/abc/").is_ok());
        for raw in [
            "https://www.douyin.com.evil.test/video/7518247213001592121",
            "https://www.douyin.com@127.0.0.1/video/7518247213001592121",
            "http://www.douyin.com/video/7518247213001592121",
            "https://www.douyin.com:444/video/7518247213001592121",
            "file:///tmp/test.mp4",
        ] {
            assert!(shared_input(raw).is_err(), "{raw}");
        }
        assert!(shared_input("https://v.douyin.com/one/ https://v.douyin.com/two/").is_err());
        assert!(
            video_id(&page_url("https://www.douyin.com/note/7518247213001592121").unwrap())
                .is_none()
        );
    }
    #[test]
    fn metadata_matches_requested_video_and_rejects_images_or_foreign_media() {
        let video = from_item(&item(), ID).unwrap();
        assert_eq!(video.duration, 330);
        assert_eq!(video.title, "歌手《曲名》");
        assert!(from_item(&item(), "7518247213001592122").is_err());
        let mut value = item();
        value["images"] = json!([{"image": "slide"}]);
        assert!(from_item(&value, ID).is_err());
        for raw in [
            "https://127.0.0.1/a.mp4",
            "https://douyinvod.com.evil.test/a.mp4",
            "https://name:pass@v3.douyinvod.com/a.mp4",
            "https://v3.douyinvod.com:444/a.mp4",
        ] {
            assert!(media_url(raw).is_err());
        }
        value = item();
        value["video"]["play_addr"]["url_list"] = json!(["http://127.0.0.1/test"]);
        assert!(from_item(&value, ID).is_err());
    }
    #[test]
    fn public_ssr_and_challenge_pages_have_distinct_outcomes() {
        let data =
            json!({"loaderData": {"video_(id)/page": {"videoInfoRes": {"item_list": [item()]}}}});
        assert_eq!(
            parse_page(
                &format!("<script>window._ROUTER_DATA = {data};</script>"),
                ID
            )
            .unwrap()
            .id,
            ID
        );
        assert!(parse_page("<html>验证码</html>", ID).is_err());
        assert!(parse_page("window._ROUTER_DATA = {broken", ID).is_err());
    }
    fn fixture(path: &std::path::Path, audio: bool) {
        let config = mp4::Mp4Config {
            major_brand: "isom".parse().unwrap(),
            minor_version: 0,
            compatible_brands: vec![],
            timescale: 48000,
        };
        let mut writer =
            mp4::Mp4Writer::write_start(std::fs::File::create(path).unwrap(), &config).unwrap();
        writer
            .add_track(&mp4::TrackConfig::from(mp4::Vp9Config {
                width: 16,
                height: 16,
            }))
            .unwrap();
        if audio {
            writer
                .add_track(&mp4::TrackConfig {
                    track_type: mp4::TrackType::Audio,
                    timescale: 48000,
                    language: "und".into(),
                    media_conf: mp4::MediaConfig::AacConfig(mp4::AacConfig::default()),
                })
                .unwrap();
        }
        for n in 0..3 {
            writer
                .write_sample(
                    1,
                    &mp4::Mp4Sample {
                        start_time: n * 1000,
                        duration: 1000,
                        rendering_offset: 0,
                        is_sync: true,
                        bytes: vec![9, 8, 7].into(),
                    },
                )
                .unwrap();
            if audio {
                writer
                    .write_sample(
                        2,
                        &mp4::Mp4Sample {
                            start_time: n * 1024,
                            duration: 1024,
                            rendering_offset: 0,
                            is_sync: true,
                            bytes: vec![1, 2, 3, n as u8].into(),
                        },
                    )
                    .unwrap();
            }
        }
        writer.write_end().unwrap();
    }
    #[test]
    fn remux_preserves_audio_samples_timing_and_removes_video() {
        let directory = tempfile::tempdir().unwrap();
        let input = directory.path().join("video.mp4");
        fixture(&input, true);
        let result = extract_audio(
            &input,
            directory.path(),
            &tokio_util::sync::CancellationToken::new(),
        )
        .unwrap();
        assert_eq!(
            audio_description(&input, 2).unwrap(),
            audio_description(result.path(), 1).unwrap()
        );
        let file = result.reopen().unwrap();
        let size = file.metadata().unwrap().len();
        let mut reader = mp4::Mp4Reader::read_header(file, size).unwrap();
        assert_eq!(reader.tracks().len(), 1);
        let track = &reader.tracks()[&1];
        assert_eq!(track.track_type().unwrap(), mp4::TrackType::Audio);
        assert_eq!(track.timescale(), 48000);
        assert_eq!(track.sample_count(), 3);
        for n in 0..3 {
            let sample = reader.read_sample(1, n + 1).unwrap().unwrap();
            assert_eq!(&sample.bytes[..], &[1, 2, 3, n as u8]);
            assert_eq!(sample.duration, 1024);
            assert_eq!(sample.start_time, u64::from(n) * 1024);
        }
    }
    #[test]
    fn remux_rejects_missing_audio_truncation_and_cancellation_without_partial_output() {
        let directory = tempfile::tempdir().unwrap();
        let input = directory.path().join("video.mp4");
        let cancel = tokio_util::sync::CancellationToken::new();
        fixture(&input, false);
        assert!(extract_audio(&input, directory.path(), &cancel).is_err());
        fixture(&input, true);
        cancel.cancel();
        assert!(extract_audio(&input, directory.path(), &cancel).is_err());
        std::fs::write(&input, b"\0\0\xff\xffmoov").unwrap();
        assert!(extract_audio(
            &input,
            directory.path(),
            &tokio_util::sync::CancellationToken::new()
        )
        .is_err());
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }
}
