//! On-demand catalog discovery. A lease spans the HTTP request, never a SQLite transaction.
use crate::{
    magnet::parse_magnet,
    movie_discovery::{self, Candidate},
    movie_library::{sqlite::Connection, Store},
};
use serde::Serialize;
use std::{path::PathBuf, time::Duration};
use uuid::Uuid;

const TABLES: &str = "
CREATE TABLE IF NOT EXISTS discovery_jobs (
    movie_id TEXT PRIMARY KEY REFERENCES douban_snapshots(id),
    state TEXT NOT NULL DEFAULT 'pending',
    attempts INTEGER NOT NULL DEFAULT 0,
    token TEXT NOT NULL DEFAULT '',
    next_at INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS discovery_candidates (
    movie_id TEXT NOT NULL REFERENCES douban_snapshots(id),
    btih TEXT NOT NULL, title TEXT NOT NULL, uri TEXT NOT NULL, source TEXT NOT NULL, reason TEXT NOT NULL,
    dismissed INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY(movie_id,btih)
);
CREATE TABLE IF NOT EXISTS discovery_licenses (
    movie_id TEXT NOT NULL, btih TEXT NOT NULL, source TEXT NOT NULL, license_url TEXT NOT NULL,
    PRIMARY KEY(movie_id,btih,source),
    FOREIGN KEY(movie_id,btih) REFERENCES discovery_candidates(movie_id,btih)
);
CREATE TABLE IF NOT EXISTS discovery_pace (
    id INTEGER PRIMARY KEY CHECK(id=1), next_at INTEGER NOT NULL, notice TEXT NOT NULL
);
INSERT OR IGNORE INTO discovery_pace VALUES(1,0,'');
CREATE VIEW IF NOT EXISTS visible_discovery_candidates AS
    SELECT c.* FROM discovery_candidates c JOIN movies m ON m.id=c.movie_id
    WHERE c.dismissed=0 AND NOT EXISTS(
        SELECT 1 FROM movie_resources r WHERE r.movie_id=c.movie_id AND r.btih=c.btih
    ) AND NOT EXISTS(
        SELECT 1 FROM removed_movie_resources r WHERE r.movie_id=c.movie_id AND r.btih=c.btih
    );";

pub struct Queue {
    path: PathBuf,
}

#[derive(Serialize)]
pub struct BatchStatus {
    pub processed: usize,
    pub total: usize,
    pub pending_review: usize,
    pub failed: usize,
    pub done: bool,
    pub message: String,
}

enum Slot {
    Movie { id: String, token: String },
    Pace(u64),
    Unavailable(String),
}

impl Queue {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    fn open(&self) -> Result<Connection, String> {
        let db = Connection::open(&self.path)?;
        db.batch(TABLES)?;
        Ok(db)
    }

    pub fn status(&self) -> Result<BatchStatus, String> {
        let db = self.open()?;
        let mut rows = db.query(
            "SELECT count(*),
             coalesce(sum(j.state IN ('complete','failed')),0),
             coalesce(sum(j.state IN ('retry','failed')),0),
             (SELECT count(*) FROM visible_discovery_candidates),
             (SELECT notice FROM discovery_pace WHERE id=1)
             FROM movies m LEFT JOIN discovery_jobs j ON j.movie_id=m.id",
            &[],
        )?;
        rows.next()?;
        let total = rows.integer(0) as usize;
        let processed = rows.integer(1) as usize;
        Ok(BatchStatus {
            total,
            processed,
            failed: rows.integer(2) as usize,
            pending_review: rows.integer(3) as usize,
            done: total == processed,
            message: rows.text(4)?,
        })
    }

    pub fn candidates(&self, id: &str) -> Result<Vec<Candidate>, String> {
        let db = self.open()?;
        let mut rows = db.query(
            "SELECT c.title,c.uri,c.source,c.reason,coalesce(l.license_url,'')
             FROM visible_discovery_candidates c LEFT JOIN discovery_licenses l
               ON l.movie_id=c.movie_id AND l.btih=c.btih AND l.source=c.source
             WHERE c.movie_id=?1 ORDER BY c.title,c.btih",
            &[id],
        )?;
        let mut results = Vec::new();
        while rows.next()? {
            results.push(Candidate {
                title: rows.text(0)?,
                uri: rows.text(1)?,
                source: rows.text(2)?,
                reason: rows.text(3)?,
                license_url: rows.text(4)?,
            });
        }
        Ok(results)
    }

    pub fn candidate_license(
        &self,
        id: &str,
        uri: &str,
        source: &str,
    ) -> Result<Option<String>, String> {
        let (hash, _, _) = parse_magnet(uri)?;
        let db = self.open()?;
        let mut rows = db.query(
            "SELECT license_url FROM discovery_licenses
            WHERE movie_id=?1 AND btih=?2 AND source=?3",
            &[id, &hash, source],
        )?;
        if rows.next()? {
            Ok(Some(rows.text(0)?))
        } else {
            Ok(None)
        }
    }

    pub fn review_ids(&self) -> Result<Vec<String>, String> {
        let db = self.open()?;
        let mut rows = db.query(
            "SELECT DISTINCT movie_id FROM visible_discovery_candidates ORDER BY movie_id",
            &[],
        )?;
        let mut ids = Vec::new();
        while rows.next()? {
            ids.push(rows.text(0)?);
        }
        Ok(ids)
    }

    pub fn dismiss(&self, id: &str, uri: &str) -> Result<(), String> {
        let (hash, _, _) = parse_magnet(uri)?;
        self.open()?.execute(
            "UPDATE discovery_candidates SET dismissed=1 WHERE movie_id=?1 AND btih=?2",
            &[id, &hash],
        )?;
        Ok(())
    }

    pub fn remember(&self, id: &str, found: &[Candidate]) -> Result<(), String> {
        let db = self.open()?;
        db.transaction(|| Self::record_candidates(&db, id, found))
    }

    fn record_candidates(db: &Connection, id: &str, found: &[Candidate]) -> Result<(), String> {
        for candidate in found {
            let (btih, _, uri) = parse_magnet(&candidate.uri)?;
            // Also retains an earlier manual dismissal when a source rediscovers the same hash.
            db.execute(
                "INSERT OR IGNORE INTO discovery_candidates(movie_id,btih,title,uri,source,reason)
                VALUES(?1,?2,?3,?4,?5,?6)",
                &[
                    id,
                    &btih,
                    &candidate.title,
                    &uri,
                    &candidate.source,
                    &candidate.reason,
                ],
            )?;
            if !candidate.license_url.is_empty() {
                db.execute(
                    "INSERT OR REPLACE INTO discovery_licenses(movie_id,btih,source,license_url)
                     VALUES(?1,?2,?3,?4)",
                    &[id, &btih, &candidate.source, &candidate.license_url],
                )?;
            } else {
                db.execute(
                    "DELETE FROM discovery_licenses WHERE movie_id=?1 AND btih=?2 AND source=?3",
                    &[id, &btih, &candidate.source],
                )?;
            }
        }
        Ok(())
    }

    fn claim(&self) -> Result<Slot, String> {
        let db = self.open()?;
        db.transaction(|| {
            // If the previous run exhausted its retry budget, a new explicit advance starts
            // another pass over only those failures; successful titles keep their checkpoint.
            if !db.query("SELECT 1 FROM movies m LEFT JOIN discovery_jobs j ON j.movie_id=m.id
                         WHERE j.state IS NULL OR j.state NOT IN ('complete','failed') LIMIT 1", &[])?.next()? {
                let reset = db.execute("UPDATE discovery_jobs SET state='pending', attempts=0,next_at=0
                            WHERE state='failed' AND movie_id IN (SELECT id FROM movies)", &[])?;
                if reset == 0 {
                    return Ok(Slot::Unavailable("当前片库已完成检索。".into()));
                }
            }
            if db.query("SELECT 1 FROM discovery_jobs j JOIN movies m ON m.id=j.movie_id WHERE j.state='running' AND
                         next_at>CAST(strftime('%s','now') AS INTEGER) LIMIT 1",&[])?.next()? {
                return Ok(Slot::Unavailable("已有影片正在检索；完成后可继续。".into()));
            }
            let mut pace = db.query("SELECT next_at-strftime('%s','now') FROM discovery_pace WHERE id=1",&[])?;
            pace.next()?;
            let delay = pace.integer(0);
            if delay > 0 {
                return Ok(Slot::Pace(delay as u64));
            }
            let mut next = db.query("SELECT m.id FROM movies m LEFT JOIN discovery_jobs j ON m.id=j.movie_id
                WHERE j.movie_id IS NULL OR (j.state IN ('pending','retry','running')
                    AND j.next_at<=CAST(strftime('%s','now') AS INTEGER))
                ORDER BY m.rating_tenths DESC,m.id LIMIT 1", &[])?;
            if !next.next()? {
                return Ok(Slot::Unavailable("当前没有可立即检索的影片。".into()));
            }
            let id = next.text(0)?;
            let token = Uuid::new_v4().to_string();
            db.execute("INSERT OR IGNORE INTO discovery_jobs(movie_id) VALUES(?1)", &[&id])?;
            db.execute("UPDATE discovery_jobs SET state='running',token=?2, attempts=attempts+1,
                        next_at=strftime('%s','now')+300 WHERE movie_id=?1", &[&id,&token])?;
            db.execute("UPDATE discovery_pace SET next_at=strftime('%s','now')+2 WHERE id=1", &[])?;
            Ok(Slot::Movie { id, token })
        })
    }

    fn finish(
        &self,
        id: &str,
        token: &str,
        title: &str,
        result: Result<Vec<Candidate>, String>,
    ) -> Result<(), String> {
        let db = self.open()?;
        db.transaction(|| {
            let mut owned = db.query("SELECT attempts FROM discovery_jobs WHERE movie_id=?1 AND token=?2 AND state='running'", &[id,token])?;
            if !owned.next()? { return Ok(()); }
            let attempts = owned.integer(0);
            let notice = match result {
                Ok(found) => {
                    Self::record_candidates(&db, id, &found)?;
                    db.execute("UPDATE discovery_jobs SET state='complete',token='',next_at=0 WHERE movie_id=?1",&[id])?;
                    if found.is_empty() { format!("《{title}》暂无匹配资源。") }
                    else { format!("《{title}》检索到 {} 个候选版本。",found.len()) }
                }
                Err(error) => {
                    let delay = movie_discovery::retry_after_seconds().max(if attempts == 1 { 30 } else { 120 });
                    let state = if attempts < 3 { "retry" } else { "failed" };
                    db.execute("UPDATE discovery_jobs SET state=?2,token='',next_at=strftime('%s','now')+?3 WHERE movie_id=?1",
                               &[id,state,&delay.to_string()])?;
                    db.execute("UPDATE discovery_pace SET next_at=strftime('%s','now')+?1 WHERE id=1",&[&delay.to_string()])?;
                    let detail: String = error.chars().take(180).collect();
                    format!("《{title}》检索失败：{detail}。{delay} 秒后可继续。")
                }
            };
            db.execute("UPDATE discovery_pace SET notice=?1 WHERE id=1", &[&notice])?;
            Ok(())
        })
    }

    pub async fn advance(&self, store: &Store) -> Result<BatchStatus, String> {
        let (id, token) = loop {
            match self.claim()? {
                Slot::Movie { id, token } => break (id, token),
                Slot::Pace(seconds) if seconds <= 2 => {
                    tokio::time::sleep(Duration::from_secs(seconds)).await;
                }
                Slot::Pace(seconds) => {
                    let mut status = self.status()?;
                    status.message = format!("来源稍后可重试，约 {seconds} 秒后继续。");
                    return Ok(status);
                }
                Slot::Unavailable(message) => {
                    let mut status = self.status()?;
                    status.message = message;
                    return Ok(status);
                }
            }
        };
        let movie = store.get(&id)?;
        let result = movie_discovery::search(&movie.title, &movie.original, movie.year).await;
        self.finish(&id, &token, &movie.title, result)?;
        self.status()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (tempfile::TempDir, Queue, Store) {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("catalog.sqlite3");
        let store = Store::open(&path).unwrap();
        let queue = Queue::new(path);
        // Keep this fixture independent from changes to the published seed snapshot.
        queue
            .open()
            .unwrap()
            .execute("UPDATE douban_snapshots SET rating_tenths=0", &[])
            .unwrap();
        store.import(r#"[{"id":"99778811","title":"测试片","original":"Test Film","year":1999,"rating_tenths":80,"observed":"2001-01-01","kind":"movie"}]"#).unwrap();
        (temp, queue, store)
    }

    fn fixture(btih: char) -> Candidate {
        Candidate {
            title: "测试片".into(),
            uri: format!("magnet:?xt=urn:btih:{}", btih.to_string().repeat(40)),
            source: "https://archive.org/details/test".into(),
            reason: "原名匹配".into(),
            license_url: "https://creativecommons.org/licenses/by/4.0/".into(),
        }
    }

    #[test]
    fn existing_discoveries_gain_only_the_license_at_their_verified_source() {
        let (_dir, queue, _store) = setup();
        let id = "99778811";
        let film = fixture('a');
        queue.remember(id, std::slice::from_ref(&film)).unwrap();
        let db = queue.open().unwrap();
        // An earlier version had candidates and review decisions, with no license table.
        db.batch("DROP TABLE discovery_licenses").unwrap();
        let reopened = Queue::new(queue.path.clone());
        assert!(reopened.candidates(id).unwrap()[0].license_url.is_empty());
        assert_eq!(
            reopened
                .candidate_license(id, &film.uri, &film.source)
                .unwrap(),
            None
        );
        reopened.remember(id, std::slice::from_ref(&film)).unwrap();
        assert_eq!(
            reopened.candidates(id).unwrap()[0].license_url,
            film.license_url
        );
        let alternate_spelling = film.uri.to_ascii_uppercase();
        assert_eq!(
            reopened
                .candidate_license(id, &alternate_spelling.replace("XT=", "xt="), &film.source)
                .unwrap(),
            Some(film.license_url.clone())
        );
        assert_eq!(
            reopened.candidate_license(id, &film.uri, "manual").unwrap(),
            None
        );
        reopened.dismiss(id, &film.uri).unwrap();
        reopened.remember(id, &[film]).unwrap();
        assert!(reopened.candidates(id).unwrap().is_empty());
    }

    #[test]
    fn concurrent_workers_and_expired_owner_cannot_commit_twice() {
        let (_dir, queue, _store) = setup();
        let Slot::Movie { id, token: stale } = queue.claim().unwrap() else {
            panic!("没有获取首个任务")
        };
        let other = Queue::new(queue.path.clone());
        assert!(matches!(other.claim().unwrap(), Slot::Unavailable(_)));
        let db = queue.open().unwrap();
        db.batch("UPDATE discovery_jobs SET next_at=0; UPDATE discovery_pace SET next_at=0")
            .unwrap();
        let Slot::Movie { token, .. } = other.claim().unwrap() else {
            panic!("过期任务未恢复")
        };
        queue
            .finish(&id, &stale, "旧请求", Ok(vec![fixture('a')]))
            .unwrap();
        assert!(queue.candidates(&id).unwrap().is_empty());
        other
            .finish(&id, &token, "新请求", Ok(vec![fixture('b')]))
            .unwrap();
        let status = queue.status().unwrap();
        assert_eq!(
            (status.total, status.processed, status.pending_review),
            (1, 1, 1)
        );
        assert!(status.done);
        assert!(matches!(queue.claim().unwrap(), Slot::Unavailable(_)));
    }

    #[test]
    fn reviewed_and_ineligible_candidates_remain_hidden_after_restart() {
        let (_dir, queue, store) = setup();
        let id = "99778811";
        let first = fixture('a');
        let second = fixture('b');
        let third = fixture('c');
        queue
            .remember(id, &[first.clone(), second.clone(), third.clone()])
            .unwrap();
        queue.dismiss(id, &first.uri).unwrap();
        store.save_link(id, &second.uri, &second.source).unwrap();
        let restarted = Queue::new(queue.path.clone());
        restarted.remember(id, &[first, second, third]).unwrap();
        store.remove_link(id, &fixture('b').uri).unwrap();
        assert!(store.links(id).unwrap().is_empty());
        assert_eq!(restarted.candidates(id).unwrap().len(), 1);
        assert_eq!(restarted.review_ids().unwrap(), [id]);
        assert_eq!(restarted.status().unwrap().pending_review, 1);
        store.import(r#"[{"id":"99778811","title":"测试片","year":1999,"rating_tenths":79,"observed":"2002-01-01","kind":"movie"}]"#).unwrap();
        assert!(restarted.candidates(id).unwrap().is_empty());
        assert!(restarted.review_ids().unwrap().is_empty());
        assert_eq!(restarted.status().unwrap().total, 0);
    }

    #[test]
    fn errors_back_off_persistently_and_explicit_new_pass_retries_exhausted_failures() {
        let (_dir, queue, _store) = setup();
        for attempt in 1..=3 {
            let Slot::Movie { id, token } = queue.claim().unwrap() else {
                panic!("应取得可重试任务")
            };
            queue
                .finish(&id, &token, "测试片", Err("离线".into()))
                .unwrap();
            let status = queue.status().unwrap();
            assert_eq!(status.failed, 1);
            assert_eq!(status.done, attempt == 3);
            assert!(status.message.contains("离线"));
            let db = queue.open().unwrap();
            if attempt < 3 {
                assert!(matches!(queue.claim().unwrap(), Slot::Pace(_)));
            }
            db.batch("UPDATE discovery_jobs SET next_at=0; UPDATE discovery_pace SET next_at=0")
                .unwrap();
        }
        let Slot::Movie { id, token } = queue.claim().unwrap() else {
            panic!("应能手动重新发起")
        };
        queue.finish(&id, &token, "测试片", Ok(vec![])).unwrap();
        let status = queue.status().unwrap();
        assert!(status.done);
        assert_eq!(status.failed, 0);
    }
}
