//! Favorites are metadata only. Downloads still require an explicitly selected CID.
use super::*;
use std::collections::HashSet;

const PAGE_SIZE: u32 = 20;

#[derive(Debug, Serialize)]
pub struct FavoriteFolder {
    pub id: String,
    pub title: String,
    pub total: u64,
}

#[derive(Debug, Serialize)]
pub struct FavoriteVideo {
    pub bvid: String,
    pub title: String,
    pub author: String,
    pub parts: u64,
    pub duration: u64,
    pub unavailable: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct FavoritePage {
    pub id: String,
    pub title: String,
    pub owner: String,
    pub total: u64,
    pub page: u32,
    pub has_more: bool,
    pub videos: Vec<FavoriteVideo>,
}

#[derive(Debug, Deserialize)]
pub struct BatchSelection {
    pub preview_id: String,
    pub cids: Vec<u64>,
}

#[derive(Debug, Serialize)]
pub struct BatchItem {
    pub preview_id: String,
    pub cid: u64,
    pub title: String,
    pub job: Option<DownloadJob>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct BatchResult {
    pub items: Vec<BatchItem>,
}

impl Downloader {
    pub async fn favorite_folders(&self) -> Result<Vec<FavoriteFolder>, String> {
        let cookie = self.cookie.lock().await.clone();
        if cookie.is_empty() {
            return Err("请先在下方登录 Bilibili，再读取自己的收藏夹。".into());
        }
        let nav = self.api("/x/web-interface/nav", &[], &cookie).await?;
        if nav["data"]["isLogin"].as_bool() != Some(true) {
            return Err("Bilibili 登录已过期，请在下方重新登录。".into());
        }
        let uid = nav["data"]["mid"]
            .as_u64()
            .filter(|id| *id > 0)
            .ok_or("无法确认 Bilibili 账号，请重新登录。")?;
        let response = self
            .api(
                "/x/v3/fav/folder/created/list-all",
                &[("up_mid".into(), uid.to_string())],
                &cookie,
            )
            .await?;
        favorite_code(&response)?;
        let data = &response["data"];
        if !data.is_object() {
            return Err("收藏夹列表暂时不可见，请直接粘贴收藏夹链接或重新登录后重试。".into());
        }
        if data["count"].as_u64() == Some(0) && data["list"].is_null() {
            return Ok(Vec::new());
        }
        data["list"]
            .as_array()
            .ok_or("收藏夹列表格式已变化，请稍后重试。")?
            .iter()
            .map(|item| {
                Ok(FavoriteFolder {
                    id: positive_id(&item["id"])?.to_string(),
                    title: text_value(&item["title"], "未命名收藏夹"),
                    total: item["media_count"].as_u64().unwrap_or(0),
                })
            })
            .collect()
    }

    pub async fn favorite_page(&self, input: &str, page: u32) -> Result<FavoritePage, String> {
        let id = favorite_id(input)?;
        if !(1..=100_000).contains(&page) {
            return Err("收藏夹页码无效，请重新读取收藏夹。".into());
        }
        let cookie = self.request_cookie().await?;
        let response = self
            .api(
                "/x/v3/fav/resource/list",
                &[
                    ("media_id".into(), id.to_string()),
                    ("pn".into(), page.to_string()),
                    ("ps".into(), PAGE_SIZE.to_string()),
                    ("order".into(), "mtime".into()),
                    ("type".into(), "0".into()),
                    ("tid".into(), "0".into()),
                    ("platform".into(), "web".into()),
                ],
                &cookie,
            )
            .await?;
        parse_favorite_page(&response, id, page)
    }

