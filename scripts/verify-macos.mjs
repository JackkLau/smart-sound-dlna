import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { readdir, readFile, lstat, mkdtemp, rm } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { reportFailure } from './ci-report.mjs';

const execute = promisify(execFile);
const archForTarget = { 'aarch64-apple-darwin': 'arm64', 'x86_64-apple-darwin': 'x86_64' };
async function plist(app, key) {
  return (await execute('/usr/libexec/PlistBuddy', ['-c', `Print :${key}`, path.join(app, 'Contents/Info.plist')])).stdout.trim();
}
export async function verifyMacosApp(app, { identifier, version, arch }) {
  // Verify the complete bundle, not just the linker's signature on the Mach-O.
  await execute('/usr/bin/codesign', ['--verify', '--deep', '--strict', '--verbose=2', app]);
  if (!(await lstat(path.join(app, 'Contents/_CodeSignature/CodeResources'))).isFile()) throw new Error('Missing macOS resource seal');
  if (await plist(app, 'CFBundleIdentifier') !== identifier) throw new Error('Unexpected macOS bundle identifier');
  if (await plist(app, 'CFBundleShortVersionString') !== version) throw new Error('Unexpected macOS application version');
  const executable = await plist(app, 'CFBundleExecutable');
  if (!executable || path.basename(executable) !== executable) throw new Error('Invalid macOS executable name');
  const binary = path.join(app, 'Contents/MacOS', executable);
  const actualArch = (await execute('/usr/bin/lipo', ['-archs', binary])).stdout.trim();
  if (actualArch !== arch) throw new Error(`Unexpected macOS architecture: expected ${arch}, got ${actualArch}`);
  return createHash('sha256').update(await readFile(binary)).digest('hex');
}
async function one(directory, extension) {
  const entries = (await readdir(directory, { withFileTypes: true })).filter(entry => entry.name.endsWith(extension) && (extension === '.app' ? entry.isDirectory() : entry.isFile()));
  if (entries.length !== 1) throw new Error(`Expected exactly one ${extension} in ${directory}`);
  return path.join(directory, entries[0].name);
}
export async function verifyMacosArtifacts(directory, { target, identifier, version }) {
  const arch = archForTarget[target];
  if (!arch) throw new Error('Unsupported macOS release target');
  const identity = { identifier, version, arch };
  const app = await one(path.join(directory, 'macos'), '.app');
  const expectedDigest = await verifyMacosApp(app, identity);
  const dmg = await one(path.join(directory, 'dmg'), '.dmg');
  await execute('/usr/bin/hdiutil', ['verify', dmg]);
  const mount = await mkdtemp(path.join(os.tmpdir(), 'xiaomi-dmg-check-'));
  let mounted = false;
  try {
    await execute('/usr/bin/hdiutil', ['attach', '-readonly', '-nobrowse', '-mountpoint', mount, dmg]);
    mounted = true;
    const packaged = await one(mount, '.app');
    if (await verifyMacosApp(packaged, identity) !== expectedDigest) throw new Error('DMG application differs from verified build');
  } finally {
    // Never modify the mounted image or the user's quarantine/Gatekeeper settings.
    if (mounted) await execute('/usr/bin/hdiutil', ['detach', mount]);
    await rm(mount, { recursive: true, force: true });
  }
}
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    if (process.platform !== 'darwin') throw new Error('macOS artifact verification must run on macOS');
    const config = JSON.parse(await readFile(new URL('../src-tauri/tauri.conf.json', import.meta.url), 'utf8'));
    await verifyMacosArtifacts(process.argv[2], { target: process.argv[3], identifier: config.identifier, version: config.version });
    process.stdout.write('Verified macOS app and DMG: sealed signature, version, architecture and identical executable.\n');
  } catch (error) { reportFailure('macOS installer verification failed', error); }
}
