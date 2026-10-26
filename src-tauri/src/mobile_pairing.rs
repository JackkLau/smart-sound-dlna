//! Remember one phone using only its credential digest, never its bearer token.
use std::{io::Write, path::Path};

pub fn digest(token: &str) -> String {
    ring::digest::digest(&ring::digest::SHA256, token.as_bytes())
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub fn load(path: &Path) -> Result<Option<String>, String> {
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("无法读取手机配对记录，请检查应用数据目录的权限。".into()),
    };
    if metadata.len() != 64 || !metadata.is_file() {
        return Err("手机配对记录无效，请点击“忘记手机并重新配对”。".into());
    }
    let hash = std::fs::read_to_string(path).map_err(|_| "无法读取手机配对记录。")?;
    if hash.len() != 64 || !hash.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err("手机配对记录无效，请点击“忘记手机并重新配对”。".into());
    }
    Ok(Some(hash.to_ascii_lowercase()))
}

pub fn save(path: &Path, hash: &str) -> Result<(), String> {
    let parent = path.parent().ok_or("手机配对保存位置无效。")?;
    let write = || -> std::io::Result<()> {
        std::fs::create_dir_all(parent)?;
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        file.write_all(hash.as_bytes())?;
        file.as_file().sync_all()?;
        file.persist(path).map_err(|e| e.error)?;
        Ok(())
    };
    write()
        .map_err(|_| "无法保存手机配对，尚未连接。请检查电脑磁盘及应用数据目录权限后重试。".into())
}

pub fn forget(path: &Path) -> Result<(), String> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err("无法清除手机配对记录，接收已暂停。请检查应用数据目录权限后重试。".into()),
    }
}
