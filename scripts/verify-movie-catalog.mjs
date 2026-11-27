import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdtempSync, readFileSync, realpathSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { DatabaseSync } from 'node:sqlite';
import { fileURLToPath } from 'node:url';

const data = new URL('../src-tauri/data/', import.meta.url);
const readSeed = (name) => JSON.parse(readFileSync(new URL(name, data), 'utf8'));
const hash = (filename) => createHash('sha256').update(readFileSync(filename)).digest('hex');

function checkRecords(filename) {
  const db = new DatabaseSync(filename, { readOnly: true });
  try {
    assert.deepEqual(
      db
        .prepare('PRAGMA integrity_check')
        .all()
        .map((row) => row.integrity_check),
      ['ok'],
    );
    assert.equal(
      db.prepare('PRAGMA foreign_key_check').all().length,
      0,
      'Dangling catalog reference',
    );

    const openFilms = readSeed('open-film-resources.json');
    const snapshots = new Map();
    for (const film of [...readSeed('movie-seed.json'), ...openFilms]) {
      const previous = snapshots.get(film.id);
      if (!previous || previous.observed < film.observed) snapshots.set(film.id, film);
    }
    const movies = db
      .prepare(
        'SELECT DISTINCT douban_id,title,original_title,year,douban_rating,rating_observed_on,rating_evidence_url FROM film_catalog',
      )
      .all();
    assert.ok(snapshots.size > 0, 'Missing movie seed');
    assert.equal(movies.length, snapshots.size, 'Unexpected number of bundled films');
    const currentYear = new Date().getFullYear();
    for (const movie of movies) {
      const film = snapshots.get(movie.douban_id);
      assert.ok(film, `Unexpected Douban ID ${movie.douban_id}`);
      assert.ok(
        movie.year >= 1990 &&
          movie.year <= currentYear &&
          movie.douban_rating >= 8 &&
          movie.douban_rating <= 10,
      );
      assert.deepEqual(
        [
          movie.title,
          movie.original_title,
          movie.year,
          Math.round(movie.douban_rating * 10),
          movie.rating_observed_on,
          movie.rating_evidence_url,
        ],
        [
          film.title,
          film.original || '',
          film.year,
          film.rating_tenths,
          film.observed,
          film.rating_evidence || null,
        ],
        `Bundled snapshot differs for Douban ID ${film.id}`,
      );
    }

    const resources = db
      .prepare(
        'SELECT douban_id, btih, magnet_uri, source_url, license_url FROM film_catalog WHERE magnet_uri IS NOT NULL',
      )
      .all();
    assert.ok(openFilms.length > 0, 'Missing magnet seed');
    assert.equal(
      resources.length,
      openFilms.length,
      'Unexpected number of bundled resource records',
    );
    for (const film of openFilms) {
      const uri = new URL(film.uri);
      const btih = uri.searchParams.get('xt').slice('urn:btih:'.length).toLowerCase();
      const resource = resources.find(
        (row) => row.douban_id === film.id && row.btih === btih && row.source_url === film.source,
      );
      assert.ok(resource, `Missing resource ${btih} for Douban ID ${film.id}`);
      const packaged = new URL(resource.magnet_uri);
      assert.equal(packaged.protocol, 'magnet:');
      assert.deepEqual([...packaged.searchParams], [...uri.searchParams]);
      assert.equal(resource.license_url, film.license_url);
    }
    return { movies: movies.length, resources: resources.length };
  } finally {
    db.close();
  }
}

const [app, target, mode] = process.argv.slice(2);
const resources = {
  'aarch64-apple-darwin': 'Contents/Resources',
  'x86_64-apple-darwin': 'Contents/Resources',
  'x86_64-pc-windows-msvc': '.',
};
if (
  !app ||
  !Object.hasOwn(resources, target) ||
  (mode !== undefined && mode !== '--first-run') ||
  process.argv.length > 5
) {
  throw new Error(
    'Usage: verify-movie-catalog.mjs <app-or-installed-directory> <target> [--first-run]',
  );
}
const template = path.join(
  realpathSync(app),
  resources[target],
  'catalog',
  'movie-library.sqlite3',
);
assert.equal(
  realpathSync(template),
  template,
  'Catalog must be a file inside the installed application',
);
const sha256 = hash(new URL('movie-library.sqlite3', data));
assert.equal(hash(template), sha256, 'Installed SQLite catalog differs from the release source');
const counts = checkRecords(template);

if (mode === '--first-run') {
  const directory = mkdtempSync(path.join(tmpdir(), 'movie-catalog-install-'));
  try {
    const userDatabase = path.join(directory, 'profile', 'movie-library.sqlite3');
    const cargo = fileURLToPath(new URL('./cargo.mjs', import.meta.url));
    execFileSync(
      process.execPath,
      [
        cargo,
        'run',
        '--quiet',
        '--locked',
        '--manifest-path',
        'src-tauri/Cargo.toml',
        '--no-default-features',
        '--example',
        'movie_library_db',
        '--',
        '--from-bundle',
        template,
        userDatabase,
      ],
      { stdio: ['ignore', 'ignore', 'inherit'] },
    );
    assert.equal(
      hash(userDatabase),
      sha256,
      'First run did not install a complete copy of the bundled catalog',
    );
    const writable = new DatabaseSync(userDatabase, { readOnly: false });
    try {
      writable.exec('BEGIN IMMEDIATE; CREATE TABLE installer_smoke_check (value TEXT); ROLLBACK');
    } finally {
      writable.close();
    }
    assert.equal(hash(template), sha256, 'First run modified the installed SQLite template');
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
}
console.log(JSON.stringify({ ...counts, sha256, first_run: mode === '--first-run' }));
