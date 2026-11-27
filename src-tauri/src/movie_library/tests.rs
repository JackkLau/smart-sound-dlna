use super::*;
use serde_json::{json, Value};

fn snapshot(id: &str, year: i32, rating: i32, observed: &str) -> Value {
    json!({"id": id, "title": "测试影库条目", "original": "Example film",
        "year": year, "rating_tenths": rating, "observed": observed, "kind": "movie"})
}

#[test]
fn boundaries_including_current_year_type_and_short_title() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("片库.sqlite3")).unwrap();
    let db = Connection::open(&store.path).unwrap();
    let mut rows = db
        .query("SELECT strftime('%Y','now','localtime')", &[])
        .unwrap();
    rows.next().unwrap();
    let current = rows.integer(0);
    let mut series = snapshot("711006", 2008, 99, "2021-03-01");
    series["kind"] = "tv".into();
    let records = vec![
        snapshot("711001", 1989, 98, "2021-03-01"),
        snapshot("711002", 1990, 80, "2021-03-01"),
        snapshot("711003", 2001, 79, "2021-03-01"),
        snapshot("711004", current, 81, "2021-03-01"),
        snapshot("711005", current + 1, 89, "2021-03-01"),
        series,
    ];
    assert_eq!(store.import(&json!(records).to_string()).unwrap(), 2);
    assert_eq!(store.list("影库").unwrap().len(), 2);
    assert_eq!(store.list("EXAMPLE").unwrap().len(), 2);
    assert_eq!(store.list("711002").unwrap()[0].year, 1990);
    assert!(store.list("1990").unwrap().iter().any(|m| m.id == "711002"));
    assert!(store.get("711001").is_err());
    assert!(store.get("711003").is_err());
    assert!(store.get("711005").is_err());
    assert!(store.get("711006").is_err());
}

#[test]
fn refresh_retracts_then_restores_without_losing_links_or_overwriting_newer_data() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("library.db");
    let store = Store::open(&path).unwrap();
    let id = "711003";
    let original = json!([snapshot(id, 1990, 80, "2020-05-01")]).to_string();
    store.import(&original).unwrap();
    let link_hex = format!("magnet:?xt=urn:btih:{}&dn=example", "0".repeat(40));
    let link_base32 = format!("magnet:?xt=urn:btih:{}", "A".repeat(32));
    store.save_link(id, &link_hex, "粘贴的链接").unwrap();
    store.save_link(id, &link_base32, "他处的来源").unwrap();
    store.save_link(id, &link_hex, "粘贴的链接").unwrap();
    assert!(store.save_link(id, "invalid", "手工").is_err());
    assert_eq!(store.get(id).unwrap().resource_count, 1);
    assert_eq!(store.links(id).unwrap()[0].sources.len(), 2);
    let db = Connection::open(&path).unwrap();
    let mut versions = db.query("SELECT count(*), min(douban_rating=8.0 AND magnet_uri IS NOT NULL) FROM film_catalog WHERE douban_id=?1", &[id]).unwrap();
    versions.next().unwrap();
    assert_eq!((versions.integer(0), versions.integer(1)), (2, 1));
    drop(versions);
    let empty = json!([snapshot("711004", 1990, 81, "2020-05-01")]).to_string();
    store.import(&empty).unwrap();
    let mut no_resource = db.query("SELECT count(*) FROM film_catalog WHERE douban_id='711004' AND douban_rating=8.1 AND magnet_uri IS NULL AND source_url IS NULL", &[]).unwrap();
    no_resource.next().unwrap();
    assert_eq!(no_resource.integer(0), 1);
    drop(no_resource);

    let license = "https://creativecommons.org/publicdomain/zero/1.0/";
    store
        .save_link_with_license(id, &link_hex, "粘贴的链接", license)
        .unwrap();
    store.save_link(id, &link_hex, "粘贴的链接").unwrap();
    assert_eq!(
        store.links(id).unwrap()[0].license_url.as_deref(),
        Some(license)
    );
    store
        .import(&json!([snapshot(id, 1990, 79, "2020-05-02")]).to_string())
        .unwrap();
    assert!(store.get(id).is_err());
    assert!(store.links(id).unwrap().is_empty());
    assert_eq!(store.import(&original).unwrap(), 0);
    assert!(store.get(id).is_err());
    drop(store);
    let restored = Store::open(&path).unwrap();
    restored
        .import(&json!([snapshot(id, 1990, 82, "2020-05-03")]).to_string())
        .unwrap();
    assert_eq!(restored.get(id).unwrap().resource_count, 1);
    assert_eq!(restored.links(id).unwrap()[0].sources.len(), 2);
    assert_eq!(restored.import(&original).unwrap(), 0);
    assert_eq!(restored.get(id).unwrap().rating_tenths, 82);
}

