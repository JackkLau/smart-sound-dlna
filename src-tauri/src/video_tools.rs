//! Resolve the packaged probe lazily: music never depends on this executable.
use std::path::{Path, PathBuf};

fn sibling_probe(executable: &Path) -> Option<PathBuf> {
    executable.parent().map(|directory| {
        directory.join(if cfg!(windows) {
            "ffprobe.exe"
        } else {
            "ffprobe"
        })
    })
}

#[cfg(debug_assertions)]
fn development_probe() -> PathBuf {
    let target = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("windows", "x86_64") => "x86_64-pc-windows-msvc",
        _ => "unsupported",
    };
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("binaries")
        .join(format!(
            "ffprobe-{target}{}",
            if cfg!(windows) { ".exe" } else { "" }
        ))
}

pub fn ffprobe_path() -> Result<PathBuf, String> {
    let sibling = std::env::current_exe()
        .ok()
        .and_then(|path| sibling_probe(&path));
    if let Some(path) = sibling.filter(|path| path.is_file()) {
        return Ok(path);
    }
    #[cfg(debug_assertions)]
    {
        let path = development_probe();
        if path.is_file() {
            return Ok(path);
        }
    }
    Err("缺少影片分析工具。开发环境请运行 npm run prepare:video；安装版请使用包含影视工具的安装包。音乐功能仍可正常使用。".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packaged_probe_uses_executable_directory_instead_of_path_lookup() {
        let root = std::env::temp_dir().join("xiaomi video tool location");
        let executable = root.join("app");
        let resolved = sibling_probe(&executable).unwrap();
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
