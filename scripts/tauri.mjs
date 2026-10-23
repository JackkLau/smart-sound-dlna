import { spawn } from 'node:child_process';
import { existsSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const env = { ...process.env };
const cargoHome = path.join(root, '.tools', 'cargo');
if (existsSync(path.join(cargoHome, 'bin', process.platform === 'win32' ? 'cargo.exe' : 'cargo'))) {
  env.CARGO_HOME = cargoHome;
  env.RUSTUP_HOME = path.join(root, '.tools', 'rustup');
  env.PATH = path.join(cargoHome, 'bin') + path.delimiter + (env.PATH || '');
}
const cli = path.join(root, 'node_modules', '@tauri-apps', 'cli', 'tauri.js');
if (!existsSync(cli)) {
  console.error('请先运行 npm ci 安装 Tauri 构建工具。');
  process.exit(1);
}
const child = spawn(process.execPath, [cli, ...process.argv.slice(2)], { cwd: root, env, stdio: 'inherit' });
for (const signal of ['SIGINT', 'SIGTERM']) process.on(signal, () => child.kill(signal));
child.on('error', (error) => { console.error(error.message); process.exitCode = 1; });
child.on('exit', (code) => { process.exitCode = code ?? 1; });
