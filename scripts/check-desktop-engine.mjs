import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const root = fileURLToPath(new URL('../', import.meta.url));
const windows = process.platform === 'win32';
const python = path.join(root, '.venv', windows ? 'Scripts/python.exe' : 'bin/python');
const engine = path.join(root, 'python-engine/dist/video-engine', windows ? 'video-engine.exe' : 'video-engine');
const result = spawnSync(python, ['scripts/check_packaged_engine.py', engine], { cwd: root, stdio: 'inherit' });
if (result.error) console.error(`Engine preflight could not run: ${result.error.message}. Prepare the project .venv first.`);
process.exit(result.status ?? 1);
