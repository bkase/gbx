import init, { worker_init, worker_flood, worker_burst, worker_backpressure } from './pkg/transport_worker.js';

const Op = {
  Init: 0,
  Flood: 1,
  Burst: 2,
  Backpressure: 3,
};

let initialized = false;

self.addEventListener("message", async (event) => {
  const data = event.data;
  const op = data.op >>> 0;

  try {
    switch (op) {
      case Op.Init:
        await handleInit(data);
        break;
      case Op.Flood:
        handleRun(Op.Flood, worker_flood, data);
        break;
      case Op.Burst:
        handleRun(Op.Burst, worker_burst, data);
        break;
      case Op.Backpressure:
        handleRun(Op.Backpressure, worker_backpressure, data);
        break;
      default:
        postMessage({ op, status: -42 });
    }
  } catch (err) {
    console.error("Worker error:", err);
    postMessage({ op, status: -99 });
  }
});

async function handleInit(data) {
  const { memory, descriptorPtr } = data;

  if (!initialized) {
    await init(undefined, memory);
    initialized = true;
  }

  const status = worker_init(descriptorPtr >>> 0);
  postMessage({ op: Op.Init, status });
}

function handleRun(op, workerFn, data) {
  if (!initialized) {
    postMessage({ op, status: -3 });
    return;
  }

  const configPtr = data.configPtr >>> 0;
  const statsPtr = data.statsPtr >>> 0;

  const status = workerFn(configPtr, statsPtr);
  postMessage({ op, status });
}
