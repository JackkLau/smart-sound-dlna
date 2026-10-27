import { readdir, readFile, mkdir, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';
import { reportFailure } from './ci-report.mjs';

const targets = [
  ['x86_64-pc-windows-msvc', '.exe', 'xiaomi-music-windows-x64-setup.exe'],
  ['aarch64-apple-darwin', '.dmg', 'xiaomi-music-macos-apple-silicon.dmg'],
  ['x86_64-apple-darwin', '.dmg', 'xiaomi-music-macos-intel.dmg'],
];
async function filesIn(directory) {
  const result = [];
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const filename = path.join(directory, entry.name);
    if (entry.isDirectory()) result.push(...await filesIn(filename));
    else if (entry.isFile()) result.push(filename);
  }
  return result;
}
function validInstaller(bytes, extension) {
  if (extension === '.dmg') return bytes.length >= 512 && bytes.subarray(-512, -508).toString('ascii') === 'koly';
  if (bytes.length < 64 || bytes.subarray(0, 2).toString('ascii') !== 'MZ') return false;
  const offset = bytes.readUInt32LE(0x3c);
  return offset >= 64 && offset + 4 <= bytes.length && bytes.subarray(offset, offset + 4).equals(Buffer.from('PE\0\0'));
}
export async function prepareRelease(input, output, { version, sha, runUrl }) {
  if (!/^\d+\.\d+\.\d+$/.test(version) || !/^[0-9a-f]{40}$/.test(sha)) throw new Error('Invalid release identity');
  const packages = [];
  for (const [target, extension, name] of targets) {
    const files = (await filesIn(path.join(input, `xiaomi-music-${target}`))).filter(file => path.extname(file).toLowerCase() === extension);
    if (files.length !== 1) throw new Error(`Expected exactly one ${target} installer, got ${files.length}`);
    const bytes = await readFile(files[0]);
    if (!validInstaller(bytes, extension)) throw new Error(`Invalid installer: ${target}`);
    packages.push({ name, bytes, sha256: createHash('sha256').update(bytes).digest('hex') });
  }
  // Preflight all platforms before staging anything that could be published.
  await mkdir(output, { recursive: true });
  for (const item of packages) await writeFile(path.join(output, item.name), item.bytes, { flag: 'wx' });
  await writeFile(path.join(output, 'SHA256SUMS'), packages.map(item => `${item.sha256}  ${item.name}\n`).join(''), { flag: 'wx' });
  await writeFile(path.join(output, 'release-notes.md'), `小米音乐投送 ${version}\n\n- 修复音响时长为零或进度缺失时无法顺序切歌；清除音响残留的单曲循环模式。\n- 增加播放全部与下一曲提示；扫码手机页面支持曲库点歌、播放模式与音响控制。\n- 保留 macOS 完整临时签名与实际 DMG 校验。\n- Windows x64、macOS Apple Silicon 与 Intel 安装包均经过原生测试和构建。\n\n源码：${sha}\n构建记录：${runUrl}\n\nmacOS 使用完整的临时签名（Ad Hoc），尚未通过 Apple 公证；Windows 暂无发行证书签名。Mac 首次安装请先拖到“应用程序”；如提示无法验证开发者，可在“系统设置 → 隐私与安全性”检查并选择“仍要打开”。临时签名不等于 Apple 认证。\n`, { flag: 'wx' });
  return packages.map(({ name, sha256 }) => ({ name, sha256 }));
}
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
    const { version } = JSON.parse(await readFile(path.join(root, 'package.json'), 'utf8'));
    const tauri = JSON.parse(await readFile(path.join(root, 'src-tauri/tauri.conf.json'), 'utf8'));
    const cargo = await readFile(path.join(root, 'src-tauri/Cargo.toml'), 'utf8');
    if (tauri.version !== version || cargo.match(/^version = "([^"]+)"/m)?.[1] !== version) throw new Error('Application versions do not match');
    const result = await prepareRelease(process.argv[2], process.argv[3], {
      version, sha: process.env.GITHUB_SHA,
      runUrl: `https://github.com/${process.env.GITHUB_REPOSITORY}/actions/runs/${process.env.GITHUB_RUN_ID}`,
    });
    process.stdout.write(`Prepared ${result.length} native installers for ${version}\n`);
  } catch (error) { reportFailure('Installer validation failed', error); }
}
