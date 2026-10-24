const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const os = require('node:os');
const path = require('node:path');
const crypto = require('node:crypto');

test('release packages preserve each platform, reject missing/invalid installers and publish checksums', async () => {
  const { prepareRelease } = await import('../scripts/prepare-release.mjs');
  const root = await fs.mkdtemp(path.join(os.tmpdir(), 'xiaomi-release-'));
  try {
    const exe = Buffer.alloc(512); exe.write('MZ'); exe.writeUInt32LE(128, 0x3c); exe.write('PE\0\0', 128);
    const arm = Buffer.alloc(1024, 1); arm.write('koly', 512);
    const intel = Buffer.alloc(1024, 2); intel.write('koly', 512);
    const inputs = [
      ['x86_64-pc-windows-msvc', 'nsis/app.exe', exe, 'xiaomi-music-windows-x64-setup.exe'],
      ['aarch64-apple-darwin', 'dmg/app.dmg', arm, 'xiaomi-music-macos-apple-silicon.dmg'],
      ['x86_64-apple-darwin', 'dmg/app.dmg', intel, 'xiaomi-music-macos-intel.dmg'],
    ];
    const identity = { version: '0.6.0', sha: 'a'.repeat(40), runUrl: 'https://github.com/example/repo/actions/runs/1' };
    for (const [target, relative, bytes] of inputs) {
      const filename = path.join(root, `xiaomi-music-${target}`, relative);
      await fs.mkdir(path.dirname(filename), { recursive: true }); await fs.writeFile(filename, bytes);
    }
    const output = path.join(root, 'ready');
    await prepareRelease(root, output, identity);
    for (const [, , bytes, name] of inputs) assert.deepEqual(await fs.readFile(path.join(output, name)), bytes);
    const sums = await fs.readFile(path.join(output, 'SHA256SUMS'), 'utf8');
    for (const [, , bytes, name] of inputs) assert.ok(sums.includes(`${crypto.createHash('sha256').update(bytes).digest('hex')}  ${name}\n`));
    await fs.writeFile(path.join(root, 'xiaomi-music-x86_64-apple-darwin/dmg/app.dmg'), 'not an installer');
    await assert.rejects(prepareRelease(root, path.join(root, 'bad'), identity), /Invalid installer/);
    await assert.rejects(fs.access(path.join(root, 'bad')), /ENOENT/);
    await fs.rm(path.join(root, 'xiaomi-music-x86_64-apple-darwin/dmg/app.dmg'));
    await assert.rejects(prepareRelease(root, path.join(root, 'missing'), identity), /exactly one/);
  } finally { await fs.rm(root, { recursive: true, force: true }); }
});
