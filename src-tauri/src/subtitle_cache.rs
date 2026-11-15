//! Leased private work directories, including recovery after an unclean app exit.
use std::{
    fs::{self, File},
    path::{Path, PathBuf},
};
use tempfile::TempDir;

#[derive(Debug)]
pub(crate) struct Workspace {
    // Windows must release the lock handle before removing its directory.
    _lease: File,
    directory: TempDir,
}

impl Workspace {
    pub(crate) fn new(root: &Path) -> Result<Self, String> {
        Self::create(root).map_err(|_| "无法创建字幕临时目录，请检查磁盘空间和目录权限。".into())
    }

    fn create(root: &Path) -> std::io::Result<Self> {
        fs::create_dir_all(root)?;
        // Serialize directory creation and reclamation across application instances.
        let guard = open_lock(root.join("index.lock"))?;
        guard.lock()?;
        for entry in fs::read_dir(root)?.flatten() {
            if !entry.file_name().to_string_lossy().starts_with("job-")
                || !entry.file_type()?.is_dir()
            {
                continue;
            }
            let lease = open_lock(entry.path().join("lease"));
            if let Ok(lease) = lease {
                if lease.try_lock().is_ok() {
                    drop(lease);
                    let _ = fs::remove_dir_all(entry.path());
                }
            }
        }
        let directory = tempfile::Builder::new().prefix("job-").tempdir_in(root)?;
        let lease = open_lock(directory.path().join("lease"))?;
        lease.lock()?;
        Ok(Self {
            _lease: lease,
            directory,
        })
    }

    pub(crate) fn path(&self) -> &Path {
        self.directory.path()
    }
}

fn open_lock(path: PathBuf) -> std::io::Result<File> {
    File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
}

// Stop a long encode before its temporary file consumes the remaining working space.
pub(crate) fn check_space(directory: &Path) -> Result<(), String> {
    let bytes = available_space(directory).map_err(|_| "无法查询字幕工作目录的可用空间。")?;
    if bytes < 256 * 1024 * 1024 {
        return Err("可用空间不足 256 MiB，字幕处理已停止。请释放磁盘空间后重新准备。".into());
    }
    Ok(())
}

#[cfg(unix)]
pub(crate) fn available_space(directory: &Path) -> std::io::Result<u64> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    let path = CString::new(directory.as_os_str().as_bytes())?;
    let mut info = std::mem::MaybeUninit::uninit();
    // statvfs fills the provided structure only when the call succeeds.
    if unsafe { libc::statvfs(path.as_ptr(), info.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    let info = unsafe { info.assume_init() };
    Ok((u128::from(info.f_bavail) * u128::from(info.f_frsize)).min(u128::from(u64::MAX)) as u64)
}

#[cfg(windows)]
pub(crate) fn available_space(directory: &Path) -> std::io::Result<u64> {
    use std::os::windows::ffi::OsStrExt;
    let path: Vec<u16> = directory.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut bytes = 0;
    // The two unused output parameters are optional.
    if unsafe {
        windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(
            path.as_ptr(),
            &mut bytes,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error());
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reclaim_abandoned_files_and_keep_live_subtitle_sessions() {
        let root = tempfile::tempdir().unwrap();
        let live = Workspace::new(root.path()).unwrap();
        let abandoned = root.path().join("job-abandoned");
        fs::create_dir(&abandoned).unwrap();
        fs::write(abandoned.join("movie.mkv"), b"partial").unwrap();
        let other = Workspace::new(root.path()).unwrap();
        assert!(live.path().is_dir());
        assert!(!abandoned.exists());
        let path = other.path().to_owned();
        drop(other);
        assert!(!path.exists());
    }
}
