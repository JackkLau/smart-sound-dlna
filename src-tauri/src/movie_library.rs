//! A local catalog of movies with an attributable Douban rating snapshot.
pub(crate) mod sqlite;

use crate::magnet::parse_magnet;
use serde::{Deserialize, Serialize};
use sqlite::{Connection, Rows};
use std::{
    fs,
    path::{Path, PathBuf},
};

// One row per film/version/publication source, and one NULL-version row for unmatched films.
const CATALOG_VIEW: &str = "
CREATE VIEW film_catalog AS
    SELECT m.id AS douban_id,m.title,m.original AS original_title,m.year,
           m.rating_tenths/10.0 AS douban_rating,m.observed AS rating_observed_on,
           nullif(m.rating_evidence,'') AS rating_evidence_url,
           'https://movie.douban.com/subject/' || m.id || '/' AS douban_url,
           m.resource_count,r.btih,r.uri AS magnet_uri,s.source,
           CASE WHEN s.source LIKE 'https://%' OR s.source LIKE 'http://%' THEN s.source END AS source_url,
           nullif(s.license_url,'') AS license_url
    FROM movie_details m
    LEFT JOIN movie_resources mr ON mr.movie_id=m.id
    LEFT JOIN resources r ON r.btih=mr.btih
    LEFT JOIN resource_sources s ON s.btih=r.btih;
PRAGMA user_version=4;";
const RATING_EVIDENCE: &str = "
CREATE TABLE movie_rating_evidence (
    movie_id TEXT NOT NULL REFERENCES douban_snapshots(id), observed TEXT NOT NULL,
    rating_tenths INTEGER NOT NULL, url TEXT NOT NULL, PRIMARY KEY(movie_id,observed,rating_tenths)
);
CREATE VIEW movie_details AS SELECT m.*,coalesce(e.url,'') AS rating_evidence FROM movies m
    LEFT JOIN movie_rating_evidence e ON e.movie_id=m.id AND e.observed=m.observed AND e.rating_tenths=m.rating_tenths;
PRAGMA user_version=3;";
const REMOVED_LINKS: &str = "
CREATE TABLE removed_movie_resources (
    movie_id TEXT NOT NULL REFERENCES douban_snapshots(id), btih TEXT NOT NULL REFERENCES resources(btih),
    PRIMARY KEY(movie_id,btih)
);
PRAGMA user_version=2;";
const SEED: &str = include_str!("../data/movie-seed.json");
const OPEN_FILMS: &str = include_str!("../data/open-film-resources.json");
const SCHEMA_VERSION: i32 = 5;
// Windows' system SQLite can omit the localtime modifier. The UTC year remains available.
const MOVIE_YEAR_VIEW: &str = "
DROP VIEW IF EXISTS movies;
CREATE VIEW movies AS
    SELECT d.*, (SELECT count(*) FROM movie_resources r WHERE r.movie_id=d.id) AS resource_count
    FROM douban_snapshots d
    WHERE kind='movie' AND rating_tenths BETWEEN 80 AND 100
      AND year BETWEEN 1990 AND CAST(coalesce(strftime('%Y','now','localtime'),substr(CURRENT_DATE,1,4)) AS INTEGER);
