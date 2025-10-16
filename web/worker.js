const Op = {
  Init: 0,
  Flood: 1,
  Burst: 2,
  Backpressure: 3,
};

let exportsRef = null;
let readyPromise = null;
let sharedMemory = null;

function ensureEnvironmentReady(memory) {
  if (!self.crossOriginIsolated) {
    console.error("transport worker requires crossOriginIsolated=true");
    postMessage({ op: Op.Init, status: -90 });
    return false;
  }

  if (!(memory instanceof WebAssembly.Memory)) {
    console.error("transport worker expected WebAssembly.Memory import");
    postMessage({ op: Op.Init, status: -91 });
    return false;
  }

  const buffer = memory.buffer;
  if (!(buffer instanceof SharedArrayBuffer)) {
    console.error("transport worker memory must be backed by SharedArrayBuffer");
    postMessage({ op: Op.Init, status: -92 });
    return false;
  }

  return true;
}

self.addEventListener("message", (event) => {
  const data = event.data;
  switch (data.op >>> 0) {
    case Op.Init:
      handleInit(data);
      break;
    case Op.Flood:
      handleRun(Op.Flood, "worker_flood", data);
      break;
    case Op.Burst:
      handleRun(Op.Burst, "worker_burst", data);
      break;
    case Op.Backpressure:
      handleRun(Op.Backpressure, "worker_backpressure", data);
      break;
    default:
      postMessage({ op: data.op, status: -42 });
  }
});

function handleInit(data) {
  const descriptorPtr = data.descriptorPtr >>> 0;
  sharedMemory = data.memory;
  const moduleBytes = data.module;

  if (!ensureEnvironmentReady(sharedMemory)) {
    return;
  }

  readyPromise = instantiate(moduleBytes, sharedMemory)
    .then((exports) => exports.worker_init(descriptorPtr))
    .then((status) => {
      postMessage({ op: Op.Init, status });
    })
    .catch((err) => {
      console.error("transport worker init failed", err);
      postMessage({ op: Op.Init, status: -1 });
    });
}

function handleRun(op, exportName, data) {
  if (!readyPromise) {
    postMessage({ op, status: -3 });
    return;
  }
  const configPtr = data.configPtr >>> 0;
  const statsPtr = data.statsPtr >>> 0;
  readyPromise
    .then((status) => {
      if (typeof status === "number" && status !== 0) {
        return status;
      }
      const exports = exportsRef;
      const fn = exports && exports[exportName];
      if (typeof fn !== "function") {
        return -4;
      }
      return fn(configPtr, statsPtr);
    })
    .then((status) => {
      postMessage({ op, status: status | 0 });
    })
    .catch((err) => {
      console.error("transport worker run failed", err);
      postMessage({ op, status: -5 });
    });
}

async function instantiate(moduleBytes, memory) {
  if (exportsRef) {
    return exportsRef;
  }
  const source = toBufferSource(moduleBytes);
  const { instance } = await WebAssembly.instantiate(source, {
    env: { memory },
  });
  exportsRef = instance.exports;
  return exportsRef;
}

function toBufferSource(moduleBytes) {
  if (moduleBytes instanceof ArrayBuffer) {
    return moduleBytes;
  }
  if (ArrayBuffer.isView(moduleBytes)) {
    const { buffer, byteOffset, byteLength } = moduleBytes;
    return buffer.slice(byteOffset, byteOffset + byteLength);
  }
  throw new TypeError("unexpected module payload");
}
