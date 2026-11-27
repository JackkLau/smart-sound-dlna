const { test } = require('node:test');
const assert = require('node:assert/strict');
const { copyFileSync, mkdtempSync, rmSync } = require('node:fs');
const { tmpdir } = require('node:os');
const path = require('node:path');

const DatabaseSync = process.getBuiltinModule('node:sqlite')?.DatabaseSync;

test(
  'the bundled film catalog filters by year and rating when SQLite lacks localtime',
  {
    skip:
      typeof DatabaseSync?.prototype.function !== 'function' &&
      'requires SQLite function registration in Node 22.13+',
  },
  (t) => {
    const dir = mkdtempSync(path.join(tmpdir(), 'catalog-calendar-'));
    t.after(() => rmSync(dir, { recursive: true, force: true }));
    const template = path.join(__dirname, '../src-tauri/data/movie-library.sqlite3');
    const databasePath = path.join(dir, 'catalog.sqlite3');
    copyFileSync(template, databasePath);
    const db = new DatabaseSync(databasePath);
    const calendar = new DatabaseSync(':memory:');
    try {
      // Windows inbox SQLite can return NULL for the optional localtime modifier.
      const nativeFormat = calendar.prepare('SELECT strftime(?, ?, ?) AS value');
      db.function('strftime', (format, value, modifier) =>
        modifier === 'localtime' ? null : nativeFormat.get(format, value, modifier).value,
      );
      assert.equal(
        db.prepare("SELECT strftime('%Y', 'now', 'localtime') AS year").get().year,
        null,
      );
      const currentYear = new Date().getUTCFullYear();
      const insert = db.prepare(
        "INSERT INTO douban_snapshots (id, title, original, year, rating_tenths, observed, kind) VALUES (?, 'Calendar fixture', '', ?, ?, '2020-01-01', ?)",
      );
      for (const [id, year, rating, kind] of [
        ['900000000001', 1989, 90, 'movie'],
        ['900000000002', 1990, 80, 'movie'],
        ['900000000003', 2000, 79, 'movie'],
        ['900000000004', currentYear, 90, 'movie'],
        ['900000000005', currentYear + 1, 90, 'movie'],
        ['900000000006', 2000, 90, 'tv'],
      ]) {
        insert.run(id, year, rating, kind);
      }
      const rows = db
        .prepare(
          "SELECT douban_id FROM film_catalog WHERE douban_id BETWEEN '900000000001' AND '900000000006' ORDER BY douban_id",
        )
        .all();
      assert.deepEqual(
        rows.map((row) => row.douban_id),
        ['900000000002', '900000000004'],
      );
    } finally {
      calendar.close();
      db.close();
    }
  },
);
