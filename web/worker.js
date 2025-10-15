const ALIGN = 8;
const ENVELOPE_LEN = 8;
const HEADER_BYTES = 32;
const SENTINEL = 0xffffffff;
const SENTINEL_BYTES = 4;
const CAPACITY_IDX = 0;
const HEAD_IDX = 1;
const TAIL_IDX = 2;

const EVENT_ENVELOPE = Object.freeze({ tag: 0x13, ver: 1, flags: 0 });

let transportState = null;

self.onmessage = (event) => {
  const { type, memory, layout, config } = event.data || {};
  switch (type) {
    case "init":
      transportState = initialiseState(memory, layout, config);
      self.postMessage({ type: "ready" });
      break;
    case "flood":
      ensureState();
      postResult("flood", runFlood(config || {}));
      break;
    case "burst":
      ensureState();
      postResult("burst", runBurst(config || {}));
      break;
    case "backpressure":
      ensureState();
      postResult("backpressure", runBackpressure(config || {}));
      break;
    default:
      console.warn("worker received unknown message", event.data);
  }
};

function initialiseState(memory, layout, config = {}) {
  if (!memory) {
    throw new Error("worker init requires shared memory");
  }
  if (!layout) {
    throw new Error("worker init requires layout");
  }

  const buffer = memory;
  return {
    cmdRing: createMsgRing(buffer, layout.cmdRing),
    evtRing: createMsgRing(buffer, layout.evtRing),
    frameSlots: createUint8Region(buffer, layout.frameSlots),
    frameFree: createIndexRing(buffer, layout.frameFree),
    frameReady: createIndexRing(buffer, layout.frameReady),
    audioSlots: createUint8Region(buffer, layout.audioSlots),
    audioFree: createIndexRing(buffer, layout.audioFree),
    audioReady: createIndexRing(buffer, layout.audioReady),
    frameSlotSize: (config.frameSlotSize >>> 0) || 0,
    frameSlotCount: (config.frameSlotCount >>> 0) || 0,
    audioSlotSize: (config.audioSlotSize >>> 0) || 0,
    audioSlotCount: (config.audioSlotCount >>> 0) || 0,
  };
}

function ensureState() {
  if (!transportState) {
    throw new Error("transport worker not initialised");
  }
}

function postResult(scenario, stats) {
  self.postMessage({
    type: "done",
    scenario,
    ...stats,
  });
}

function createMsgRing(buffer, desc) {
  if (!desc || !desc.header || !desc.data) {
    throw new Error("msg ring layout missing regions");
  }
  const header = createInt32Region(buffer, desc.header);
  const data = new Uint8Array(
    buffer,
    desc.data.offset >>> 0,
    desc.data.length >>> 0
  );
  const capacity = (desc.capacity >>> 0) || 0;
  return { header, data, capacity };
}

function createIndexRing(buffer, desc) {
  if (!desc || !desc.header || !desc.entries) {
    throw new Error("index ring layout missing regions");
  }
  const header = createInt32Region(buffer, desc.header);
  const data = createInt32Region(buffer, desc.entries);
  const capacity = (desc.capacity >>> 0) || 0;
  return { header, data, capacity };
}

function createUint8Region(buffer, region) {
  if (!region) {
    throw new Error("region descriptor missing");
  }
  return new Uint8Array(buffer, region.offset >>> 0, region.length >>> 0);
}

function createInt32Region(buffer, region) {
  if (!region) {
    throw new Error("region descriptor missing");
  }
  const length = (region.length >>> 0) / 4;
  return new Int32Array(buffer, region.offset >>> 0, length >>> 0);
}

function runFlood(config) {
  const total = (config.frameCount >>> 0) || 0;
  const stats = { produced: 0, wouldBlockReady: 0, wouldBlockEvt: 0, freeWaits: 0 };
  for (let frameId = 0; frameId < total; frameId++) {
    produceFrame(frameId, stats);
  }
  return stats;
}

function runBurst(config) {
  const bursts = (config.bursts >>> 0) || 0;
  const burstSize = (config.burstSize >>> 0) || 0;
  const stats = { produced: 0, wouldBlockReady: 0, wouldBlockEvt: 0, freeWaits: 0 };
  for (let i = 0; i < bursts; i++) {
    for (let j = 0; j < burstSize; j++) {
      produceFrame(stats.produced, stats);
    }
  }
  return stats;
}

function runBackpressure(config) {
  const frames = (config.frames >>> 0) || 0;
  const stats = { produced: 0, wouldBlockReady: 0, wouldBlockEvt: 0, freeWaits: 0 };
  for (let frameId = 0; frameId < frames; frameId++) {
    produceFrame(frameId, stats);
  }
  return stats;
}

function produceFrame(frameId, stats) {
  const slotIdx = acquireFreeSlot(stats);
  writeFrame(slotIdx, frameId);
  pushReady(slotIdx, stats);
  pushEvent(frameId, slotIdx, stats);
  stats.produced++;
}

