const { test } = require('node:test');
const assert = require('node:assert/strict');
const { mkdtemp, mkdir, writeFile, readFile, rm } = require('node:fs/promises');
const { createHash } = require('node:crypto');
const os = require('node:os');
const path = require('node:path');

test('video packaging rejects unsupported and foreign target triples', async () => {
  const { hostTarget, prepareVideoTools } = await import('../scripts/prepare-video-tools.mjs');
  assert.equal(hostTarget('darwin', 'arm64'), 'aarch64-apple-darwin');
  assert.equal(hostTarget('win32', 'x64'), 'x86_64-pc-windows-msvc');
  assert.throws(() => hostTarget('win32', 'arm64'), /仅为/);
  assert.throws(() => hostTarget('linux', 'x64'), /仅为/);
  const supportedHost =
    (process.platform === 'darwin' && ['arm64', 'x64'].includes(process.arch)) ||
    (process.platform === 'win32' && process.arch === 'x64');
  // Unsupported hosts fail before cross-target validation; both must reject
  // before reading dependencies or creating a staged binary.
  await assert.rejects(
    prepareVideoTools('/unused', 'wrong-target'),
    supportedHost ? /目标平台/ : /仅为/,
  );
});

test('video sidecar checks exact package and bytes before staging', async () => {
  const { stageProbe } = await import('../scripts/prepare-video-tools.mjs');
  const root = await mkdtemp(path.join(os.tmpdir(), 'video tools '));
  try {
    const packageDir = path.join(root, 'package');
    const destination = path.join(root, 'output');
    await mkdir(packageDir);
    const bytes = Buffer.from('test executable bytes');
    const spec = {
      package: 'fixture-probe',
      version: '1.0.0',
      file: 'ffprobe',
      sha256: createHash('sha256').update(bytes).digest('hex'),
    };
    await writeFile(
      path.join(packageDir, 'package.json'),
      JSON.stringify({
        name: spec.package,
        version: spec.version,
      }),
    );
    await writeFile(path.join(packageDir, spec.file), bytes);
    const result = await stageProbe({ packageDir, destination, target: 'test-target', spec });
    assert.deepEqual(await readFile(result), bytes);
    await writeFile(path.join(packageDir, spec.file), 'changed');
    await assert.rejects(
      stageProbe({ packageDir, destination, target: 'test-target', spec }),
      /校验失败/,
    );
    await writeFile(
      path.join(packageDir, 'package.json'),
      '{"name":"fixture-probe","version":"2"}',
    );
    await assert.rejects(
      stageProbe({ packageDir, destination, target: 'test-target', spec }),
      /版本/,
    );
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});
