//! Local title recognition and the shared key used for spoken song names.
use lofty::{
    config::{apply_global_options, GlobalOptions, ParseOptions},
    file::{AudioFile, TaggedFileExt},
    probe::Probe,
    tag::Accessor,
};
use serde::Serialize;
use std::{
    fs::File,
    io::{self, BufReader, Read, Seek, SeekFrom},
    time::{Duration, Instant},
};

pub const MAX_TITLE_CHARS: usize = 120;

#[derive(Debug, Serialize)]
pub struct Suggestion {
    pub title: Option<String>,
    pub source: &'static str,
}

pub fn recognize(file: File, text: &str, video: bool) -> Suggestion {
    if let Some(title) = tag_title(file) {
        return Suggestion {
            title: Some(title),
            source: "audio_tag",
        };
    }
    Suggestion {
        title: from_text(text),
        source: if video { "video_title" } else { "filename" },
    }
}

// Bound parser IO and allocations; do not decode audio or cover art. All access
// goes through a Library-validated open handle, never a path supplied by the UI.
struct MetadataReader {
    file: File,
    remaining: usize,
    operations: usize,
    started: Instant,
}
impl MetadataReader {
    fn check(&mut self) -> io::Result<()> {
        if self.operations == 0 || self.started.elapsed() > Duration::from_secs(2) {
            return Err(io::Error::other("metadata read limit"));
        }
        self.operations -= 1;
        Ok(())
    }
}
impl Read for MetadataReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.check()?;
        if self.remaining == 0 {
            return Err(io::Error::other("metadata size limit"));
        }
        let limit = buf.len().min(self.remaining);
        let read = self.file.read(&mut buf[..limit])?;
        self.remaining -= read;
        Ok(read)
    }
}
impl Seek for MetadataReader {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.check()?;
        self.file.seek(position)
    }
}
/// Read container properties with the same bounded, validated handle as tags.
pub fn audio_duration(file: File) -> u64 {
    apply_global_options(
        GlobalOptions::new()
            .allocation_limit(1024 * 1024)
            .use_custom_resolvers(false)
            .preserve_format_specific_items(false),
    );
    let reader = BufReader::new(MetadataReader {
        file,
        remaining: 8 * 1024 * 1024,
        operations: 20_000,
        started: Instant::now(),
    });
    Probe::new(reader)
        .options(ParseOptions::new().read_tags(false).read_cover_art(false))
        .guess_file_type()
        .ok()
        .and_then(|p| p.read().ok())
        .map(|audio| audio.properties().duration().as_secs())
        .unwrap_or(0)
}

fn tag_title(file: File) -> Option<String> {
    apply_global_options(
        GlobalOptions::new()
            .allocation_limit(1024 * 1024)
            .use_custom_resolvers(false)
            .preserve_format_specific_items(false),
    );
    let reader = BufReader::new(MetadataReader {
        file,
        remaining: 8 * 1024 * 1024,
        operations: 20_000,
        started: Instant::now(),
    });
    let tagged = Probe::new(reader)
        .options(
            ParseOptions::new()
                .read_properties(false)
                .read_cover_art(false),
        )
        .guess_file_type()
        .ok()?
        .read()
        .ok()?;
    tagged
        .primary_tag()
        .into_iter()
        .chain(tagged.tags().iter())
        .filter_map(|tag| tag.title())
        .find_map(|title| usable(&title))
}

fn usable(title: &str) -> Option<String> {
    let title = validate(title).ok()?;
    let normalized = key(&title);
    if [
        "audio",
        "music",
        "unknown",
        "untitled",
        "未知",
        "未知歌曲",
        "未命名",
        "无标题",
    ]
    .contains(&normalized.as_str())
        || normalized
            .strip_prefix("track")
            .is_some_and(|rest| !rest.is_empty() && rest.bytes().all(|c| c.is_ascii_digit()))
        || normalized
            .strip_prefix('p')
            .is_some_and(|rest| !rest.is_empty() && rest.bytes().all(|c| c.is_ascii_digit()))
    {
        return None;
    }
    Some(title)
}

fn from_text(text: &str) -> Option<String> {
    // A single explicitly quoted song title is stronger evidence than the
    // surrounding video copy. A medley with several titles remains ambiguous.
    let titles: Vec<_> = text
        .split('《')
        .skip(1)
        .filter_map(|s| s.split_once('》').map(|(title, _)| title))
        .collect();
    if titles.len() == 1 {
        return usable(titles[0]);
    }
    if titles.len() > 1 {
        return None;
    }
    let mut name = text.trim();
    if let Some((prefix, suffix)) = name.rsplit_once(" [BV") {
        if suffix.ends_with(']')
            && suffix[..suffix.len() - 1]
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            name = prefix.trim();
        }
    }
    // Only remove track numbering when separated by a space (not "1.5").
    if let Some((number, rest)) = name.split_once(' ') {
        let number = number.trim_end_matches(['.', '-', '_']);
        if !number.is_empty() && number.len() <= 3 && number.bytes().all(|b| b.is_ascii_digit()) {
            name = rest.trim();
        }
    }
    let parts: Vec<_> = name.split(" - ").collect();
    if parts.len() == 2 && !parts[0].trim().is_empty() {
        name = parts[1].trim();
    }
    // Avoid pretending that obvious video copy, generic labels or compilations
    // are an identified song. Keep the existing title when no hint is reliable.
    if name.chars().count() > 50
        || [
            "这个",
            "合集",
            "盘点",
            "太燃",
            "必听",
            "入坑",
            "超清",
            "修复",
            "完整版",
            "BV1",
        ]
        .iter()
        .any(|word| name.contains(word))
    {
        return None;
    }
    usable(name)
}