    pub async fn start_batch(
        self: &Arc<Self>,
        engine: Weak<Engine>,
        selections: Vec<BatchSelection>,
        shutdown: CancellationToken,
    ) -> Result<BatchResult, String> {
        let count = selections.iter().try_fold(0usize, |count, selection| {
            count.checked_add(selection.cids.len())
        });
        if !matches!(count, Some(1..=MAX_QUEUE)) || selections.len() > MAX_QUEUE {
            return Err("请选择 1 至 100 首音轨后再下载。".into());
        }
        // Validate the entire selection before starting any work. A stale preview
        // or an invalid CID must not silently submit only half of a selection.
        let previews = self.previews.lock().await;
        let mut tracks = Vec::new();
        for selection in selections {
            let preview = previews
                .get(&selection.preview_id)
                .filter(|p| online::now_ms().saturating_sub(p.created) < PREVIEW_TTL)
                .ok_or("部分音轨预览已过期，请重新解析相应视频后再下载。")?;
            for cid in selection.cids {
                let part = preview
                    .parts
                    .iter()
                    .find(|part| part.cid == cid)
                    .ok_or("选择中包含无效的分 P，请重新解析视频。")?;
                tracks.push((preview.clone(), part.clone()));
            }
        }
        drop(previews);
        let mut seen = HashSet::new();
        let mut items = Vec::with_capacity(tracks.len());
        for (preview, part) in tracks {
            let preview_id = preview.id.clone();
            let cid = part.cid;
            let title = if preview.parts.len() == 1 {
                preview.title.clone()
            } else {
                format!("{} · P{} {}", preview.title, part.page, part.title)
            };
            let result = if seen.insert((preview.bvid.clone(), cid)) {
                self.enqueue(engine.clone(), preview, part, shutdown.clone())
                    .await
            } else {
                Err("已跳过本次选择中重复的音轨。".into())
            };
            let (job, error) = match result {
                Ok(job) => (Some(job), None),
                Err(error) => (None, Some(error)),
            };
            items.push(BatchItem {
                preview_id,
                cid,
                title,
                job,
                error,
            });
        }
        Ok(BatchResult { items })
    }
}

fn positive_id(value: &Value) -> Result<u64, String> {
    value
        .as_u64()
        .filter(|id| *id > 0)
        .ok_or_else(|| "收藏夹信息不完整，请稍后重试。".into())
}

fn text_value(value: &Value, fallback: &str) -> String {
    value
        .as_str()
        .unwrap_or(fallback)
        .chars()
        .take(300)
        .collect()
}

fn favorite_code(response: &Value) -> Result<(), String> {
    match response["code"].as_i64() {
        Some(-403 | -101 | -10403) => {
            Err("无权读取此收藏夹，请在下方登录拥有访问权限的 Bilibili 账号后重试。".into())
        }
        Some(-404 | 11010 | 11011 | 11201) => {
            Err("收藏夹不存在或已被删除，请检查链接中的 fid。".into())
        }
        _ => check_code(response),
    }
}

fn favorite_id(input: &str) -> Result<u64, String> {
    const HELP: &str = "请粘贴带 fid 的 Bilibili 收藏夹链接，或填写收藏夹 ID。";
    let input = input.trim();
    if input.len() > 2048 {
        return Err(HELP.into());
    }
    fn number(input: &str) -> Option<u64> {
        if input.is_empty() || !input.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        input.parse().ok().filter(|n| *n > 0)
    }
    if let Some(id) = number(input.strip_prefix("ml").unwrap_or(input)) {
        return Ok(id);
    }
    let url = Url::parse(input).map_err(|_| HELP)?;
    if !matches!(url.scheme(), "https" | "http")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
    {
        return Err(HELP.into());
    }
    let path: Vec<_> = url.path().trim_matches('/').split('/').collect();
    match url.host_str() {
        Some("space.bilibili.com")
            if path.len() == 2
                && number(path[0]).is_some()
                && matches!(path[1], "favlist" | "lists") =>
        {
            let ids: Vec<_> = url.query_pairs().filter(|(k, _)| k == "fid").collect();
            if ids.len() == 1 {
                return number(&ids[0].1).ok_or_else(|| HELP.into());
            }
        }
        Some("www.bilibili.com" | "bilibili.com") => {
            let candidate = match path.as_slice() {
                ["medialist", "detail" | "play", id] | ["list", id] => Some(*id),
                _ => None,
            };
            if let Some(id) = candidate
                .and_then(|id| id.strip_prefix("ml"))
                .and_then(number)
            {
                return Ok(id);
            }
        }
        _ => {}
    }
    Err(HELP.into())
}

fn parse_favorite_page(response: &Value, id: u64, page: u32) -> Result<FavoritePage, String> {
    favorite_code(response)?;
    let data = &response["data"];
    let info = &data["info"];
    if positive_id(&info["id"])? != id {
        return Err("返回的收藏夹与请求不一致，请重新读取。".into());
    }
    let total = info["media_count"].as_u64().ok_or("收藏夹数量信息缺失。")?;
    let has_more = data["has_more"].as_bool().ok_or("收藏夹分页信息缺失。")?;
    let empty = Vec::new();
    let medias = if data["medias"].is_null() && !has_more {
        &empty
    } else {
        data["medias"]
            .as_array()
            .ok_or("收藏夹内容格式已变化，请稍后重试。")?
    };
    if medias.len() > PAGE_SIZE as usize || (has_more && medias.is_empty()) {
        return Err("收藏夹分页信息异常，请稍后重试。".into());
    }
    let videos = medias
        .iter()
        .map(|item| {
            let bvid = item["bvid"]
                .as_str()
                .filter(|id| valid_bvid(id))
                .or_else(|| item["bv_id"].as_str().filter(|id| valid_bvid(id)))
                .unwrap_or_default()
                .to_string();
            let title = text_value(&item["title"], "已失效视频");
            let unavailable = if item["type"].as_u64() != Some(2) {
                Some("暂不支持此收藏类型，仅支持普通投稿视频。".into())
            } else if bvid.is_empty() || title == "已失效视频" {
                Some("视频已失效，无法读取音轨。".into())
            } else {
                None
            };
            FavoriteVideo {
                bvid,
                title,
                author: text_value(&item["upper"]["name"], "未知作者"),
                parts: item["page"].as_u64().unwrap_or(1),
                duration: item["duration"].as_u64().unwrap_or(0),
                unavailable,
            }
        })
        .collect();
    Ok(FavoritePage {
        id: id.to_string(),
        title: text_value(&info["title"], "未命名收藏夹"),
        owner: text_value(&info["upper"]["name"], "未知作者"),
        total,
        page,
        has_more,
        videos,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn favorites_input_is_an_id_not_an_arbitrary_fetch_url() {
        for input in [
            "3359576528",
            "ml3359576528",
            "https://space.bilibili.com/319110828/favlist?fid=3359576528&ftype=create",
            "https://space.bilibili.com/319110828/lists?fid=3359576528",
            "https://www.bilibili.com/medialist/detail/ml3359576528",
            "https://www.bilibili.com/list/ml3359576528",
        ] {
            assert_eq!(favorite_id(input).unwrap(), 3359576528);
        }
        for input in [
            "0",
            "-1",
            "18446744073709551616",
            "https://example.com/?fid=1",
            "https://space.bilibili.com.evil.org/1/favlist?fid=2",
            "https://space.bilibili.com/1/favlist?fid=2&fid=3",
            "https://user@space.bilibili.com/1/favlist?fid=2",
            "https://space.bilibili.com:9000/1/favlist?fid=2",
            "https://space.bilibili.com/1",
            "https://www.bilibili.com/video/BV1sVtw6JEeq/",
        ] {
            assert!(favorite_id(input).is_err(), "{input}");
        }
    }

    #[test]
    fn pages_keep_single_multi_and_unavailable_items_distinct() {
        let response = json!({"code":0,"data":{"info":{"id":12,"title":"歌单","media_count":22,"upper":{"name":"作者"}},"has_more":true,"medias":[
            {"bvid":"BV1sVtw6JEeq","type":2,"title":"单曲","page":1},
            {"bv_id":"BV1xx411c7mD","type":2,"title":"现场","page":3},
            {"bvid":"BV1xx411c7mD","type":2,"title":"已失效视频"},
            {"type":24,"title":"收藏的其他内容"}
        ]}});
        let page = parse_favorite_page(&response, 12, 1).unwrap();
        assert!(page.has_more);
        assert_eq!(page.total, 22);
        assert_eq!(page.videos[0].parts, 1);
        assert_eq!(page.videos[1].parts, 3);
        assert!(page.videos[2].unavailable.is_some());
        assert!(page.videos[3].unavailable.is_some());
        assert!(parse_favorite_page(&response, 13, 1).is_err());
        assert!(parse_favorite_page(&json!({"code":-403}), 12, 1)
            .unwrap_err()
            .contains("权限"));
        assert!(parse_favorite_page(&json!({"code":0,"data":null}), 12, 1).is_err());
        let empty = json!({"code":0,"data":{"info":{"id":12,"media_count":0},"has_more":false,"medias":null}});
        assert!(parse_favorite_page(&empty, 12, 1)
            .unwrap()
            .videos
            .is_empty());
    }

    #[tokio::test]
    async fn batch_validates_before_enqueue_and_deduplicates_across_previews() {
        let directory = tempfile::tempdir().unwrap();
        let engine = Engine::new(directory.path().to_owned()).await.unwrap();
        let downloader = &engine.downloads;
        let permit = downloader.queue.acquire().await.unwrap();
        let mut preview = parse_preview(
            &json!({"bvid":"BV1xx411c7mD","title":"现场","pages":[
                {"cid":1,"page":1,"part":"第一首"}, {"cid":2,"page":2,"part":"第二首"}
            ]}),
            1,
        )
        .unwrap();
        preview.id = "multi".into();
        downloader
            .previews
            .lock()
            .await
            .insert(preview.id.clone(), preview.clone());
        let selection = |id: &str, cids: Vec<u64>| BatchSelection {
            preview_id: id.into(),
            cids,
        };
        let submit = |selections| {
            downloader.start_batch(Arc::downgrade(&engine), selections, engine.cancellation())
        };
        assert!(submit(vec![
            selection("multi", vec![1]),
            selection("expired", vec![2])
        ])
        .await
        .is_err());
        assert!(submit(vec![selection("multi", vec![1, 999])])
            .await
            .is_err());
        assert!(submit(vec![selection("multi", vec![1; 101])])
            .await
            .is_err());
        assert!(downloader.state().await.jobs.is_empty());
        preview.id = "again".into();
        downloader
            .previews
            .lock()
            .await
            .insert(preview.id.clone(), preview);
        let result = submit(vec![
            selection("multi", vec![1, 2]),
            selection("again", vec![1]),
        ])
        .await
        .unwrap();
        assert_eq!(result.items.iter().filter(|i| i.job.is_some()).count(), 2);
        assert!(result.items[2].error.as_ref().unwrap().contains("重复"));
        let duplicate = submit(vec![selection("multi", vec![2])]).await.unwrap();
        assert!(duplicate.items[0].error.as_ref().unwrap().contains("队列"));
        assert_eq!(downloader.state().await.jobs.len(), 2);
        engine.cancellation().cancel();
        drop(permit);
    }

    #[tokio::test]
    async fn favorites_http_to_single_and_multi_part_library_import() {
        use axum::{
            extract::Query,
            http::{HeaderMap, Uri},
            response::{IntoResponse, Response},
            routing::get,
            Router,
        };
        async fn serve(
            uri: Uri,
            Query(query): Query<HashMap<String, String>>,
            headers: HeaderMap,
        ) -> Response {
            let response = match uri.path() {
                "/x/frontend/finger/spi" => {
                    json!({"code":0,"data":{"b_3":"visitor-three","b_4":"visitor-four"}})
                }
                "/x/web-interface/nav" => {
                    json!({"code":0,"data":{"isLogin":true,"mid":123,"wbi_img":{"img_url":"https://i.example/7cd084941338484aae1ad9425b84077c.png","sub_url":"https://i.example/4932caff0ff746eab6f01bf08b70ac45.png"}}})
                }
                "/x/v3/fav/folder/created/list-all" => {
                    assert_eq!(query["up_mid"], "123");
                    assert_eq!(headers["cookie"], "SESSDATA=fixture-only");
                    json!({"code":0,"data":{"count":1,"list":[{"id":456,"title":"自己的收藏夹","media_count":2}]}})
                }
                "/x/v3/fav/resource/list" => {
                    assert_eq!(query["media_id"], "456");
                    assert_eq!(query["ps"], "20");
                    let first = query["pn"] == "1";
                    json!({"code":0,"data":{"info":{"id":456,"title":"收藏夹","media_count":2,"upper":{"name":"作者"}},"has_more":first,"medias":[
                        {"bvid":if first {"BV1xx411c7mD"} else {"BV1sVtw6JEeq"},"type":2,"title":if first {"多 P"} else {"单 P"},"page":if first {2} else {1}}
                    ]}})
                }
                "/x/web-interface/wbi/view/detail" => {
                    if query["bvid"] == "BV1xx411c7mD" {
                        json!({"code":0,"data":{"View":{"bvid":"BV1xx411c7mD","title":"多 P","owner":{"name":"作者"},"pages":[{"cid":10,"page":1,"part":"第一首"},{"cid":20,"page":2,"part":"第二首"}]}}})
                    } else {
                        json!({"code":0,"data":{"View":{"bvid":"BV1sVtw6JEeq","title":"单 P","pages":[{"cid":40,"page":1,"part":"单曲"}]}}})
                    }
                }
                "/x/player/wbi/playurl" => {
                    assert_eq!(headers["cookie"], "SESSDATA=fixture-only");
                    json!({"code":0,"data":{"dash":{"audio":[{"codecs":"mp4a.40.2","mimeType":"audio/mp4","bandwidth":128000,"baseUrl":format!("https://cdn.bilivideo.com/audio/{}",query["cid"])}]}}})
                }
                path if path.starts_with("/audio/") => {
                    assert!(!headers.contains_key("cookie"));
                    let mut bytes = Vec::new();
                    for (kind, body) in [
                        (b"ftyp", b"M4A     ".as_slice()),
                        (b"moov", b"fixture".as_slice()),
                        (b"mdat", path.as_bytes()),
                    ] {
                        bytes.extend_from_slice(&((body.len() + 8) as u32).to_be_bytes());
                        bytes.extend_from_slice(kind);
                        bytes.extend_from_slice(body);
                    }
                    return ([("content-type", "audio/mp4")], bytes).into_response();
                }
                _ => return axum::http::StatusCode::NOT_FOUND.into_response(),
            };
            axum::Json(response).into_response()
        }
        let directory = tempfile::tempdir().unwrap();
        let engine = Engine::new(directory.path().to_owned()).await.unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, Router::new().fallback(get(serve)))
                .await
                .unwrap();
        });
        let mut downloader = Downloader::new(directory.path()).unwrap();
        let owned = Arc::get_mut(&mut downloader).unwrap();
        owned.client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        owned.test_origin = Some(format!("http://{address}").parse().unwrap());
        assert!(downloader
            .favorite_folders()
            .await
            .unwrap_err()
            .contains("先在下方登录"));
        let first = downloader
            .favorite_page("https://space.bilibili.com/123/favlist?fid=456", 1)
            .await
            .unwrap();
        assert!(first.has_more);
        let second = downloader.favorite_page(&first.id, 2).await.unwrap();
        assert!(!second.has_more);
        assert_eq!(second.page, 2);
        downloader
            .set_session("SESSDATA=fixture-only; ignored=secret")
            .await
            .unwrap();
        assert_eq!(downloader.favorite_folders().await.unwrap()[0].id, "456");
        let multi = downloader.parse(&first.videos[0].bvid).await.unwrap();
        let single = downloader.parse(&second.videos[0].bvid).await.unwrap();
        let result = downloader
            .start_batch(
                Arc::downgrade(&engine),
                vec![
                    BatchSelection {
                        preview_id: multi.id.clone(),
                        cids: vec![20],
                    },
                    BatchSelection {
                        preview_id: single.id,
                        cids: vec![40],
                    },
                ],
                engine.cancellation(),
            )
            .await
            .unwrap();
        assert_eq!(
            result
                .items
                .iter()
                .filter(|item| item.job.is_some())
                .count(),
            2
        );
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let jobs = downloader.state().await.jobs;
                if jobs.iter().all(|job| !active_phase(&job.phase)) {
                    assert!(jobs.iter().all(|job| job.phase == "completed"), "{jobs:?}");
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let library = crate::library::Library::load(directory.path().to_owned()).unwrap();
        assert_eq!(library.tracks().len(), 2);
        let track = library
            .tracks()
            .into_iter()
            .find(|track| track.title == "第二首")
            .unwrap();
        assert_eq!(track.source.as_ref().unwrap().id, "BV1xx411c7mD:20");
        assert!(track.source.as_ref().unwrap().url.ends_with("?p=2"));
        let duplicate = downloader
            .start_batch(
                Arc::downgrade(&engine),
                vec![BatchSelection {
                    preview_id: multi.id,
                    cids: vec![20],
                }],
                engine.cancellation(),
            )
            .await
            .unwrap();
        assert!(duplicate.items[0]
            .error
            .as_ref()
            .unwrap()
            .contains("已在曲库"));
        assert_eq!(downloader.state().await.jobs.len(), 2);
        downloader.disconnect().await;
        assert!(downloader.favorite_folders().await.is_err());
        engine.shutdown();
        server.abort();
    }
}
