#!/usr/bin/env node
// Wrapper - delegates to unified server.js
import { fileURLToPath } from 'url';
import path from 'path';
import { spawn } from 'child_process';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

const port = process.env.GBX_PORT || 8000;
const serverPath = path.resolve(__dirname, '..', 'server.js');
const rootDir = __dirname;

const proc = spawn('node', [serverPath, rootDir, port], {
  stdio: 'inherit',
  env: { ...process.env, PORT: port }
});

proc.on('exit', (code) => process.exit(code));
process.on('SIGINT', () => {
  proc.kill('SIGINT');
});
