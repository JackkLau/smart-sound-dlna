const { test } = require('node:test');
const assert = require('node:assert/strict');
const { mkdtemp, mkdir, writeFile, readFile, rm, symlink } = require('node:fs/promises');
const os = require('node:os');
const path = require('node:path');

function macho(cpu = 0x0100000c) {
  const bytes = Buffer.alloc(64);
  bytes.writeUInt32LE(0xfeedfacf, 0);
  bytes.writeUInt32LE(cpu, 4);
  bytes.writeUInt32LE(2, 12);
  return bytes;
}
function pe(machine = 0x8664) {
  const bytes = Buffer.alloc(256);
  bytes.write('MZ', 0);
  bytes.writeUInt32LE(128, 0x3c);
  bytes.write('PE\0\0', 128);
  bytes.writeUInt16LE(machine, 132);
  bytes.writeUInt16LE(0x20b, 152);
  return bytes;
}
async function fixture(target = 'aarch64-apple-darwin') {
  const root = await mkdtemp(path.join(os.tmpdir(), 'video bundle 中文 '));
  const mac = target.endsWith('apple-darwin');
  const executableDir = mac ? path.join(root, 'Contents/MacOS') : root;
  const resourceDir = mac ? path.join(root, 'Contents/Resources') : root;
  await mkdir(executableDir, { recursive: true });
  await mkdir(path.join(resourceDir, 'licenses'), { recursive: true });
  const probe = path.join(executableDir, mac ? 'ffprobe' : 'ffprobe.exe');
  const main = path.join(executableDir, `xiaomi-dlna-desktop${mac ? '' : '.exe'}`);
  const bytes = mac ? macho(target.startsWith('aarch64') ? 0x0100000c : 0x01000007) : pe();
  await writeFile(probe, bytes);
  await writeFile(path.join(executableDir, mac ? 'ffmpeg' : 'ffmpeg.exe'), bytes);
  await writeFile(main, bytes);
  const specs = JSON.parse(
    await readFile(path.join(__dirname, '../scripts/video-tools.json'), 'utf8'),
  );
  const spec = specs.targets[target];
  const manifest = {
    target,
    package: spec.package,
    package_version: spec.version,
    source_binary_sha256: spec.sha256,
    version: 'ffprobe version fixture\n',
    license: 'fixture license\n',
    source: 'https://github.com/SavageCore/node-ffprobe-installer',
  };
  const manifestFile = path.join(resourceDir, 'ffprobe-manifest.json');
  await writeFile(manifestFile, JSON.stringify(manifest));
  await writeFile(path.join(resourceDir, 'licenses/ffprobe.md'), 'Fixture tool declaration');
  return { root, probe, main, manifest, manifestFile, resourceDir };
}

test('bundle verification reads target architecture from executable headers, not filenames', async () => {
  const { executableTarget } = await import('../scripts/verify-video-bundle.mjs');
  assert.equal(executableTarget(macho()), 'aarch64-apple-darwin');
  assert.equal(executableTarget(macho(0x01000007)), 'x86_64-apple-darwin');
  assert.equal(executableTarget(pe()), 'x86_64-pc-windows-msvc');
  for (const bytes of [Buffer.alloc(3), macho(7), pe(0x14c), Buffer.from('shell script')]) {
    assert.throws(() => executableTarget(bytes), /executable|architecture/i);
  }
  const truncated = pe();
  truncated.writeUInt32LE(0xffffffff, 0x3c);
  assert.throws(() => executableTarget(truncated), /executable/i);
});