PRAGMA user_version=5;";
const SCHEMA: &str = "
CREATE TABLE douban_snapshots (
    id TEXT PRIMARY KEY, title TEXT NOT NULL, original TEXT NOT NULL,
    year INTEGER NOT NULL, rating_tenths INTEGER NOT NULL, observed TEXT NOT NULL, kind TEXT NOT NULL
);
CREATE INDEX douban_rating ON douban_snapshots(kind,rating_tenths,year);
CREATE TABLE resources (btih TEXT PRIMARY KEY, uri TEXT NOT NULL);
CREATE TABLE resource_sources (
    btih TEXT NOT NULL REFERENCES resources(btih), source TEXT NOT NULL,
    license_url TEXT NOT NULL DEFAULT '', PRIMARY KEY(btih,source)
);
CREATE TABLE movie_resources (
    movie_id TEXT NOT NULL REFERENCES douban_snapshots(id), btih TEXT NOT NULL REFERENCES resources(btih),
    PRIMARY KEY(movie_id,btih)
);
PRAGMA user_version=1;
";

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Movie {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub original: String,
    pub year: i32,
    pub rating_tenths: i32,
    pub observed: String,
    pub kind: String,
    #[serde(default, skip_deserializing)]
    pub resource_count: usize,
    #[serde(default, skip_deserializing)]
    pub rating_evidence: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Link {
    pub uri: String,
    pub source: String,
    pub sources: Vec<String>,
    pub license_url: Option<String>,
}

// Only data bundled with the application is deserialized through this type.
// External imports accept movie observations, but cannot install clickable evidence URLs.
#[derive(Deserialize)]
struct SeedMovie {
    #[serde(flatten)]
    movie: Movie,
    #[serde(default)]
    rating_evidence: Option<String>,
}

impl SeedMovie {
    fn save(&self, db: &Connection) -> Result<(), String> {
        let m = &self.movie;
        import_movies(db, std::slice::from_ref(m), false)?;
        if let Some(url) = &self.rating_evidence {
            db.execute("INSERT OR IGNORE INTO movie_rating_evidence(movie_id,observed,rating_tenths,url) VALUES(?1,?2,?3,?4)",
                &[&m.id, &m.observed, &m.rating_tenths.to_string(), url])?;
        }
        Ok(())
    }
}

#[derive(Deserialize)]
struct OpenFilm {
    #[serde(flatten)]
    seed: SeedMovie,
    uri: String,
    source: String,
    license_url: String,
}

#[derive(Clone)]
pub struct Store {
    path: PathBuf,
}

fn validate_template(path: &Path) -> Result<(), String> {
    let db = Connection::open(path)?;
    let mut version = db.query("PRAGMA user_version", &[])?;
    if !version.next()? || version.integer(0) != SCHEMA_VERSION {
        return Err("不支持的初始片库版本".into());
    }
    let mut check = db.query("PRAGMA quick_check", &[])?;
    if !check.next()? || check.text(0)? != "ok" {
        return Err("SQLite 完整性检查失败".into());
    }
    if db.query("PRAGMA foreign_key_check", &[])?.next()? {
        return Err("片库关联数据不完整".into());
    }
    if !db
        .query(
            "SELECT douban_id, magnet_uri FROM film_catalog LIMIT 1",
            &[],
        )?
        .next()?
    {
        return Err("未找到预置电影记录".into());
    }
    Ok(())
}

impl Store {
    /// Install a private, writable copy of the distributed catalog on first use.
    /// Later launches migrate and merge seeds into the existing user database.
    pub fn open_packaged(path: &Path, template: &Path) -> Result<Self, String> {
        if path
            .try_exists()
            .map_err(|e| format!("无法检查用户片库：{e}"))?
        {
            return Self::open(path);
        }
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        fs::create_dir_all(parent).map_err(|e| format!("无法创建片库目录：{e}"))?;
        let mut source =
            fs::File::open(template).map_err(|e| format!("无法打开安装包内的初始片库：{e}"))?;
        let mut staging = tempfile::NamedTempFile::new_in(parent)
            .map_err(|e| format!("无法创建用户片库：{e}"))?;
        std::io::copy(&mut source, staging.as_file_mut())
            .and_then(|_| staging.as_file().sync_all())
            .map_err(|e| format!("无法复制初始片库：{e}"))?;
        let staging = staging.into_temp_path();
        validate_template(&staging).map_err(|e| format!("安装包内的初始片库无效：{e}"))?;
        match staging.persist_noclobber(path) {
            Ok(()) => Ok(Self {
                path: path.to_owned(),
            }),
            // Another process finished the same first-run copy while we were verifying ours.
            Err(e) if e.error.kind() == std::io::ErrorKind::AlreadyExists => Self::open(path),
            Err(e) => Err(format!("无法保存用户片库：{e}")),
        }
    }

    pub fn open(path: &Path) -> Result<Self, String> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent).map_err(|e| format!("无法创建片库目录：{e}"))?;
        }
        let store = Self {
            path: path.to_owned(),
        };
        let db = Connection::open(path)?;
        db.transaction(|| {
            let version = {
                let mut check = db.query("PRAGMA user_version", &[])?;
                check.next()?;
                check.integer(0)
            };
            match version {
                0 => {
                    if db.query("SELECT 1 FROM sqlite_master WHERE type='table' AND name NOT GLOB 'sqlite_*' LIMIT 1", &[])?.next()? {
                        return Err("所选数据库已包含其他数据，请选择新的片库文件".into());
                    }
                    db.batch(SCHEMA)?;
                }
                1..=SCHEMA_VERSION => {
                    if !db.query("SELECT 1 FROM sqlite_master WHERE type='table' AND name='douban_snapshots'", &[])?.next()? {
                        return Err("所选数据库不是此应用的电影片库".into());
                    }
                }
                _ => return Err(format!("当前程序不支持片库数据库版本 {version}")),
            }
            if version < 2 {
                db.batch(REMOVED_LINKS)?;
            }
            if version < 3 { db.batch(RATING_EVIDENCE)?; }
            if version < 4 { db.batch(CATALOG_VIEW)?; }
            if version < 5 { db.batch(MOVIE_YEAR_VIEW)?; }
            let movies: Vec<SeedMovie> = serde_json::from_str(SEED)
                .map_err(|e| format!("内置片单数据格式错误：{e}"))?;
            let open_films: Vec<OpenFilm> = serde_json::from_str(OPEN_FILMS)
                .map_err(|e| format!("内置资源数据格式错误：{e}"))?;
            for film in movies {
                film.save(&db)?;
            }
            for film in open_films {
                film.seed.save(&db)?;
                let id = &film.seed.movie.id;
                if is_eligible(&db, id)? {
                    save_link(&db, id, &film.uri, &film.source, &film.license_url, false)?;
                }
            }
            Ok(())
        })?;
        Ok(store)
    }

    /// Number of newly inserted/updated records that currently qualify for the catalog.
    pub fn import(&self, json: &str) -> Result<usize, String> {
        let records = decode_movies(json)?;
        let db = Connection::open(&self.path)?;
        db.transaction(|| import_movies(&db, &records, true))
    }

    pub fn list(&self, query: &str) -> Result<Vec<Movie>, String> {
        let db = Connection::open(&self.path)?;
        let mut rows = db.query("SELECT id,title,original,year,rating_tenths,observed,kind,resource_count,rating_evidence FROM movie_details
            WHERE ?1='' OR instr(lower(title),lower(?1))>0 OR instr(lower(original),lower(?1))>0 OR id=?1 OR CAST(year AS TEXT)=?1
            ORDER BY rating_tenths DESC,year DESC,id", &[query.trim()])?;
        let mut films = Vec::new();
        while rows.next()? {
            films.push(movie_from_row(&rows)?);
        }
        Ok(films)
    }

    pub fn get(&self, id: &str) -> Result<Movie, String> {
        let db = Connection::open(&self.path)?;
        let mut rows = db.query("SELECT id,title,original,year,rating_tenths,observed,kind,resource_count,rating_evidence FROM movie_details WHERE id=?1", &[id])?;
        if rows.next()? {
            movie_from_row(&rows)
        } else {
            Err("当前片库中没有这部符合条件的电影".into())
        }
    }

    pub fn save_link(&self, id: &str, uri: &str, source: &str) -> Result<(), String> {
        self.save_link_with_license(id, uri, source, "")
    }

    pub fn save_link_with_license(
        &self,
        id: &str,
        uri: &str,
        source: &str,
        license_url: &str,
    ) -> Result<(), String> {
        let db = Connection::open(&self.path)?;
        db.transaction(|| {
            if !is_eligible(&db, id)? {
                return Err("当前片库中没有这部符合条件的电影".into());
            }
            save_link(&db, id, uri, source, license_url, true)
        })
    }

    pub fn remove_link(&self, id: &str, uri: &str) -> Result<(), String> {
        let (btih, _, _) = parse_magnet(uri)?;
        let db = Connection::open(&self.path)?;
        db.transaction(|| {
            let keys = [id, btih.as_str()];
            db.execute(
                "INSERT OR IGNORE INTO removed_movie_resources(movie_id,btih)
                SELECT movie_id,btih FROM movie_resources WHERE movie_id=?1 AND btih=?2",
                &keys,
            )?;
            db.execute(
                "DELETE FROM movie_resources WHERE movie_id=?1 AND btih=?2",
                &keys,
            )?;
            Ok(())
        })
    }

    pub fn links(&self, id: &str) -> Result<Vec<Link>, String> {
        let db = Connection::open(&self.path)?;
        let mut rows = db.query(
            "SELECT r.btih,r.uri,s.source,s.license_url FROM movies m
            JOIN movie_resources mr ON mr.movie_id=m.id JOIN resources r ON r.btih=mr.btih
            JOIN resource_sources s ON s.btih=r.btih WHERE m.id=?1 ORDER BY r.btih,s.rowid",
            &[id],
        )?;
        let mut links = Vec::<Link>::new();
        let mut last_hash = String::new();
        while rows.next()? {
            let hash = rows.text(0)?;
            let source = rows.text(2)?;
            if hash != last_hash {
                links.push(Link {
                    uri: rows.text(1)?,
                    source: source.clone(),
                    sources: Vec::new(),
                    license_url: None,
                });
                last_hash = hash;
            }
            if let Some(link) = links.last_mut() {
                let license = rows.text(3)?;
                if !license.is_empty() && link.license_url.is_none() {
                    link.license_url = Some(license);
                }
                link.sources.push(source);
            }
        }
        Ok(links)
    }
}

