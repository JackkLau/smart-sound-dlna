//! Clipboard text is transient. Only validated magnet links cross back into the main window.
use crate::magnet::parse_magnet;
use serde::Serialize;
use std::collections::HashSet;

const MAX_BYTES: usize = 256 * 1024;
const MAX_LINKS: usize = 20;
const TOO_LARGE: &str = "文本过长，请只复制包含磁力链接的段落。";

#[derive(Debug, Serialize)]
pub struct Candidate {
    id: String,
    title: String,
    uri: String,
}

pub fn read_links(text: Option<String>) -> Result<Vec<Candidate>, String> {
    let text = match text {
        Some(text) => text,
        None => clipboard_text()?,
    };
    if text.len() > MAX_BYTES {
        return Err(TOO_LARGE.into());
    }
    let folded = text.to_ascii_lowercase();
    let starts: Vec<_> = folded.match_indices("magnet:?").map(|(i, _)| i).collect();
    let mut seen = HashSet::new();
    let mut candidates = Vec::new();
    for (index, &start) in starts.iter().enumerate() {
        let end = starts.get(index + 1).copied().unwrap_or(text.len());
        let segment = &text[start..end];
        let end = segment
            .find(|c: char| {
                c.is_whitespace() || c.is_control() || "<>\"“”‘’「」【】，。；！？".contains(c)
            })
            .unwrap_or(segment.len());
        let mut link = &segment[..end];
        // Strip matching prose wrappers without removing parentheses inside a tracker URL.
        if let Some(close) = text[..start].chars().next_back().and_then(|c| match c {
            '(' => Some(')'),
            '（' => Some('）'),
            '[' => Some(']'),
            '\'' => Some('\''),
            '`' => Some('`'),
            _ => None,
        }) {
            if let Some(cut) = link.trim_end_matches(['.', ',', ';']).strip_suffix(close) {
                link = cut;
            }
        }
        let link = link.replace("&amp;", "&");
        let parsed = parse_magnet(&link)
            .map(|v| (v, link.as_str()))
            .or_else(|_| {
                let trimmed = link.trim_end_matches(['.', ',', ';', ')', ']', '）']);
                parse_magnet(trimmed).map(|v| (v, trimmed))
            });
        if let Ok(((id, title, _), uri)) = parsed {
            if seen.insert(id.clone()) {
                if candidates.len() == MAX_LINKS {
                    return Err(format!("一次最多识别 {MAX_LINKS} 个不同资源，请分段复制。"));
                }
                candidates.push(Candidate {
                    id,
                    title,
                    uri: uri.to_owned(),
                });
            }
        }
    }
    Ok(candidates)
}

#[cfg(target_os = "macos")]
fn clipboard_text() -> Result<String, String> {
    use objc2::{msg_send, rc::Retained, ClassType};
    use objc2_app_kit::NSPasteboard;
    // The pasteboard service may be unavailable in a restricted login session.
    // Accept nil even though the generated AppKit binding declares this nonnull.
    let board: Option<Retained<NSPasteboard>> =
        unsafe { msg_send![NSPasteboard::class(), generalPasteboard] };
    let board = board.ok_or("系统剪贴板暂不可用，请手动粘贴。")?;
    mac_board_text(&board)
}

#[cfg(target_os = "macos")]
fn mac_board_text(board: &objc2_app_kit::NSPasteboard) -> Result<String, String> {
    use objc2_app_kit::NSPasteboardTypeString;
    // AppKit's exported constant is a process-lifetime immutable string.
    let Some(value) = board.stringForType(unsafe { NSPasteboardTypeString }) else {
        return Ok(String::new());
    };
    if value.length() > MAX_BYTES {
        return Err(TOO_LARGE.into());
    }
    Ok(value.to_string())
}

