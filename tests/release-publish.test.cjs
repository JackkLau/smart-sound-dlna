const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const os = require('node:os');
const path = require('node:path');

test('releases publish only after all assets upload and never replace a newer or mismatched source', async () => {
  const { publishRelease, releaseAssets } = await import('../scripts/publish-release.mjs');
  const root = await fs.mkdtemp(path.join(os.tmpdir(), 'xiaomi-publish-'));
  try {
    for (const name of [...releaseAssets, 'release-notes.md']) await fs.writeFile(path.join(root, name), 'fixture');
    const options = { repo: 'example/music', sha: 'a'.repeat(40), defaultBranch: 'master', runNumber: 7, version: '0.6.0', directory: root };
    function github({ existing, newer = false, networkError = false } = {}) {
      const calls = []; let heads = 0;
      return { calls, async execute(args) {
        calls.push(args);
        if (args[0] === 'api' && args[1].includes('/commits/')) return ++heads > 1 && newer ? 'b'.repeat(40) : options.sha;
        if (args[0] === 'api') {
          if (networkError) throw Object.assign(new Error('unavailable'), { stderr: 'HTTP 503: unavailable' });
          if (existing) return JSON.stringify(existing);
          throw Object.assign(new Error('not found'), { stderr: 'HTTP 404: Not Found' });
        }
        return '';
      } };
    }
    const happy = github();
    assert.equal((await publishRelease(options, happy.execute)).status, 'published');
    const mutations = happy.calls.filter(call => call[0] === 'release');
    assert.deepEqual(mutations.map(call => call[1]), ['create', 'upload', 'edit']);
    assert.ok(mutations[0].includes('--draft'));
    assert.ok(mutations[2].includes('--draft=false'));
    for (const name of releaseAssets) assert.ok(mutations[1].includes(path.join(root, name)));
    const superseded = github({ newer: true });
    assert.equal((await publishRelease(options, superseded.execute)).status, 'superseded');
    assert.ok(!superseded.calls.some(call => call[0] === 'release' && call[1] === 'edit'));
    const wrong = github({ existing: { target_commitish: 'b'.repeat(40), draft: true } });
    await assert.rejects(publishRelease(options, wrong.execute), /source mismatch/);
    assert.ok(!wrong.calls.some(call => call[0] === 'release'));
    const outage = github({ networkError: true });
    await assert.rejects(publishRelease(options, outage.execute), /unavailable/);
    assert.ok(!outage.calls.some(call => call[0] === 'release'));
    const existing = github({ existing: { target_commitish: options.sha, draft: false, assets: releaseAssets.map(name => ({ name, size: 10 })) } });
    assert.equal((await publishRelease(options, existing.execute)).status, 'exists');
    assert.ok(!existing.calls.some(call => call[0] === 'release'));
  } finally { await fs.rm(root, { recursive: true, force: true }); }
});
