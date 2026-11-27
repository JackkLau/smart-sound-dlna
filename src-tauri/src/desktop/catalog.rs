use super::{magnet, video_magnet_start, AppState, PickerGuard};
use crate::{
    catalog_queue::{BatchStatus, Queue},
    douban_catalog::{CatalogSync, RefreshReport, SyncReport},
    movie_discovery::{self, Candidate},
    movie_library::{Link, Movie, Store},
};
use std::{
    io::Read,
    path::{Path, PathBuf},
    sync::{atomic::Ordering, Arc},
};
use tauri::State;
use tauri_plugin_dialog::DialogExt;

pub(super) struct Catalog {
    db: Result<Arc<Store>, String>,
    sync: CatalogSync,
    queue: Queue,
}

impl Catalog {
    pub(super) fn new(directory: &Path, resource_dir: tauri::Result<PathBuf>) -> Self {
        let path = directory.join("movie-library.sqlite3");
        let db = resource_dir
            .map_err(|e| format!("无法定位应用资源目录：{e}"))
            .and_then(|dir| Store::open_packaged(&path, &dir.join("catalog/movie-library.sqlite3")))
            .map(Arc::new);
        Self {
            db,
            sync: CatalogSync::new(directory.join("douban-sync.json")),
            queue: Queue::new(path),
        }
    }

    fn db(&self) -> Result<Arc<Store>, String> {
        self.db.clone()
    }
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub(super) async fn movie_library_list(
    query: String,
    state: State<'_, AppState>,
) -> Result<Vec<Movie>, String> {
    let db = state.catalog.db()?;
    blocking(move || db.list(&query)).await
}

#[tauri::command]
pub(super) async fn movie_library_links(
    id: String,
    state: State<'_, AppState>,
) -> Result<Vec<Link>, String> {
    let db = state.catalog.db()?;
    blocking(move || db.links(&id)).await
}

#[tauri::command]
pub(super) async fn movie_library_search(
    id: String,
    state: State<'_, AppState>,
) -> Result<Vec<Candidate>, String> {
    let db = state.catalog.db()?;
    let key = id.clone();
    let movie = blocking(move || db.get(&key)).await?;
    let found = movie_discovery::search(&movie.title, &movie.original, movie.year).await?;
    state.catalog.queue.remember(&id, &found)?;
    state.catalog.queue.candidates(&id)
}

#[tauri::command]
pub(super) async fn movie_library_save(
    id: String,
    uri: String,
    source: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let db = state.catalog.db()?;
    let license = state
        .catalog
        .queue
        .candidate_license(&id, &uri, &source)?
        .unwrap_or_default();
    blocking(move || db.save_link_with_license(&id, &uri, &source, &license)).await
}

#[tauri::command]
pub(super) async fn movie_library_import(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<usize>, String> {
    let db = state.catalog.db()?;
    if state.picker_busy.swap(true, Ordering::AcqRel) {
        return Err("请先完成已打开的文件选择窗口。".into());
    }
    let guard = PickerGuard(state.picker_busy.clone());
    blocking(move || {
        let selected = app
            .dialog()
            .file()
            .set_title("导入豆瓣评分快照")
            .add_filter("JSON 片单", &["json"])
            .blocking_pick_file();
        drop(guard);
        let Some(selected) = selected else {
            return Ok(None);
        };
        let path = selected.into_path().map_err(|_| "无法读取所选文件。")?;
        let mut text = String::new();
        const LIMIT: u64 = 16 * 1024 * 1024;
        std::fs::File::open(path)
            .map_err(|e| e.to_string())?
            .take(LIMIT + 1)
            .read_to_string(&mut text)
            .map_err(|_| "片单应为 UTF-8 JSON 文件。")?;
        if text.len() as u64 > LIMIT {
            return Err("片单超过 16 MiB，请分批导入。".into());
        }
        db.import(&text).map(Some)
    })
    .await
}

#[tauri::command]
pub(super) async fn movie_library_start(
    id: String,
    uri: String,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<magnet::Snapshot>, String> {
    let db = state.catalog.db()?;
    let uri = blocking(move || {
        db.get(&id)?;
        db.links(&id)?
            .into_iter()
            .find(|link| link.uri == uri)
            .map(|link| link.uri)
            .ok_or_else(|| "请先确认并收录这个资源。".to_string())
    })
    .await?;
    video_magnet_start(uri, app, state).await
}

fn bundled_pages() -> &'static std::collections::HashSet<url::Url> {
    static PAGES: std::sync::OnceLock<std::collections::HashSet<url::Url>> =
        std::sync::OnceLock::new();
    PAGES.get_or_init(|| {
        let mut pages = std::collections::HashSet::new();
        for source in [
            include_str!("../../data/movie-seed.json"),
            include_str!("../../data/open-film-resources.json"),
        ] {
            let records: Vec<serde_json::Value> = serde_json::from_str(source).unwrap_or_default();
            for entry in records {
                for key in [
                    "source",
                    "license_url",
                    "rating_evidence",
                    "distribution_page",
                ] {
                    if let Some(url) = entry[key].as_str().and_then(|v| url::Url::parse(v).ok()) {
                        pages.insert(url);
                    }
                }
            }
        }
        pages
    })
}

fn source_page(input: &str) -> Result<url::Url, String> {
    let mut url = url::Url::parse(input).map_err(|_| "无效的来源网址。")?;
    if url.scheme() == "http" && url.host_str() == Some("creativecommons.org") {
        url.set_scheme("https").map_err(|_| "许可来源网址无效。")?;
    }
    let parts: Vec<_> = url.path().trim_matches('/').split('/').collect();
    // The curated snapshots may cite a specific paginated list or archival workbook.
    let bundled = bundled_pages().contains(&url);
    let valid = match (url.host_str(), parts.as_slice()) {
        (
            Some("github.com"),
            ["aaamqrx", "doban-data", "blob", "89aa62a129ec61c720392ba35774437d70e1d025", "data", "raw", name],
        ) => name.strip_suffix(".xlsx").is_some_and(|stem| {
            !stem.is_empty()
                && stem
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
        }),
        (Some("www.douban.com"), ["doulist", id]) => {
            !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit())
        }
        (Some("movie.douban.com"), ["chart" | "top250"]) => true,
        (Some("movie.douban.com"), ["subject", id]) => {
            !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit())
        }
        (Some("creativecommons.org"), ["licenses", kind, version, ..]) => {
            matches!(
                *kind,
                "by" | "by-sa" | "by-nd" | "by-nc" | "by-nc-sa" | "by-nc-nd"
            ) && matches!(*version, "1.0" | "2.0" | "2.5" | "3.0" | "4.0")
        }
        (Some("creativecommons.org"), ["publicdomain", "mark" | "zero", "1.0"]) => true,
        (Some("archive.org"), ["details", id]) => {
            !id.is_empty()
                && id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        }
        _ => bundled,
    };
    if !valid
        || url.scheme() != "https"
        || url.port().is_some()
        || url.password().is_some()
        || !url.username().is_empty()
        || (!bundled && (url.query().is_some() || url.fragment().is_some()))
    {
        return Err("仅支持查看片库支持的影片来源及授权页面。".into());
    }
    Ok(url)
}

