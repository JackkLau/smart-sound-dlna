import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { copyFile, lstat, mkdtemp, readFile, realpath, rm } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { hostTarget, toolManifest } from './prepare-video-tools.mjs';
import { verifyMacosApp } from './verify-macos.mjs';

const execute = promisify(execFile);
const projectRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const MAX_OUTPUT = 1024 * 1024;
const digest = (bytes) => createHash('sha256').update(bytes).digest('hex');
const normalized = (value) => value.replace(/\r\n/g, '\n').trim();

// The package name and filename cannot establish the executable architecture.
export function executableTarget(bytes) {
  if (bytes.length < 32) throw new Error('Truncated executable header');
  if (bytes.readUInt32LE(0) === 0xfeedfacf) {
    const cpu = bytes.readUInt32LE(4);
    if (cpu === 0x0100000c) return 'aarch64-apple-darwin';
    if (cpu === 0x01000007) return 'x86_64-apple-darwin';
    throw new Error('Unsupported Mach-O architecture');
  }
  if (bytes.length >= 64 && bytes.toString('ascii', 0, 2) === 'MZ') {
    const offset = bytes.readUInt32LE(0x3c);
    if (
      offset < 64 ||
      offset > bytes.length - 26 ||
      bytes.toString('ascii', offset, offset + 4) !== 'PE\0\0'
    ) {
      throw new Error('Invalid PE executable header');
    }
    if (bytes.readUInt16LE(offset + 4) === 0x8664 && bytes.readUInt16LE(offset + 24) === 0x20b) {
      return 'x86_64-pc-windows-msvc';
    }
    throw new Error('Unsupported PE architecture');
  }
  throw new Error('Unsupported executable format');
}

function machCommands(bytes) {
  executableTarget(bytes);
  if (bytes.readUInt32LE(0) !== 0xfeedfacf) throw new Error('Expected Mach-O executable');
  const end = 32 + bytes.readUInt32LE(20);
  if (end > bytes.length) throw new Error('Truncated Mach-O load commands');
  const commands = [];
  let offset = 32;
  for (let index = 0; index < bytes.readUInt32LE(16); index += 1) {
    if (offset + 8 > end) throw new Error('Truncated Mach-O load command');
    const size = bytes.readUInt32LE(offset + 4);
    if (size < 8 || offset + size > end) throw new Error('Invalid Mach-O load command');
    commands.push({ type: bytes.readUInt32LE(offset), offset, size });
    offset += size;
  }
  if (offset !== end) throw new Error('Invalid Mach-O load command length');
  return commands;
}

export function unsignedMachODigest(bytes) {
  const copy = Buffer.from(bytes);
  for (const command of machCommands(copy)) {
    if (command.type === 0x1d) throw new Error('Remove Mach-O signature before comparing code');
    if (
      command.type === 0x19 &&
      command.size >= 72 &&
      copy.toString('ascii', command.offset + 8, command.offset + 24).replace(/\0/g, '') ===
        '__LINKEDIT'
    ) {
      // codesign removal retains the virtual allocation for the old signature.
      // Only this allocation changes; compare every code/data byte and all other metadata.
      copy.writeBigUInt64LE(0n, command.offset + 32);
    }
  }
  return digest(copy);
}

async function bundleFile(root, relative, maxSize = 512 * 1024 * 1024) {
  const file = path.join(root, relative);
  const info = await lstat(file);
  if (!info.isFile() || info.isSymbolicLink() || !info.size || info.size > maxSize) {
    throw new Error(`Expected a nonempty regular file: ${relative}`);
  }
  const base = await realpath(root);
  const resolved = await realpath(file);
  const rel = path.relative(base, resolved);
  if (rel.startsWith(`..${path.sep}`) || rel === '..' || path.isAbsolute(rel)) {
    throw new Error(`Bundle resource resolves outside its directory: ${relative}`);
  }
  return file;
}

export async function inspectVideoBundle(directory, target) {
  const spec = toolManifest.targets[target];
  if (!spec) throw new Error('Unsupported video bundle target');
  const root = path.resolve(directory);
  const mac = spec.platform === 'darwin';
  const executableDir = mac ? 'Contents/MacOS/' : '';
  const resourceDir = mac ? 'Contents/Resources/' : '';
  const probe = await bundleFile(root, `${executableDir}${spec.file}`);
  const encoder = await bundleFile(root, executableDir + (mac ? 'ffmpeg' : 'ffmpeg.exe'));
  const main = await bundleFile(root, `${executableDir}xiaomi-dlna-desktop${mac ? '' : '.exe'}`);
  const manifestFile = await bundleFile(root, `${resourceDir}ffprobe-manifest.json`, MAX_OUTPUT);
  await bundleFile(root, `${resourceDir}licenses/ffprobe.md`, MAX_OUTPUT);
  let manifest;
  try {
    manifest = JSON.parse(await readFile(manifestFile, 'utf8'));
    assert.equal(manifest.target, target);
    assert.equal(manifest.package, spec.package);
    assert.equal(manifest.package_version, spec.version);
    assert.equal(manifest.source_binary_sha256, spec.sha256);
    assert.equal(manifest.source, 'https://github.com/SavageCore/node-ffprobe-installer');
    assert.ok(
      typeof manifest.version === 'string' && manifest.version.startsWith('ffprobe version '),
    );
    assert.ok(typeof manifest.license === 'string' && manifest.license.trim());
  } catch {
    throw new Error('Packaged FFprobe manifest does not match the pinned tool source');
  }
  for (const file of [probe, encoder, main]) {
    if (executableTarget(await readFile(file)) !== target) {
      throw new Error(`Unexpected executable architecture: ${path.basename(file)}`);
    }
  }
  return { root, target, spec, probe, encoder, main, manifest };
}