pub fn key(text: &str) -> String {
    text.chars()
        .flat_map(char::to_lowercase)
        .filter(|c| c.is_alphanumeric())
        .collect()
}

pub fn validate(title: &str) -> Result<String, String> {
    let title = title.trim();
    if title.is_empty() || key(title).is_empty() {
        return Err("请输入可用于点播的曲名，至少包含一个文字或数字。".into());
    }
    if title.chars().count() > MAX_TITLE_CHARS {
        return Err(format!(
            "曲名最多 {MAX_TITLE_CHARS} 个字符，请缩短后再保存。"
        ));
    }
    if title.chars().any(char::is_control) {
        return Err("曲名不能包含换行或控制字符。".into());
    }
    Ok(title.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recognize_bytes(bytes: &[u8], title: &str) -> Suggestion {
        use std::io::Write;
        let mut file = tempfile::tempfile().unwrap();
        file.write_all(bytes).unwrap();
        file.rewind().unwrap();
        recognize(file, title, false)
    }

    #[test]
    fn duration_reads_pcm_properties_and_rejects_unknown_data() {
        use std::io::Write;
        let samples = 8000u32 * 3;
        let mut wav = b"RIFF".to_vec();
        wav.extend_from_slice(&(36 + samples).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
        wav.extend_from_slice(&1u16.to_le_bytes()); // mono
        wav.extend_from_slice(&8000u32.to_le_bytes());
        wav.extend_from_slice(&8000u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&8u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&samples.to_le_bytes());
        wav.extend(vec![128; samples as usize]);
        for (bytes, expected) in [(&wav[..], 3), (b"not audio".as_slice(), 0)] {
            let mut f = tempfile::tempfile().unwrap();
            f.write_all(bytes).unwrap();
            f.rewind().unwrap();
            assert_eq!(audio_duration(f), expected);
        }
    }

    #[test]
    fn id3_utf8_title_wins_over_filename_and_mp4_title_is_read() {
        let title = "夏天与海";
        let mut frame = vec![3]; // ID3v2.4 UTF-8 encoding.
        frame.extend_from_slice(title.as_bytes());
        let mut body = b"TIT2".to_vec();
        body.extend_from_slice(&(frame.len() as u32).to_be_bytes());
        body.extend_from_slice(&[0, 0]);
        body.extend(frame);
        let mut bytes = b"ID3\x04\0\0".to_vec();
        bytes.extend_from_slice(&(body.len() as u32).to_be_bytes());
        bytes.extend(body);
        for _ in 0..3 {
            bytes.extend_from_slice(&[0xff, 0xfb, 0x90, 0x64]);
            bytes.extend(vec![0; 413]);
        }
        let suggestion = recognize_bytes(&bytes, "wrong title");
        assert_eq!(suggestion.title.as_deref(), Some(title));
        assert_eq!(suggestion.source, "audio_tag");

        fn atom(kind: &[u8; 4], content: &[u8]) -> Vec<u8> {
            let mut result = ((content.len() + 8) as u32).to_be_bytes().to_vec();
            result.extend_from_slice(kind);
            result.extend_from_slice(content);
            result
        }
        let mut data = vec![0, 0, 0, 1, 0, 0, 0, 0]; // UTF-8, locale 0.
        data.extend_from_slice(title.as_bytes());
        let ilst = atom(b"ilst", &atom(b"\xa9nam", &atom(b"data", &data)));
        let mut meta = vec![0; 4];
        meta.extend(ilst);
        let mut mp4 = atom(b"ftyp", b"M4A \0\0\0\0M4A isom");
        mp4.extend(atom(b"moov", &atom(b"udta", &atom(b"meta", &meta))));
        let suggestion = recognize_bytes(&mp4, "wrong title");
        assert_eq!(suggestion.title.as_deref(), Some(title));
        assert_eq!(suggestion.source, "audio_tag");
    }

    #[test]
    fn broken_metadata_falls_back_and_oversized_input_is_bounded() {
        assert_eq!(
            recognize_bytes(b"not audio", "01. 歌手 - 晴天")
                .title
                .as_deref(),
            Some("晴天")
        );
        assert!(recognize_bytes(b"ID3\x04\0\0\x7f\x7f\x7f\x7f", "audio")
            .title
            .is_none());
        let mut reader = MetadataReader {
            file: tempfile::tempfile().unwrap(),
            remaining: 0,
            operations: 1,
            started: Instant::now(),
        };
        assert!(reader.read(&mut [0; 1]).is_err());
        assert!(reader.seek(SeekFrom::Start(0)).is_err());
    }

    #[test]
    fn quoted_song_names_numbered_files_and_ambiguous_videos() {
        assert_eq!(
            from_text("【4K修复】谢霆锋《活着Viva》超清MV").as_deref(),
            Some("活着Viva")
        );
        assert_eq!(
            from_text("01. 周杰伦 - 晴天 [BV1test-123]").as_deref(),
            Some("晴天")
        );
        assert_eq!(from_text("晴天 (Live)").as_deref(), Some("晴天 (Live)"));
        for title in [
            "audio",
            "Track01",
            "P1",
            "这个螺旋不插电版本现场太燃了！",
            "《晴天》《稻香》串烧",
        ] {
            assert!(from_text(title).is_none(), "{title}");
        }
    }
}
