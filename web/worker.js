import init, { worker_init, worker_flood, worker_burst, worker_backpressure } from './transport_worker.js';

const Op = {
  Init: 0,
  Flood: 1,
  Burst: 2,
  Backpressure: 3,
};

let initialized = false;

self.onmessage = async (event) => {
  const data = event.data;
  const op = data.op >>> 0;
  console.log(`Worker onmessage, op=${op}`);

  try {
    if (op === Op.Init) {
      const { memory, descriptorPtr } = data;
      console.log(`Worker: Init op, initialized=${initialized}, descriptorPtr=${descriptorPtr}`);
      if (!initialized) {
        console.log("Worker: calling await init(undefined, memory)");
        await init(undefined, memory);
        initialized = true;
        console.log("Worker: init complete!");
      }
      console.log("Worker: calling worker_init");
      const status = worker_init(descriptorPtr >>> 0);
      console.log(`Worker: worker_init status=${status}`);
      postMessage({ op, status });
    } else {
      if (!initialized) {
        postMessage({ op, status: -3 });
        return;
      }

      const configPtr = data.configPtr >>> 0;
      const statsPtr = data.statsPtr >>> 0;

      let workerFn;
      switch (op) {
        case Op.Flood:
          workerFn = worker_flood;
          break;
        case Op.Burst:
          workerFn = worker_burst;
          break;
        case Op.Backpressure:
          workerFn = worker_backpressure;
          break;
        default:
          postMessage({ op, status: -42 });
          return;
      }

      const status = workerFn(configPtr, statsPtr);
      postMessage({ op, status });
    }
  } catch (err) {
    console.error("Worker error:", err);
    postMessage({ op, status: -99 });
  }
};
