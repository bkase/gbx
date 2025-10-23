#!/usr/bin/env node
import fs from "node:fs";
import path from "node:path";
import process from "node:process";

async function main() {
  const target = process.argv[2] ?? "web/pkg/fabric_worker_wasm_bg.wasm";
  const wasmPath = path.resolve(target);

  if (!fs.existsSync(wasmPath)) {
    throw new Error(`WASM artifact not found at ${wasmPath}`);
  }

  const wasmBytes = fs.readFileSync(wasmPath);
  await WebAssembly.compile(wasmBytes);
  console.log(`✅ WebAssembly module compiled successfully (${wasmPath})`);
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