fn movie_from_row(row: &Rows<'_>) -> Result<Movie, String> {
    Ok(Movie {
        id: row.text(0)?,
        title: row.text(1)?,
        original: row.text(2)?,
        year: row.integer(3),
        rating_tenths: row.integer(4),
        observed: row.text(5)?,
        kind: row.text(6)?,
        resource_count: row.integer(7) as usize,
        rating_evidence: Some(row.text(8)?).filter(|url| !url.is_empty()),
    })
}

fn decode_movies(json: &str) -> Result<Vec<Movie>, String> {
    serde_json::from_str(json).map_err(|e| format!("豆瓣片单 JSON 格式错误：{e}"))
}

fn is_eligible(db: &Connection, id: &str) -> Result<bool, String> {
    db.query("SELECT 1 FROM movies WHERE id=?1", &[id])?.next()
}

fn import_movies(db: &Connection, records: &[Movie], refresh_today: bool) -> Result<usize, String> {
    let mut count = 0;
    for film in records {
        if film.id.is_empty()
            || film.id.len() > 20
            || film.id.starts_with('0')
            || !film.id.bytes().all(|b| b.is_ascii_digit())
            || film.title.trim().is_empty()
            || !(1..=9999).contains(&film.year)
            || !(0..=100).contains(&film.rating_tenths)
            || film.kind.trim().is_empty()
        {
            return Err("条目需要数字豆瓣 ID、片名、年份、类型和 0–100 的十分制评分整数".into());
        }
        // Observation dates have no timezone. UTC+14 is the latest calendar day already in use worldwide.
        let mut date = db.query("SELECT 1 WHERE ?1 GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]' AND date(?1,'+0 days')=?1 AND ?1<=date('now','+14 hours')", &[&film.observed])?;
        if !date.next()? {
            return Err(format!("豆瓣条目 {} 的观测日期无效", film.id));
        }
        let year = film.year.to_string();
        let rating = film.rating_tenths.to_string();
        let args = [
            film.id.as_str(),
            film.title.trim(),
            film.original.trim(),
            &year,
            &rating,
            &film.observed,
            &film.kind,
            if refresh_today { "1" } else { "0" },
        ];
        // INSERT+UPDATE works with older inbox SQLite versions and never replaces a referenced row.
        let inserted = db.execute("INSERT OR IGNORE INTO douban_snapshots(id,title,original,year,rating_tenths,observed,kind) VALUES (?1,?2,?3,?4,?5,?6,?7)", &args[..7])?;
        let changed = inserted != 0 || db.execute("UPDATE douban_snapshots SET title=?2,original=coalesce(nullif(?3,''),original),year=?4,rating_tenths=?5,observed=?6,kind=?7 WHERE id=?1 AND (observed<?6 OR
            (observed=?6 AND ?8='1' AND (title<>?2 OR (?3<>'' AND original<>?3) OR year<>?4 OR rating_tenths<>?5 OR kind<>?7)))", &args)? != 0;
        if changed && is_eligible(db, &film.id)? {
            count += 1;
        }
    }
    Ok(count)
}

