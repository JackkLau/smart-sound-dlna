use super::*;
use serde_json::json;

fn item() -> Item {
    serde_json::from_value(json!({
        "metadata": {"identifier":"test", "title":"The Test", "date":"2014-06-27",
                     "addeddate":"2026-01-01","mediatype":"movies","licenseurl":"https://creativecommons.org/licenses/by-nc-sa/4.0/"},
        "files":[{"name":"test_archive.torrent"}]
    })).unwrap()
}

#[test]
fn requires_movie_identity_public_license_and_actual_torrent() {
    let mut doc = item();
    let names = ["The Test", "测试"];
    assert_eq!(
        inspect(&doc, &names, 2014).unwrap().license,
        "https://creativecommons.org/licenses/by-nc-sa/4.0/"
    );
    assert!(
        inspect(&doc, &names, 2026).is_none(),
        "upload date must not count as release year"
    );
    for title in [
        "The Test II",
        "The Test (Trailer)",
        "The Test: Behind the Scenes",
    ] {
        doc.metadata["title"] = json!(title);
        assert!(inspect(&doc, &names, 2014).is_none());
    }
    doc.metadata["title"] = json!("The Test (2014)");
    doc.metadata["licenseurl"] = json!(["https://creativecommons.org/licenses/by/3.0/"]);
    assert!(inspect(&doc, &names, 2014).is_some());
    doc.files[0].private = json!("true");
    assert!(inspect(&doc, &names, 2014).is_none());
    doc.files.clear();
    assert!(inspect(&doc, &names, 2014).is_none());
    assert!(license(&json!(
        "https://creativecommons.org.example/licenses/by/4.0/"
    ))
    .is_none());
    assert!(license(&json!("https://creativecommons.org/licenses/unspecified/")).is_none());
}

#[test]
fn distinguishes_unknown_release_year_and_rejects_nonitems() {
    let mut doc = item();
    doc.metadata["date"] = json!("unspecified");
    assert!(inspect(&doc, &["The Test"], 2014)
        .unwrap()
        .reason
        .contains("缺少作品年份"));
    for value in [json!("true"), json!(true), json!("1")] {
        doc.metadata["access-restricted-item"] = value;
        assert!(inspect(&doc, &["The Test"], 2014).is_none());
    }
    assert!(!valid_id("../test"));
    assert!(archive_host(
        &Url::parse("https://ia12.us.archive.org/x").unwrap()
    ));
    assert!(!archive_host(&Url::parse("https://example.org/x").unwrap()));
}

fn torrent(name: &[u8]) -> Vec<u8> {
    let mut bytes = b"d4:infod6:lengthi1e4:name8:".to_vec();
    bytes.extend_from_slice(name);
    bytes.extend_from_slice(b"12:piece lengthi16384e6:pieces20:");
    bytes.extend_from_slice(&[0; 20]);
    bytes.extend_from_slice(b"ee");
    bytes
}

#[test]
fn hashes_raw_info_and_requires_supported_media() {
    let bytes = torrent(b"film.mp4");
    let uri = Url::parse(&torrent_magnet(&bytes, "电影").unwrap()).unwrap();
    let xt = uri.query_pairs().find(|(k, _)| k == "xt").unwrap().1;
    assert_eq!(xt, "urn:btih:7c4b0d063fd1a41d8cd04865aa424c9a19d5fa33");
    assert!(torrent_magnet(&torrent(b"film.txt"), "电影").is_none());
    assert!(torrent_magnet(b"d4:infodee", "missing v1 information").is_none());
}

#[test]
fn honors_retry_after_seconds_and_http_date() {
    let t = SystemTime::UNIX_EPOCH + Duration::from_secs(2);
    assert_eq!(backoff(429, Some("90"), t), Some(Duration::from_secs(90)));
    assert_eq!(
        backoff(503, Some("Thu, 01 Jan 1970 00:01:00 GMT"), t),
        Some(Duration::from_secs(58))
    );
    assert_eq!(backoff(429, None, t), Some(Duration::from_secs(60)));
    assert_eq!(backoff(404, Some("90"), t), None);
}

#[test]
fn uses_published_torrent_filename_from_the_item() {
    let mut doc = item();
    doc.files[0].name = "The Test.torrent".to_string();
    assert_eq!(
        inspect(&doc, &["The Test"], 2014).unwrap().torrent,
        "The Test.torrent"
    );
    doc.files[0].name = "../The Test.torrent".to_string();
    assert!(inspect(&doc, &["The Test"], 2014).is_none());
}
