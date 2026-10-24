import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { readFile, stat } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { reportFailure } from './ci-report.mjs';

const exec = promisify(execFile);
export const releaseAssets = ['xiaomi-music-windows-x64-setup.exe', 'xiaomi-music-macos-apple-silicon.dmg', 'xiaomi-music-macos-intel.dmg', 'SHA256SUMS'];
export async function publishRelease({ repo, sha, defaultBranch, runNumber, version, directory }, execute = async args => (await exec('gh', args)).stdout.trim()) {
  if (!/^[\w.-]+\/[\w.-]+$/.test(repo) || !/^[a-f0-9]{40}$/.test(sha) || !/^\d+$/.test(String(runNumber)) || !/^\d+\.\d+\.\d+$/.test(version) || !defaultBranch) throw new Error('Invalid release identity');
  for (const name of [...releaseAssets, 'release-notes.md']) {
    const file = await stat(path.join(directory, name));
    if (!file.isFile() || file.size === 0) throw new Error(`Missing staged asset: ${name}`);
  }
  const head = () => execute(['api', `repos/${repo}/commits/${encodeURIComponent(defaultBranch)}`, '--jq', '.sha']);
  if ((await head()).trim() !== sha) return { status: 'superseded' };
  const tag = `v${version}-build.${runNumber}`;
  let existing;
  try {
    existing = JSON.parse(await execute(['api', `repos/${repo}/releases/tags/${tag}`]));
  } catch (error) {
    // A network/auth error must not be mistaken for an absent release.
    if (!/HTTP 404\b/.test(error.stderr || '')) throw error;
  }
  if (existing) {
    if (existing.target_commitish !== sha) throw new Error('Release source mismatch');
    if (!existing.draft) {
      if (!releaseAssets.every(name => existing.assets?.some(asset => asset.name === name && asset.size > 0))) throw new Error('Incomplete published release');
      return { status: 'exists', tag };
    }
  } else {
    await execute(['release', 'create', tag, '--repo', repo, '--target', sha, '--title', `小米音乐投送 ${version} · Build ${runNumber}`, '--draft', '--notes-file', path.join(directory, 'release-notes.md')]);
  }
  await execute(['release', 'upload', tag, '--repo', repo, ...releaseAssets.map(name => path.join(directory, name)), '--clobber']);
  if ((await head()).trim() !== sha) return { status: 'superseded', tag };
  await execute(['release', 'edit', tag, '--repo', repo, '--draft=false', '--prerelease=false', '--latest']);
  return { status: 'published', tag, url: `https://github.com/${repo}/releases/tag/${tag}` };
}
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const { version } = JSON.parse(await readFile(new URL('../package.json', import.meta.url), 'utf8'));
    const result = await publishRelease({
      repo: process.env.GITHUB_REPOSITORY, sha: process.env.GITHUB_SHA,
      defaultBranch: process.env.DEFAULT_BRANCH, runNumber: process.env.GITHUB_RUN_NUMBER,
      version, directory: process.argv[2],
    });
    process.stdout.write(`${JSON.stringify(result)}\n`);
  } catch (error) { reportFailure('Release publication failed', error); }
}