fn save_link(
    db: &Connection,
    id: &str,
    uri: &str,
    source: &str,
    license_url: &str,
    confirmed: bool,
) -> Result<(), String> {
    let source = source.trim();
    if source.is_empty() || source.len() > 4096 || source.chars().any(char::is_control) {
        return Err("请提供有效的资源来源".into());
    }
    let (hash, _, canonical) = parse_magnet(uri)?;
    if confirmed {
        db.execute(
            "DELETE FROM removed_movie_resources WHERE movie_id=?1 AND btih=?2",
            &[id, &hash],
        )?;
    } else if db
        .query(
            "SELECT 1 FROM removed_movie_resources WHERE movie_id=?1 AND btih=?2",
            &[id, &hash],
        )?
        .next()?
    {
        return Ok(());
    }
    db.execute(
        "INSERT OR IGNORE INTO resources(btih,uri) VALUES(?1,?2)",
        &[&hash, &canonical],
    )?;
    db.execute(
        "INSERT OR IGNORE INTO resource_sources(btih,source,license_url) VALUES(?1,?2,?3)",
        &[&hash, source, license_url],
    )?;
    if !license_url.is_empty() {
        db.execute("UPDATE resource_sources SET license_url=?3 WHERE btih=?1 AND source=?2 AND license_url=''", &[&hash,source,license_url])?;
    }
    db.execute(
        "INSERT OR IGNORE INTO movie_resources(movie_id,btih) VALUES(?1,?2)",
        &[id, &hash],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests;
