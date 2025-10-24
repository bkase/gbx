#!/usr/bin/env node

import { readFile } from 'node:fs/promises';
import path from 'node:path';
import url from 'node:url';

const here = path.dirname(url.fileURLToPath(import.meta.url));
const pkgDir = path.resolve(here, '../web/pkg');

const {
  default: init,
  gbx_init,
  gbx_load_rom,
  gbx_tick,
  gbx_debug_state,
  gbx_consume_frame,
  fabric_worker_run,
} = await import(
  url.pathToFileURL(path.join(pkgDir, 'fabric_worker_wasm.js'))
);

const verbose = process.env.UI_BLARGG_VERBOSE === '1';
const maxTicks = Number.parseInt(process.env.UI_BLARGG_MAX_TICKS ?? '8192', 10);
if (!Number.isFinite(maxTicks) || maxTicks <= 0) {
  throw new Error(`UI_BLARGG_MAX_TICKS must be a positive integer, got ${process.env.UI_BLARGG_MAX_TICKS}`);
}
const progressInterval = Number.parseInt(process.env.UI_BLARGG_PROGRESS_INTERVAL ?? '120', 10);
const workerDrainLimit = Number.parseInt(process.env.UI_BLARGG_WORKER_PASSES ?? '8', 10);
const baseConsoleLog = console.log.bind(console);

if (!verbose) {
  console.log = (...args) => {
    if (args.length > 0 && typeof args[0] === 'string') {
      const first = args[0];
      if (first.startsWith('gbx_tick:')) {
        return;
      }
    }
    baseConsoleLog(...args);
  };
}

const log = (...args) => {
  baseConsoleLog(...args);
};

const debug = (...args) => {
  if (verbose) {
    baseConsoleLog(...args);
  }
};

function findVisibleDetail(pixelData) {
  if (!pixelData || pixelData.length < 8) {
    return null;
  }
  const r0 = pixelData[0];
  const g0 = pixelData[1];
  const b0 = pixelData[2];
  const a0 = pixelData[3];
  for (let i = 4; i < pixelData.length; i += 4) {
    const r = pixelData[i];
    const g = pixelData[i + 1];
    const b = pixelData[i + 2];
    const a = pixelData[i + 3];
    if (r !== r0 || g !== g0 || b !== b0 || a !== a0) {
      const pixelIndex = i / 4;
      return {
        pixelIndex,
        rgba: [r, g, b, a],
      };
    }
  }
  return null;
}

async function main() {
  const wasmBytes = await readFile(path.join(pkgDir, 'fabric_worker_wasm_bg.wasm'));
  await init(wasmBytes);

  await gbx_init();
  log(`initial fabric_worker_run = ${fabric_worker_run()}`);

  const override = process.env.WASM_ROM_PATH;
  const romPath = path.resolve(here, override ?? '../web/roms/tetris.gb');
  const romBytes = new Uint8Array(await readFile(romPath));
  await gbx_load_rom(romBytes);

  for (let tick = 0; tick < maxTicks; tick++) {
    const reports = gbx_tick(256);
    let totalWorkerPasses = 0;
    for (let pass = 0; pass < workerDrainLimit; pass++) {
      const work = fabric_worker_run();
      totalWorkerPasses += work;
      if (work === 0) {
        break;
      }
    }
    const state = gbx_debug_state();
    if (verbose) {
      debug(`tick ${tick}: reports=${reports.length} worker_passes=${totalWorkerPasses}`, state);
    } else if (tick % progressInterval === 0) {
      log(
        `[progress] tick=${tick}/${maxTicks} frame_id=${state.frame_id} reports=${reports.length} worker_passes=${totalWorkerPasses}`
      );
    }

    if (verbose) {
      for (const report of reports) {
        if (report.type !== 'Kernel.LaneFrame') {
          debug(`Report: ${report.type}`);
        }
      }
    }

    const frames = reports.filter((r) => r.type === 'Kernel.LaneFrame');
    for (const frame of frames) {
      const slotCount = typeof frame.slot_count === 'number' ? frame.slot_count : 1;
      let pixelData = frame.pixels ?? null;
      let width = frame.width || 160;
      let height = frame.height || 144;
      if (!pixelData && typeof frame.start_idx === 'number') {
        const consumed = gbx_consume_frame(frame.start_idx, slotCount, width, height);
        pixelData = consumed.pixels;
        width = consumed.width || width;
        height = consumed.height || height;
      }
      if (!pixelData) {
        continue;
      }
      const sample = Array.from(pixelData.slice(0, 16));
      debug(
        `LaneFrame frame_id=${frame.frame_id} lane=${frame.lane} width=${width} height=${height} len=${pixelData.length} pixels[0..16]=`,
        sample
      );
      const detail = findVisibleDetail(pixelData);
      if (detail) {
        log(
          `[detail] tick=${tick} frame_id=${frame.frame_id} lane=${frame.lane} pixel=${detail.pixelIndex} rgba=${detail.rgba.join(
            ','
          )} width=${width} height=${height}`
        );
        return;
      }
    }
  }

  throw new Error(`No LaneFrame with visible detail observed after ${maxTicks} ticks`);
}

main().catch((err) => {
  console.error(err);
  process.exitCode = 1;
});
