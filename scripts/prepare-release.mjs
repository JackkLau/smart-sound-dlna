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
  const notes = `小米音乐投送 ${version}

- 安装包内含 SQLite 电影片库。首次启动即可从「影视 → 浏览电影资源库」离线查找 1990 年以来豆瓣 8.0 分及以上的电影，再按需检索、确认和下载磁力版本。
- 在「导入音乐 → 磁力链接下载」打开窗口或切回客户端时，可从剪贴板识别有效链接。多条资源去重后供选择，点击「开始下载」才会添加任务。
- 通用磁力下载支持进度、暂停和续传，需要本机安装 aria2c。影视页仍可用内置下载功能获取影片。
- 在「影视」页选择本地 MP4、MKV 或 M2TS，可查看音轨和字幕轨并将影片直接投送到电视。
- 粘贴含 BTIH 的磁力链接，选择保存目录。下载完成后点击「载入影片」，即可选择电视播放。
- 内封字幕可标记为默认轨并保留原画质，也可合入画面输出 1080p。Windows x64 和 Apple Silicon 还支持将 HDR10 影片的 PGS 字幕合入原分辨率画面。默认字幕能否显示取决于电视播放器。

安装包附有 FFmpeg、FFprobe 和相应的许可说明。选择与你的电脑对应的安装包，可用同页的 SHA256SUMS 核对文件。macOS 版本使用 Ad Hoc 签名，尚未经过 Apple 公证。Windows 版本未使用发行证书签名。

源码提交 \`${sha}\` · [构建记录](${runUrl})
`;
  await writeFile(path.join(output, 'release-notes.md'), notes, { flag: 'wx' });
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