test('installed package requires pinned manifest, declaration, sidecar and matching main architecture', async () => {
  const { inspectVideoBundle } = await import('../scripts/verify-video-bundle.mjs');
  const value = await fixture();
  try {
    const result = await inspectVideoBundle(value.root, value.manifest.target);
    assert.equal(result.probe, value.probe);
    for (const [field, invalid] of [
      ['target', 'x86_64-apple-darwin'],
      ['package', 'different-package'],
      ['package_version', 'latest'],
      ['source_binary_sha256', '0'.repeat(64)],
      ['version', ''],
      ['license', ''],
    ]) {
      await writeFile(value.manifestFile, JSON.stringify({ ...value.manifest, [field]: invalid }));
      await assert.rejects(inspectVideoBundle(value.root, value.manifest.target), /manifest/i);
    }
    await writeFile(value.manifestFile, JSON.stringify(value.manifest));
    await writeFile(value.probe, macho(0x01000007));
    await assert.rejects(inspectVideoBundle(value.root, value.manifest.target), /architecture/i);
    await writeFile(value.probe, macho());
    await writeFile(value.main, macho(0x01000007));
    await assert.rejects(inspectVideoBundle(value.root, value.manifest.target), /architecture/i);
    await writeFile(value.main, macho());
    await rm(path.join(value.resourceDir, 'licenses/ffprobe.md'));
    await assert.rejects(inspectVideoBundle(value.root, value.manifest.target), /ffprobe.md/);
    await writeFile(path.join(value.resourceDir, 'licenses/ffprobe.md'), 'declaration');
    await rm(value.probe);
    await assert.rejects(inspectVideoBundle(value.root, value.manifest.target), /ffprobe/);
  } finally {
    await rm(value.root, { recursive: true, force: true });
  }
});

test('Windows installed layout resolves sidecar beside EXE and never searches PATH', async () => {
  const { inspectVideoBundle, probeEnvironment } =
    await import('../scripts/verify-video-bundle.mjs');
  const value = await fixture('x86_64-pc-windows-msvc');
  try {
    const result = await inspectVideoBundle(value.root, value.manifest.target);
    assert.equal(result.probe, path.join(value.root, 'ffprobe.exe'));
    const environment = probeEnvironment({
      PATH: '/external/tools',
      Path: 'C:\\tools',
      SystemRoot: 'C:\\Windows',
      TOKEN: 'private',
    });
    assert.equal(environment.PATH, '');
    assert.equal(environment.Path, undefined);
    assert.equal(environment.TOKEN, undefined);
    assert.equal(environment.SystemRoot, 'C:\\Windows');
  } finally {
    await rm(value.root, { recursive: true, force: true });
  }
});

test(
  'bundle resource paths cannot escape through a symbolic link',
  { skip: process.platform === 'win32' },
  async () => {
    const { inspectVideoBundle } = await import('../scripts/verify-video-bundle.mjs');
    const value = await fixture();
    const other = await fixture();
    try {
      await rm(value.probe);
      await symlink(other.probe, value.probe);
      await assert.rejects(
        inspectVideoBundle(value.root, value.manifest.target),
        /regular file|outside/i,
      );
    } finally {
      await rm(value.root, { recursive: true, force: true });
      await rm(other.root, { recursive: true, force: true });
    }
  },
);

test('signature normalization only ignores the LINKEDIT virtual allocation', async () => {
  const { unsignedMachODigest } = await import('../scripts/verify-video-bundle.mjs');
  const original = Buffer.alloc(128);
  macho().copy(original);
  original.writeUInt32LE(1, 16);
  original.writeUInt32LE(72, 20);
  original.writeUInt32LE(0x19, 32);
  original.writeUInt32LE(72, 36);
  original.write('__LINKEDIT', 40);
  original.writeBigUInt64LE(16384n, 64);
  const resigned = Buffer.from(original);
  resigned.writeBigUInt64LE(32768n, 64);
  assert.equal(unsignedMachODigest(original), unsignedMachODigest(resigned));
  resigned[120] = 1;
  assert.notEqual(unsignedMachODigest(original), unsignedMachODigest(resigned));
  const signed = Buffer.from(original);
  signed.writeUInt32LE(0x1d, 32);
  assert.throws(() => unsignedMachODigest(signed), /Remove Mach-O signature/);
});