#[cfg(windows)]
fn clipboard_text() -> Result<String, String> {
    use windows_sys::Win32::System::{
        DataExchange::{
            CloseClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
        },
        Memory::{GlobalLock, GlobalSize, GlobalUnlock},
    };
    const CF_UNICODETEXT: u32 = 13;
    const UNAVAILABLE: &str = "剪贴板暂时无法读取，请重试或手动粘贴。";
    struct OpenBoard;
    impl Drop for OpenBoard {
        fn drop(&mut self) {
            unsafe {
                CloseClipboard();
            }
        }
    }
    // The clipboard owns the handle. Keep it open until the bounded UTF-16 data
    // has been copied, then release the lock before the clipboard guard drops.
    unsafe {
        if IsClipboardFormatAvailable(CF_UNICODETEXT) == 0 {
            return Ok(String::new());
        }
        if OpenClipboard(std::ptr::null_mut()) == 0 {
            return Err(UNAVAILABLE.into());
        }
        let _board = OpenBoard;
        let handle = GetClipboardData(CF_UNICODETEXT);
        if handle.is_null() {
            return Err(UNAVAILABLE.into());
        }
        let units = GlobalSize(handle) / 2;
        if units > MAX_BYTES + 1 {
            return Err(TOO_LARGE.into());
        }
        let ptr = GlobalLock(handle).cast::<u16>();
        if ptr.is_null() {
            return Err(UNAVAILABLE.into());
        }
        let data = std::slice::from_raw_parts(ptr, units);
        let text = data
            .iter()
            .position(|v| *v == 0)
            .map(|end| String::from_utf16_lossy(&data[..end]));
        GlobalUnlock(handle);
        text.ok_or_else(|| UNAVAILABLE.into())
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
fn clipboard_text() -> Result<String, String> {
    Err("此系统请使用手动粘贴。".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    const HEX: &str = "0123456789abcdef0123456789abcdef01234567";

    #[test]
    fn extracts_only_valid_distinct_resources_in_source_order() {
        let share = format!("私密备注【MAGNET:?xt=urn:btih:{HEX}&amp;dn=电影】\n磁力：magnet:?xt=urn:btih:{HEX}。\n[另一项](magnet:?xt=urn:btih:{})\nmagnet:?xt=urn:btmh:xxx 无关文字", "A".repeat(32));
        let links = read_links(Some(share)).unwrap();
        assert_eq!(links.len(), 2);
        assert_eq!(links[0].id, HEX);
        assert_eq!(links[0].title, "电影");
        assert!(!links[0].uri.contains("&amp;"));
        assert_eq!(links[1].id, "0".repeat(40));
        assert!(!serde_json::to_string(&links).unwrap().contains("私密备注"));
    }

    #[test]
    fn preserves_tracker_query_and_handles_plain_prose_and_empty_input() {
        let uri =
            format!("magnet:?xt=urn:btih:{HEX}&tr=https%3A%2F%2Fexample.test%2Fa%3Fx%3D1%26y%3D2");
        let links = read_links(Some(format!("请看 [{uri}]"))).unwrap();
        assert_eq!(links[0].uri, uri);
        assert!(read_links(Some("这是普通文本。magnet:?dn=无校验值".into()))
            .unwrap()
            .is_empty());
        assert_eq!(
            read_links(Some(format!("See magnet:?xt=urn:btih:{HEX}."))).unwrap()[0].id,
            HEX
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "requires an interactive macOS pasteboard service"]
    fn native_private_pasteboard_round_trip() {
        use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString};
        let board = NSPasteboard::pasteboardWithUniqueName();
        let sample = board.name();
        board.clearContents();
        assert!(board.setString_forType(&sample, unsafe { NSPasteboardTypeString }));
        assert_eq!(mac_board_text(&board).unwrap(), sample.to_string());
        board.clearContents();
    }

    #[test]
    fn enforces_text_and_distinct_candidate_limits() {
        assert!(read_links(Some("a".repeat(MAX_BYTES + 1))).is_err());
        let text = (0..MAX_LINKS + 1)
            .map(|n| format!("magnet:?xt=urn:btih:{n:040x}"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(read_links(Some(text)).is_err());
    }
}
