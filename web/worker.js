import init, {
  fabric_worker_init,
  fabric_worker_run,
  worker_init,
  worker_register_test
} from './transport_worker.js';

const Op = {
  Init: 0,
  RegisterTest: 1,
  Run: 2,
};

let initialized = false;

self.onmessage = async (event) => {
  const data = event.data;
  const op = data.op >>> 0;
  console.log(`Worker onmessage, op=${op}`);

  try {
    if (op === Op.Init) {
      const { memory, descriptorPtr, layoutPtr, layoutLen } = data;

      // Initialize WASM module if needed
      if (!initialized) {
        console.log("Worker: calling await init(undefined, memory)");
        await init(undefined, memory);
        initialized = true;
        console.log("Worker: WASM module initialized!");
      }

      // Initialize fabric runtime (unified path for all scenarios)
      let status;
      if (layoutPtr !== undefined && layoutLen !== undefined) {
        // New fabric path: init with FabricLayout
        console.log(`Worker: Fabric init with layoutPtr=${layoutPtr}, layoutLen=${layoutLen}`);
        status = fabric_worker_init(layoutPtr >>> 0, layoutLen >>> 0);
      } else if (descriptorPtr !== undefined) {
        // Legacy test path: init with WorkerInitDescriptor
        console.log(`Worker: Legacy init with descriptorPtr=${descriptorPtr}`);
        status = worker_init(descriptorPtr >>> 0);
      } else {
        // Empty fabric init
        console.log("Worker: Empty fabric init");
        status = fabric_worker_init(0, 0);
      }

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
