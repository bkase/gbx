#!/usr/bin/env node
import http from 'http';
import path from 'path';
import fs from 'fs';
import { fileURLToPath } from 'url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

// Configuration from environment or CLI args
const PORT = Number(process.env.PORT || process.argv[3] || 8000);
const ROOT_DIR = process.argv[2] || path.resolve(__dirname, 'web');
const ROOT = path.resolve(__dirname, ROOT_DIR);

const MIME = new Map([
  ['.html', 'text/html; charset=utf-8'],
  ['.js', 'application/javascript'],
  ['.wasm', 'application/wasm'],
  ['.json', 'application/json'],
  ['.css', 'text/css'],
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
    // SharedArrayBuffer requires these headers
    res.setHeader('Cross-Origin-Opener-Policy', 'same-origin');
    res.setHeader('Cross-Origin-Embedder-Policy', 'require-corp');
    res.writeHead(200);
    res.end(data);
  });
});

server.listen(PORT, () => {
  console.log(`Serving ${ROOT}`);
  console.log(`Server listening on http://localhost:${PORT}`);
  console.log(`Press Ctrl+C to stop`);
});

process.on('SIGINT', () => {
  console.log('\nShutting down server...');
  server.close(() => process.exit(0));
});