#[test]
fn rolls_back_invalid_batch_and_preserves_previous_records() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("catalog.sqlite")).unwrap();
    let records = vec![
        snapshot("711001", 1990, 81, "2020-01-01"),
        snapshot("711002", 1990, 82, "2020-02-31"),
    ];
    assert!(store.import(&json!(records).to_string()).is_err());
    assert!(store.get("711001").is_err());
    assert!(!store.list("").unwrap().is_empty());
}

#[test]
fn same_day_refresh_overrides_bundled_observation_and_survives_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("library.sqlite3");
    let store = Store::open(&path).unwrap();
    let mut film = store.list("").unwrap().remove(0);
    let id = film.id.clone();
    film.rating_tenths = 79;
    assert_eq!(
        store
            .import(&serde_json::to_string(&[film]).unwrap())
            .unwrap(),
        0
    );
    assert!(store.get(&id).is_err());
    assert!(Store::open(&path).unwrap().get(&id).is_err());
}

#[test]
fn removing_one_association_survives_seed_refresh_and_preserves_other_movies() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("library.sqlite");
    let store = Store::open(&db_path).unwrap();
    let open_films: Vec<OpenFilm> = serde_json::from_str(OPEN_FILMS).unwrap();
    let open = open_films.first().expect("need a verified seed film");
    let id = &open.seed.movie.id;
    assert_eq!(
        store.get(id).unwrap().rating_evidence,
        open.seed.rating_evidence
    );
    let other = "711009";
    store
        .import(&json!([snapshot(other, 2008, 83, "2021-05-01")]).to_string())
        .unwrap();
    store
        .save_link(other, &open.uri, "another publisher")
        .unwrap();
    assert_eq!(
        store.links(id).unwrap()[0].license_url.as_deref(),
        Some(open.license_url.as_str())
    );
    store.remove_link(id, &open.uri).unwrap();
    store.remove_link(id, &open.uri).unwrap();
    let reopened = Store::open(&db_path).unwrap();
    assert!(reopened.links(id).unwrap().is_empty());
    assert_eq!(reopened.get(other).unwrap().resource_count, 1);
    reopened.save_link(id, &open.uri, &open.source).unwrap();
    assert_eq!(reopened.get(id).unwrap().resource_count, 1);
    let mut updated = reopened.get(id).unwrap();
    updated.rating_tenths = if updated.rating_tenths == 80 { 81 } else { 80 };
    reopened
        .import(&serde_json::to_string(&[updated]).unwrap())
        .unwrap();
    assert!(reopened.get(id).unwrap().rating_evidence.is_none());
}

#[test]
fn existing_unrelated_sqlite_file_is_left_intact() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("other.sqlite");
    let other = Connection::open(&path).unwrap();
    other
        .batch("CREATE TABLE other(value TEXT); INSERT INTO other VALUES('first')")
        .unwrap();
    assert!(Store::open(&path).is_err());
    let mut rows = other
        .query("SELECT name FROM sqlite_master WHERE type='table'", &[])
        .unwrap();
    assert!(rows.next().unwrap());
    assert_eq!(rows.text(0).unwrap(), "other");
    assert!(!rows.next().unwrap());
}

