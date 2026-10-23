import { spawn } from 'node:child_process';
import { existsSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const env = { ...process.env };
const cargoHome = path.join(root, '.tools', 'cargo');
let cargo = process.platform === 'win32' ? 'cargo.exe' : 'cargo';
if (existsSync(path.join(cargoHome, 'bin', cargo))) {
  cargo = path.join(cargoHome, 'bin', cargo);
  env.CARGO_HOME = cargoHome;
  env.RUSTUP_HOME = path.join(root, '.tools', 'rustup');
}
const child = spawn(cargo, process.argv.slice(2), { cwd: root, env, stdio: 'inherit' });
for (const signal of ['SIGINT', 'SIGTERM']) process.on(signal, () => child.kill(signal));
child.on('error', (error) => { console.error(error.message); process.exitCode = 1; });
child.on('exit', (code) => { process.exitCode = code ?? 1; });
