//! Resolve the packaged probe lazily: music never depends on this executable.
use std::path::{Path, PathBuf};

fn sibling_tool(executable: &Path, name: &str) -> Option<PathBuf> {
    executable
        .parent()
        .map(|dir| dir.join(format!("{name}{}", std::env::consts::EXE_SUFFIX)))
}

#[cfg(debug_assertions)]
fn development_tool(name: &str) -> PathBuf {
    let target = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("windows", "x86_64") => "x86_64-pc-windows-msvc",
        _ => "unsupported",
    };
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("binaries")
        .join(format!(
            "{name}-{target}{}",
            if cfg!(windows) { ".exe" } else { "" }
        ))
}

fn resolve(name: &str) -> Result<PathBuf, String> {
    if let Some(path) = std::env::current_exe()
        .ok()
        .and_then(|app| sibling_tool(&app, name))
        .filter(|p| p.is_file())
    {
        return Ok(path);
    }
    #[cfg(debug_assertions)]
    {
        let path = development_tool(name);
        if path.is_file() {
            return Ok(path);
        }
    }
    Err(format!("缺少影视工具 {name}。开发环境请运行 npm run prepare:video，安装版请使用包含影视工具的安装包。"))
}

pub fn ffprobe_path() -> Result<PathBuf, String> {
    resolve("ffprobe")
}

pub fn ffmpeg_path() -> Result<PathBuf, String> {
    resolve("ffmpeg")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packaged_probe_uses_executable_directory_instead_of_path_lookup() {
        let root = std::env::temp_dir().join("xiaomi video tool location");
        let executable = root.join("app");
        let resolved = sibling_tool(&executable, "ffprobe").unwrap();
        assert_eq!(resolved.parent(), Some(root.as_path()));
        assert_eq!(
            resolved.file_name().unwrap(),
            if cfg!(windows) {
                "ffprobe.exe"
            } else {
                "ffprobe"
            }
        );
    }
}