function acquireFreeSlot(stats) {
  const ring = transportState.frameFree;
  for (;;) {
    const head = Atomics.load(ring.header, HEAD_IDX) >>> 0;
    const tail = Atomics.load(ring.header, TAIL_IDX) >>> 0;
    if (tail !== head) {
      const index = (tail % ring.capacity) >>> 0;
      const value = ring.data[index] >>> 0;
      Atomics.store(ring.header, TAIL_IDX, (tail + 1) >>> 0);
      return value;
    }
    stats.freeWaits++;
    Atomics.wait(ring.header, HEAD_IDX, head, 1);
  }
}

function pushReady(slotIdx, stats) {
  const ring = transportState.frameReady;
  for (;;) {
    if (indexRingPush(ring, slotIdx)) {
      return;
    }
    stats.wouldBlockReady++;
    const tail = Atomics.load(ring.header, TAIL_IDX);
    Atomics.wait(ring.header, TAIL_IDX, tail, 1);
  }
}

function pushEvent(frameId, slotIdx, stats) {
  const ring = transportState.evtRing;
  const payload = new Uint8Array(8);
  writeU32(payload, 0, frameId >>> 0);
  writeU32(payload, 4, slotIdx >>> 0);
  const totalLen = ENVELOPE_LEN + payload.length;
  const recordLen = alignUp(totalLen, ALIGN);

  for (;;) {
    const head = Atomics.load(ring.header, HEAD_IDX) >>> 0;
    const tail = Atomics.load(ring.header, TAIL_IDX) >>> 0;
    const reservation = reserveOffset(ring, head, tail, recordLen);
    if (reservation) {
      const { offset, newHead } = reservation;
      writeRecord(ring, offset, payload, totalLen, recordLen);
      Atomics.store(ring.header, HEAD_IDX, newHead >>> 0);
      return;
    }
    stats.wouldBlockEvt++;
    Atomics.wait(ring.header, TAIL_IDX, tail, 1);
  }
}

function indexRingPush(ring, value) {
  const head = Atomics.load(ring.header, HEAD_IDX) >>> 0;
  const tail = Atomics.load(ring.header, TAIL_IDX) >>> 0;
  if ((head - tail) >>> 0 >= ring.capacity) {
    return false;
  }
  const index = (head % ring.capacity) >>> 0;
  ring.data[index] = value >>> 0;
  Atomics.store(ring.header, HEAD_IDX, (head + 1) >>> 0);
  return true;
}

function reserveOffset(ring, head, tail, recordLen) {
  const capacity = ring.capacity;
  let headPos = head;
  if (headPos >= capacity || tail >= capacity) {
    return null;
  }

  if (headPos >= tail) {
    const spaceAtEnd = capacity - headPos;
    if (spaceAtEnd >= recordLen) {
      let newHead = headPos + recordLen;
      if (newHead === capacity) {
        newHead = 0;
      }
      if (newHead === tail) {
        return null;
      }
      return { offset: headPos, newHead };
    }
    const spaceAtStart = tail;
    if (spaceAtStart <= recordLen) {
      return null;
    }
    if (spaceAtEnd < SENTINEL_BYTES) {
      return null;
    }
    emitSentinel(ring, headPos);
    headPos = 0;
    const newHead = recordLen;
    if (newHead === tail) {
      return null;
    }
    return { offset: headPos, newHead };
  }

  if (recordLen >= (tail - headPos)) {
    return null;
  }
  const newHead = headPos + recordLen;
  return { offset: headPos, newHead };
}

function emitSentinel(ring, offset) {
  writeU32(ring.data, offset, SENTINEL);
  const padEnd = offset + ENVELOPE_LEN;
  for (let i = offset + SENTINEL_BYTES; i < padEnd; i++) {
    ring.data[i] = 0;
  }
}

function writeRecord(ring, offset, payload, totalLen, recordLen) {
  writeU32(ring.data, offset, totalLen >>> 0);
  const envOffset = offset + 4;
  ring.data[envOffset] = EVENT_ENVELOPE.tag & 0xff;
  ring.data[envOffset + 1] = EVENT_ENVELOPE.ver & 0xff;
  ring.data[envOffset + 2] = EVENT_ENVELOPE.flags & 0xff;
  ring.data[envOffset + 3] = (EVENT_ENVELOPE.flags >>> 8) & 0xff;

  const payloadOffset = offset + ENVELOPE_LEN;
  ring.data.set(payload, payloadOffset);
  const payloadEnd = payloadOffset + payload.length;
  const recordEnd = offset + recordLen;
  for (let i = payloadEnd; i < recordEnd; i++) {
    ring.data[i] = 0;
  }
}

function writeFrame(slotIdx, frameId) {
  const slotSize = transportState.frameSlotSize || 0;
  if (slotSize === 0) {
    return;
  }
  const offset = slotIdx * slotSize;
  writeU32(transportState.frameSlots, offset, frameId >>> 0);
}

function writeU32(view, offset, value) {
  view[offset] = value & 0xff;
  view[offset + 1] = (value >>> 8) & 0xff;
  view[offset + 2] = (value >>> 16) & 0xff;
  view[offset + 3] = (value >>> 24) & 0xff;
}

function alignUp(value, align) {
  return (value + (align - 1)) & ~(align - 1);
}