// Prove the installed tool works independently of developer tools on PATH.
// Do not pass account/service credentials or dynamic-loader overrides to it.
export function probeEnvironment(environment = process.env) {
  const result = { PATH: '', LANG: 'C', LC_ALL: 'C' };
  for (const name of ['SystemRoot', 'WINDIR', 'TEMP', 'TMP']) {
    if (environment[name]) result[name] = environment[name];
  }
  return result;
}

async function verifyToolBytes(bundle, file, name, expectedHash) {
  const packaged = await readFile(file);
  if (bundle.spec.platform === 'win32') {
    if (digest(packaged) !== expectedHash)
      throw new Error('Packaged tool bytes do not match the pinned source');
    return;
  }
  // Tauri replaces the Mach-O signature while assembling the app. Compare
  // copies with their signatures removed; never mutate the app or source tool.
  const staged = path.join(projectRoot, 'src-tauri/binaries', `${name}-${bundle.target}`);
  if (digest(await readFile(staged)) !== expectedHash) {
    throw new Error('Staged tool bytes do not match the pinned source');
  }
  const temporary = await mkdtemp(path.join(os.tmpdir(), 'video-probe-identity-'));
  try {
    const copies = [path.join(temporary, 'packaged'), path.join(temporary, 'source')];
    await copyFile(file, copies[0]);
    await copyFile(staged, copies[1]);
    // The pinned Intel binary is unsigned. Its first signature aligns __LINKEDIT
    // with trailing zeroes that codesign retains on removal. Apply the same
    // round trip to the verified source copy so every file byte is compared.
    if (!machCommands(await readFile(copies[1])).some((command) => command.type === 0x1d)) {
      await execute('/usr/bin/codesign', ['--sign', '-', copies[1]], {
        timeout: 10000,
        maxBuffer: MAX_OUTPUT,
      });
    }
    for (const file of copies) {
      if (machCommands(await readFile(file)).some((command) => command.type === 0x1d)) {
        await execute('/usr/bin/codesign', ['--remove-signature', file], {
          timeout: 10000,
          maxBuffer: MAX_OUTPUT,
        });
      }
    }
    if (
      unsignedMachODigest(await readFile(copies[0])) !==
      unsignedMachODigest(await readFile(copies[1]))
    ) {
      throw new Error('Packaged tool code differs from the pinned source');
    }
  } finally {
    await rm(temporary, { recursive: true, force: true });
  }
}

export async function verifyVideoBundle(directory, target) {
  if (target !== hostTarget())
    throw new Error('Video bundle must be executed on its native target runner');
  const bundle = await inspectVideoBundle(directory, target);
  await verifyToolBytes(bundle, bundle.probe, 'ffprobe', bundle.spec.sha256);
  const encoders = JSON.parse(await readFile(path.join(projectRoot, 'scripts/subtitle-tools.json'), 'utf8'));
  await verifyToolBytes(bundle, bundle.encoder, 'ffmpeg', encoders[target].sha256);
  if (bundle.spec.platform === 'darwin') {
    const config = JSON.parse(
      await readFile(path.join(projectRoot, 'src-tauri/tauri.conf.json'), 'utf8'),
    );
    await verifyMacosApp(bundle.root, {
      identifier: config.identifier,
      version: config.version,
      arch: bundle.spec.arch === 'arm64' ? 'arm64' : 'x86_64',
    });
  }
  const options = {
    env: probeEnvironment(),
    windowsHide: true,
    timeout: 10000,
    maxBuffer: MAX_OUTPUT,
  };
  const encoderVersion = (await execute(bundle.encoder, ['-version'], options)).stdout;
  assert.match(encoderVersion, /^ffmpeg version /);
  const version = (await execute(bundle.probe, ['-version'], options)).stdout;
  const license = (await execute(bundle.probe, ['-L'], options)).stdout;
  if (
    normalized(version) !== normalized(bundle.manifest.version) ||
    normalized(license) !== normalized(bundle.manifest.license)
  ) {
    throw new Error('Packaged FFprobe runtime output differs from its build manifest');
  }
  const fixture = path.join(projectRoot, 'src-tauri/tests/fixtures/generated-video.mkv');
  const output = (
    await execute(
      bundle.probe,
      [
        '-v',
        'error',
        '-protocol_whitelist',
        'file',
        '-format_whitelist',
        'matroska',
        '-show_entries',
        'format=duration:stream=codec_name,width,height',
        '-of',
        'json',
        fixture,
      ],
      options,
    )
  ).stdout;
  const result = JSON.parse(output);
  if (
    result.streams?.length !== 1 ||
    result.streams[0].codec_name !== 'png' ||
    result.streams[0].width !== 16 ||
    result.streams[0].height !== 16 ||
    Number(result.format?.duration) !== 2
  ) {
    throw new Error('Packaged FFprobe failed the real Matroska fixture contract');
  }
  return {
    target,
    version: version.split(/\r?\n/)[0],
    path_independent: true,
    fixture_verified: true,
  };
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    if (process.argv.length !== 4)
      throw new Error(
        'Usage: node scripts/verify-video-bundle.mjs <app-or-installed-directory> <target>',
      );
    const result = await verifyVideoBundle(process.argv[2], process.argv[3]);
    console.log(`Verified packaged video tool: ${JSON.stringify(result)}`);
  } catch (error) {
    console.error(`Video bundle verification failed: ${error.message}`);
    process.exitCode = 1;
  }
}
