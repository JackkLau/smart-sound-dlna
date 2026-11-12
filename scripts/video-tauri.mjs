import { spawn } from 'node:child_process';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { hostTarget, prepareVideoTools } from './prepare-video-tools.mjs';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const [command, ...args] = process.argv.slice(2);
if (!['dev', 'build'].includes(command)) {
  console.error('使用 npm run dev:video 或 npm run build:video。');
  process.exit(1);
}
try {
  const index = args.indexOf('--target');
  const explicitTarget = args.find((argument) => argument.startsWith('--target='))?.slice(9);
  const requestedTarget = explicitTarget || (index >= 0 ? args[index + 1] : hostTarget());
  await prepareVideoTools(root, requestedTarget);
  const child = spawn(
    process.execPath,
    [
      path.join(root, 'scripts', 'tauri.mjs'),
      command,
      '--config',
      'src-tauri/tauri.video.conf.json',
      ...args,
    ],
    { cwd: root, env: { ...process.env, XIAOMI_DLNA_VIDEO: '1' }, stdio: 'inherit' },
  );
  for (const signal of ['SIGINT', 'SIGTERM']) process.on(signal, () => child.kill(signal));
  child.on('error', (error) => {
    console.error(error.message);
    process.exitCode = 1;
  });
  child.on('exit', (code) => {
    process.exitCode = code ?? 1;
  });
} catch (error) {
  console.error(error.message);
  process.exitCode = 1;
}
