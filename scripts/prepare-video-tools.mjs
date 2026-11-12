import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { copyFile, chmod, mkdir, readFile, writeFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const projectRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
export const toolManifest = JSON.parse(
  await readFile(new URL('./video-tools.json', import.meta.url), 'utf8'),
);

export function hostTarget(platform = process.platform, arch = process.arch) {
  const target = Object.entries(toolManifest.targets).find(
    ([, value]) => value.platform === platform && value.arch === arch,
  );
  if (!target) throw new Error('影视工具目前仅为 Apple Silicon、Intel Mac 和 Windows x64 提供。');
  return target[0];
}

export async function stageProbe({ packageDir, destination, target, spec }) {
  const metadata = JSON.parse(await readFile(path.join(packageDir, 'package.json'), 'utf8'));
  if (metadata.name !== spec.package || metadata.version !== spec.version) {
    throw new Error('FFprobe 包版本与固定清单不符。请使用 npm ci 恢复依赖。');
  }
  const source = path.join(packageDir, spec.file);
  const bytes = await readFile(source);
  const hash = createHash('sha256').update(bytes).digest('hex');
  if (hash !== spec.sha256) throw new Error('FFprobe 二进制校验失败，未生成打包文件。');
  await mkdir(destination, { recursive: true });
  const executable = path.join(
    destination,
    `ffprobe-${target}${spec.file.endsWith('.exe') ? '.exe' : ''}`,
  );
  await copyFile(source, executable);
  if (process.platform !== 'win32') await chmod(executable, 0o755);
  return executable;
}

export async function prepareVideoTools(root = projectRoot, requestedTarget = hostTarget()) {
  const target = hostTarget();
  if (requestedTarget !== target) {
    throw new Error('请在目标平台 runner 构建影视安装包，不能把当前平台工具标成其他架构。');
  }
  const spec = toolManifest.targets[target];
  const require = createRequire(path.join(root, 'package.json'));
  const installerFile = require.resolve(`${toolManifest.installer.name}/package.json`);
  const installer = JSON.parse(await readFile(installerFile, 'utf8'));
  if (installer.version !== toolManifest.installer.version) {
    throw new Error('FFprobe installer 版本不符，请运行 npm ci。');
  }
  const fromInstaller = createRequire(installerFile);
  const packageDir = path.dirname(fromInstaller.resolve(`${spec.package}/package.json`));
  const destination = path.join(root, 'src-tauri', 'binaries');
  const executable = await stageProbe({ packageDir, destination, target, spec });
  // Run only the hash-verified host executable. Never use a shell or PATH lookup.
  const version = execFileSync(executable, ['-version'], {
    encoding: 'utf8',
    timeout: 10000,
    maxBuffer: 1024 * 1024,
    windowsHide: true,
  });
  const license = execFileSync(executable, ['-L'], {
    encoding: 'utf8',
    timeout: 10000,
    maxBuffer: 1024 * 1024,
    windowsHide: true,
  });
  await writeFile(
    path.join(destination, 'ffprobe-manifest.json'),
    JSON.stringify(
      {
        target,
        package: spec.package,
        package_version: spec.version,
        source_binary_sha256: spec.sha256,
        version,
        license,
        source: 'https://github.com/SavageCore/node-ffprobe-installer',
      },
      null,
      2,
    ) + '\n',
  );
  return { executable, target, version: version.split('\n')[0] };
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const result = await prepareVideoTools();
    console.log(`影视探测工具已校验：${result.target}，${result.version}`);
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