#[test]
fn only_bundled_snapshot_may_supply_rating_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("catalog.sqlite3");
    let store = Store::open(&path).unwrap();
    let mut record = snapshot("711010", 2000, 87, "2020-01-01");
    let fixed = "https://example.org/pinned-version/workbook";
    record["rating_evidence"] = json!(fixed);
    let seed: SeedMovie = serde_json::from_value(record.clone()).unwrap();
    let db = Connection::open(&path).unwrap();
    db.transaction(|| seed.save(&db)).unwrap();
    assert_eq!(
        store.get("711010").unwrap().rating_evidence.as_deref(),
        Some(fixed)
    );
    record["rating_evidence"] = json!("https://example.org/untrusted");
    store.import(&json!([record.clone()]).to_string()).unwrap();
    assert_eq!(
        store.get("711010").unwrap().rating_evidence.as_deref(),
        Some(fixed)
    );
    record["observed"] = json!("2020-01-02");
    record["rating_tenths"] = json!(86);
    store.import(&json!([record]).to_string()).unwrap();
    assert!(store.get("711010").unwrap().rating_evidence.is_none());
    assert!(!db
        .query(
            "SELECT 1 FROM movie_rating_evidence WHERE url='https://example.org/untrusted'",
            &[]
        )
        .unwrap()
        .next()
        .unwrap());
}

fn bundled_template() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("data/movie-library.sqlite3")
}

#[test]
fn packaged_database_is_copied_once_and_user_changes_survive_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("用户数据/片库.sqlite3");
    let template = bundled_template();
    let original = fs::read(&template).unwrap();
    let store = Store::open_packaged(&path, &template).unwrap();
    assert_eq!(fs::read(&path).unwrap(), original);
    assert!(!store.list("").unwrap().is_empty());
    let open_films: Vec<OpenFilm> = serde_json::from_str(OPEN_FILMS).unwrap();
    let film = &open_films[0];
    let id = &film.seed.movie.id;
    assert!(!store.links(id).unwrap().is_empty());
    store.remove_link(id, &film.uri).unwrap();
    let imported = snapshot("711100", 2008, 85, "2020-01-01");
    store.import(&json!([imported]).to_string()).unwrap();

    let reopened = Store::open_packaged(&path, &dir.path().join("missing.sqlite3")).unwrap();
    assert_eq!(reopened.get("711100").unwrap().rating_tenths, 85);
    assert!(reopened.links(id).unwrap().is_empty());
    assert_eq!(fs::read(&template).unwrap(), original);
}

#[test]
fn missing_or_corrupt_bundles_and_existing_corrupt_databases_are_handled_safely() {
    let dir = tempfile::tempdir().unwrap();
    let parent = dir.path().join("user");
    let destination = parent.join("library.sqlite3");
    let template = dir.path().join("template.sqlite3");
    assert!(Store::open_packaged(&destination, &template).is_err());
    fs::write(&template, b"damaged").unwrap();
    assert!(Store::open_packaged(&destination, &template).is_err());
    fs::remove_file(&template).unwrap();
    Connection::open(&template)
        .unwrap()
        .batch("PRAGMA user_version=4")
        .unwrap();
    assert!(Store::open_packaged(&destination, &template).is_err());
    assert_eq!(fs::read_dir(&parent).unwrap().count(), 0);

    fs::write(&destination, b"existing corrupt data").unwrap();
    assert!(Store::open_packaged(&destination, &bundled_template()).is_err());
    assert_eq!(fs::read(destination).unwrap(), b"existing corrupt data");
}

#[test]
fn simultaneous_first_launches_keep_all_writes() {
    use std::{sync::Barrier, thread};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("library.sqlite3");
    let barrier = Barrier::new(3);
    thread::scope(|scope| {
        for id in ["711120", "711121", "711122"] {
            let (path, barrier) = (&path, &barrier);
            scope.spawn(move || {
                barrier.wait();
                let store = Store::open_packaged(path, &bundled_template()).unwrap();
                let record = snapshot(id, 2010, 80, "2020-01-01");
                store.import(&json!([record]).to_string()).unwrap();
            });
        }
    });
    let store = Store::open_packaged(&path, &bundled_template()).unwrap();
    for id in ["711120", "711121", "711122"] {
        assert!(store.get(id).is_ok());
    }
}
