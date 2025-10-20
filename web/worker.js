import init, {
  fabric_worker_init,
  fabric_worker_run,
  worker_register_services,
  worker_register_test
} from './transport_worker.js';

const Op = {
  Init: 0,
  RegisterTest: 1,
  RegisterServices: 2,
  Run: 3,
};

let initialized = false;

self.onmessage = async (event) => {
  const data = event.data;
  const op = data.op >>> 0;
  console.log(`Worker onmessage, op=${op}`);

  try {
    if (op === Op.Init) {
      const { memory, layoutPtr = 0, layoutLen = 0 } = data;

      // Initialize WASM module if needed
      if (!initialized) {
        console.log("Worker: calling await init(undefined, memory)");
        await init(undefined, memory);
        initialized = true;
        console.log("Worker: WASM module initialized!");
      }

      // Initialize fabric runtime (unified path for all scenarios)
      console.log(`Worker: Fabric init with layoutPtr=${layoutPtr}, layoutLen=${layoutLen}`);
      const status = fabric_worker_init(layoutPtr >>> 0, layoutLen >>> 0);

      console.log(`Worker: initialization status=${status}`);
      postMessage({ op, status });
    } else if (op === Op.RegisterTest) {
      // Register test scenario engine
      if (!initialized) {
        postMessage({ op, status: -3 });
        return;
      }

      const configPtr = data.configPtr >>> 0;
      const statsPtr = data.statsPtr >>> 0;
      const status = worker_register_test(configPtr, statsPtr);
      postMessage({ op, status });
    } else if (op === Op.RegisterServices) {
      if (!initialized) {
        postMessage({ op, status: -3 });
        return;
      }

      const { layoutPtr = 0, layoutLen = 0 } = data;
      const status = worker_register_services(layoutPtr >>> 0, layoutLen >>> 0);
      postMessage({ op, status });
    } else if (op === Op.Run) {
      // Run fabric worker tick
      if (!initialized) {
        postMessage({ op, status: -3 });
        return;
      }
      const work = fabric_worker_run();
      postMessage({ op, status: 0, work });
    } else {
      postMessage({ op, status: -42 });
    }
  } catch (err) {
    console.error("Worker error:", err);
    postMessage({ op, status: -99 });
  }
};
