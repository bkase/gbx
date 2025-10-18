#!/usr/bin/env node
import http from 'http';
import path from 'path';
import fs from 'fs';
import { fileURLToPath } from 'url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

const PORT = Number(process.env.WASM_TEST_PORT || 4510);
const ROOT = path.resolve(__dirname, 'wasm');

const MIME = new Map([
  ['.html', 'text/html; charset=utf-8'],
  ['.js', 'application/javascript'],
  ['.wasm', 'application/wasm'],
  ['.json', 'application/json'],
]);

const server = http.createServer((req, res) => {
  const url = new URL(req.url, `http://localhost:${PORT}`);
  let filePath = url.pathname;
  if (filePath === '/' || filePath === '') {
    filePath = '/index.html';
  }
  const safePath = path.normalize(filePath).replace(/^\/+/, '');
  const dest = path.join(ROOT, safePath);

  if (!dest.startsWith(ROOT)) {
    res.writeHead(403);
    res.end('Forbidden');
    return;
  }

  fs.readFile(dest, (err, data) => {
    if (err) {
      res.writeHead(404);
      res.end('Not found');
      return;
    }

    const ext = path.extname(dest);
    const contentType = MIME.get(ext) || 'application/octet-stream';
    res.setHeader('Content-Type', contentType);
    res.setHeader('Cross-Origin-Opener-Policy', 'same-origin');
    res.setHeader('Cross-Origin-Embedder-Policy', 'require-corp');
    res.writeHead(200);
    res.end(data);
  });
});

server.listen(PORT, () => {
  console.log(`wasm test server listening on http://localhost:${PORT}`);
});

process.on('SIGINT', () => {
  server.close(() => process.exit(0));
});
