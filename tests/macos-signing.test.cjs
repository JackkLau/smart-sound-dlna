const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const os = require('node:os');
const path = require('node:path');
const { execFile } = require('node:child_process');
const { promisify } = require('node:util');
const execute = promisify(execFile);

test('macOS release rejects a standalone signature and tampered bundle resources', { skip: process.platform !== 'darwin' }, async () => {
  const { verifyMacosApp } = await import('../scripts/verify-macos.mjs');
  const root = await fs.mkdtemp(path.join(os.tmpdir(), 'xiaomi-sign-test-'));
  const identity = { identifier: 'com.localmusic.xiaomi-dlna', version: '0.7.2', arch: process.arch === 'arm64' ? 'arm64' : 'x86_64' };
  try {
    const source = path.join(root, 'main.c'); const binary = path.join(root, 'main');
    await fs.writeFile(source, 'int main(void) { return 0; }\n');
    await execute('/usr/bin/clang', [source, '-o', binary]);
    await execute('/usr/bin/codesign', ['--force', '--sign', '-', binary]);
    const app = path.join(root, 'Fixture.app');
    await fs.mkdir(path.join(app, 'Contents/MacOS'), { recursive: true });
    await fs.mkdir(path.join(app, 'Contents/Resources'), { recursive: true });
    await fs.copyFile(binary, path.join(app, 'Contents/MacOS/main'));
    await fs.writeFile(path.join(app, 'Contents/Resources/music.txt'), 'original resource');
    await fs.writeFile(path.join(app, 'Contents/Info.plist'), `<?xml version="1.0" encoding="UTF-8"?><!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd"><plist version="1.0"><dict><key>CFBundleIdentifier</key><string>${identity.identifier}</string><key>CFBundleExecutable</key><string>main</string><key>CFBundlePackageType</key><string>APPL</string><key>CFBundleShortVersionString</key><string>${identity.version}</string></dict></plist>`);
    // This is the former release: a signed standalone executable copied into an
    // app, without signing the Info.plist or resources of the assembled bundle.
    await assert.rejects(verifyMacosApp(app, identity), /resources|Info.plist|not signed/i);
    await execute('/usr/bin/codesign', ['--force', '--sign', '-', '--identifier', identity.identifier, app]);
    assert.match(await verifyMacosApp(app, identity), /^[a-f0-9]{64}$/);
    await assert.rejects(verifyMacosApp(app, { ...identity, arch: identity.arch === 'arm64' ? 'x86_64' : 'arm64' }), /architecture/);
    await fs.appendFile(path.join(app, 'Contents/Resources/music.txt'), 'tampered');
    await assert.rejects(verifyMacosApp(app, identity), /resource|invalid|modified/i);
  } finally { await fs.rm(root, { recursive: true, force: true }); }
});