#[tauri::command]
pub(super) async fn movie_library_open_source(url: String) -> Result<(), String> {
    let url = source_page(&url)?;
    blocking(move || {
        #[cfg(target_os = "macos")]
        let opener = "/usr/bin/open";
        #[cfg(windows)]
        let opener = "explorer.exe";
        #[cfg(not(any(target_os = "macos", windows)))]
        let opener = "xdg-open";
        std::process::Command::new(opener)
            .arg(url.as_str())
            .spawn()
            .map(|_| ())
            .map_err(|err| err.to_string())
    })
    .await
}

#[tauri::command]
pub(super) async fn movie_library_refresh(
    id: String,
    state: State<'_, AppState>,
) -> Result<RefreshReport, String> {
    state.catalog.sync.refresh(state.catalog.db()?, id).await
}

#[tauri::command]
pub(super) async fn movie_library_sync(state: State<'_, AppState>) -> Result<SyncReport, String> {
    state.catalog.sync.advance(state.catalog.db()?).await
}

#[tauri::command]
pub(super) async fn movie_library_batch_status(
    state: State<'_, AppState>,
) -> Result<BatchStatus, String> {
    state.catalog.db()?;
    state.catalog.queue.status()
}

#[tauri::command]
pub(super) async fn movie_library_batch_advance(
    state: State<'_, AppState>,
) -> Result<BatchStatus, String> {
    let db = state.catalog.db()?;
    state.catalog.queue.advance(&db).await
}

#[tauri::command]
pub(super) async fn movie_library_candidates(
    id: String,
    state: State<'_, AppState>,
) -> Result<Vec<Candidate>, String> {
    state.catalog.db()?;
    state.catalog.queue.candidates(&id)
}

#[tauri::command]
pub(super) async fn movie_library_remove(
    id: String,
    uri: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let db = state.catalog.db()?;
    blocking(move || db.remove_link(&id, &uri)).await
}

#[tauri::command]
pub(super) async fn movie_library_dismiss(
    id: String,
    uri: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state.catalog.db()?;
    state.catalog.queue.dismiss(&id, &uri)
}

#[tauri::command]
pub(super) async fn movie_library_review_ids(
    state: State<'_, AppState>,
) -> Result<Vec<String>, String> {
    state.catalog.db()?;
    state.catalog.queue.review_ids()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn browser_opens_only_canonical_catalog_sources() {
        for good in [
            "https://movie.douban.com/subject/1234/",
            "https://archive.org/details/BigBuckBunny_328",
        ] {
            assert!(source_page(good).is_ok());
        }
        for bad in [
            "file:///tmp/file",
            "https://archive.org@other.test/details/film",
            "https://archive.org/details/a?next=example",
            "https://archive.org/details/%2F",
            "https://movie.douban.com:444/subject/123/",
            "https://archive.org/details/",
        ] {
            assert!(source_page(bad).is_err());
        }
    }
}
