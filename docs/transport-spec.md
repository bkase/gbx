Got it — here’s the **end-to-end, M1-scoped engineering doc** for **(3) SAB Rings & Transport** with the finalized choices: **multiple SABs**, **rkyv** serialization, and **Loom** verification. It’s concrete enough to implement directly and ties into the rest of the architecture.

---

# (3) SAB Rings & Transport — E2E Engineering Plan (M1, rkyv + Loom)

**Theme:** Make the boot sequence render on the Web with the *real* transport in place.
**Scope:** Transport only (Cmd/Evt/Frame/Audio lanes), ready to wire into Services/Reducers/Hub/Scheduler.

---

## 0) Objectives & Non-Goals

**Objectives**

* Lock-free **SPSC** transport identical across **Web** (SharedArrayBuffer + Atomics) and **Native** (mmap/aligned memory).
* Four lanes:

  * **CmdRing** (App → Service) — small controls
  * **EvtRing** (Service → App) — small reports
  * **FramePool** (Service → App) — fixed-size, zero-copy frames
  * **AudioPool** (Service → App) — fixed-size audio chunks
* **rkyv** for zero-copy message payloads (+ `bytecheck` in debug/CI).
* **Loom** to verify the atomic head/tail & index rings under adversarial interleavings.

**Non-Goals (M1)**

* MPMC rings, dynamic resizing, priority inside rings, GPU zero-copy into textures, remote/network transport.

---

## 1) Final Design Decisions

### 1.1 SAB allocation model

✅ **Multiple SABs (8 total)**:

* CmdRing SAB
* EvtRing SAB
* FramePool: `slots` SAB + `free_ring` SAB + `ready_ring` SAB
* AudioPool: `slots` SAB + `free_ring` SAB + `ready_ring` SAB

**Why:** Isolation, independent sizing, simpler fencing/ownership, fewer false-sharing pitfalls. Startup plumbing cost is trivial.

### 1.2 Serialization

✅ **rkyv** archived payloads in MsgRing records (+ `bytecheck` via `#[archive(check_bytes)]` in debug/CI).
ABI stability enforced with **golden archives** and an envelope **`ver`** byte.

### 1.3 Concurrency model

✅ Strict **SPSC** with Release/Acquire pairs only; no busy-waits.
Backpressure is explicit via `WouldBlock` → mapped to `SubmitOutcome`.

---

## 2) Data Structures & Layouts

### 2.1 MsgRing (Cmd/Evt) — one SAB each

```
Header (32B, 8B-aligned, 64B cacheline start)
  u32 capacity_bytes
  u32 head_bytes   // producer only
  u32 tail_bytes   // consumer only
  u32 flags_or_pad
  u64 magic = 0x4D534752494E4755 ('MSGRINGU')   // debug builds
  u64 _reserved
Data region (capacity_bytes, 8B-aligned records)
  record := [u32 total_len][u8 tag][u8 ver][u16 flags][rkyv_archived_bytes…][pad→8]
  if not enough room at end: write sentinel total_len=0xFFFF_FFFF, wrap to 0 next record
```

* **tag**: discriminant (e.g., `0x01=KernelCmd`, `0x11=KernelRep`, etc).
* **ver**: schema epoch for that tag; bump on breaking changes.
* **flags**: 16 bits reserved for future feature toggles or sequencing (defaults to 0).
* **Endianness:** little-endian (WASM and our native targets).

### 2.2 SlotPool (Frame/Audio) — three SABs per pool

* `slots` SAB: fixed-size slots, base aligned to 4 KiB (web) / page (native), each slot **64B-aligned**.

  * **Frame**: `slot_size = 128 KiB`, `N = 8` (≈1 MiB total)
  * **Audio**: `slot_size = 32 KiB`, `N = 16` (≈512 KiB total)
* `free_ring` SAB (SPSC ring of `u32` indices)
* `ready_ring` SAB (SPSC ring of `u32` indices)

**Index ring header (32B, per SAB):**

```
u32 capacity; u32 head; u32 tail; u32 pad;
u64 magic = 0x53504F4F4C465245 ('SPOOLFRE') // use 'SPOOLRDY' for ready
u64 _reserved
```

---

## 3) Atomics & Ordering

**MsgRing Producer**

1. Ensure space (or write wrap sentinel) →
2. write payload → write envelope (len/tag/ver) →
3. `head.store(new_head, Release)`.

**MsgRing Consumer**

1. `let head = head.load(Acquire)` →
2. if `head == tail` => `Empty`; else if envelope.len==`0xFFFF_FFFF` ⇒ set `tail=0` and continue;
3. read payload → `tail.store(new_tail, Release)` in `pop_advance()`.

**SlotPool Rings**

* Push: write idx → `head.store(Release)`
* Pop: `head.load(Acquire)` → read idx → `tail.store(Release)`

Web uses `Atomics.load/store` on `Int32Array`; native uses `Ordering::{Acquire,Release}`.

---

## 4) Rust API (final)

### 4.1 MsgRing (payload = rkyv bytes)

```rust
pub struct Envelope {
  pub tag: u8,
  pub ver: u8,
  pub flags: u16,
}

pub struct ProducerGrant<'a> { /* mutable slice + envelope */ }
pub struct Record<'a> { pub envelope: Envelope, pub payload: &'a [u8] }

impl MsgRing {
  /// Attempts to reserve `need` bytes of payload capacity.
  /// Returns `Some(ProducerGrant)` on success, `None` if the ring is full.
  pub fn try_reserve(&mut self, need: usize) -> Option<ProducerGrant<'_>>;
  pub fn try_reserve_with(
    &mut self,
    envelope: Envelope,
    need: usize,
  ) -> Option<ProducerGrant<'_>>;

  /// Returns the next record without advancing the consumer tail.
  pub fn consumer_peek(&self) -> Option<Record<'_>>;
  /// Advances the consumer tail past the record returned by `consumer_peek`.
  pub fn consumer_pop_advance(&mut self);
  /// Returns the envelope captured during the most recent peek (if any).
  pub fn consumer_last_envelope(&self) -> Option<Envelope>;
}
```

**rkyv usage (example):**

```rust
use rkyv::ser::{Serializer, serializers::WriteSerializer};
use std::io::Cursor;

#[derive(Default)]
struct CountingWriter {
  bytes: usize,
}

impl std::io::Write for CountingWriter {
  fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
    self.bytes += buf.len();
    Ok(buf.len())
  }

  fn flush(&mut self) -> std::io::Result<()> {
    Ok(())
  }
}

fn send_rep(rep: &KernelRep, ring: &mut MsgRing) {
  // Pass 1: measure archived size.
  let mut counting = CountingWriter::default();
  WriteSerializer::new(&mut counting).serialize_value(rep).unwrap();
  let need = counting.bytes;

  // Pass 2: write directly into the reserved payload.
  let mut grant = ring.try_reserve(need).expect("ring would block");
  let written = {
    let payload = grant.payload();
    let mut cursor = Cursor::new(payload);
    WriteSerializer::new(&mut cursor).serialize_value(rep).unwrap();
    cursor.position() as usize
  };
  assert_eq!(written, need);
  grant.commit(written);
}

fn recv_rep(record: Record<'_>) -> Option<rkyv::Archived<KernelRep>> {
  #[cfg(debug_assertions)]
  { rkyv::check_archived_root::<KernelRep>(record.payload).ok() }
  #[cfg(not(debug_assertions))]
  { Some(unsafe { rkyv::archived_root::<KernelRep>(record.payload) }) }
}
```

### 4.2 SlotPool

```rust
pub enum SlotPush { Ok, WouldBlock }
pub enum SlotPop  { Ok { slot_idx: u32 }, Empty }

pub struct SlotPool { /* slots + free + ready */ }

impl SlotPool {
  pub fn try_acquire_free(&mut self) -> Option<u32>;
  pub fn slot_mut(&mut self, idx: u32) -> &mut [u8]; // exact slot_size len
  pub fn push_ready(&mut self, idx: u32) -> SlotPush;

  pub fn pop_ready(&mut self) -> SlotPop;
  pub fn release_free(&mut self, idx: u32);
}
```

---

## 5) Sizing, Watermarks, Backpressure

* **CmdRing**: 32 KiB
* **EvtRing**: 512 KiB
* **Frame slots**: 8 × 128 KiB (GB frame RGBA8 = 92,160 B)
* **Audio slots**: 16 × 32 KiB (≈10 ms @ 44.1 kHz stereo f32)

**Backpressure mapping**

* `try_reserve` → `None` ⇒ service returns `SubmitOutcome::WouldBlock`.
* `try_acquire_free`/`push_ready` → `None/WouldBlock` ⇒ **BestEffort** work drops; **Must** work retried next rAF by scheduler; **Lossless** handled via P0-front requeue at Intent level.

---

## 6) Error Handling, Recovery, Versioning

* **Version skew** (envelope `ver` > supported): drop record + counter; no panic.
* **Corruption guards**: debug-only magic fields and optional CRC32 on Frame slots; drop + counter if mismatch.
* **Closed**: surfaced as `SubmitOutcome::Closed` by higher layers.
* **Schema stability**: golden archived fixtures in `tests/golden/*.bin` per tag. CI fails on byte diff unless `ver` bump + golden regen.

---

## 7) End-to-End Examples

### 7.1 Frame upload path (mock kernel → GPU)

**Producer (Kernel Worker):**

```rust
if let Some(idx) = frame_pool.try_acquire_free() {
  let buf = frame_pool.slot_mut(idx);
  write_frame_rgba_160x144(buf);
  if let SlotPush::Ok = frame_pool.push_ready(idx) {
    let rep = KernelRep::LaneFrame { group: 0, lane: display_lane, slot_idx: idx, frame_id };
    send_rep(&rep, &mut evt_ring);
  } else {
    frame_pool.release_free(idx); // BestEffort drop (thumbnails); display uses scheduler retry upstream
  }
}
```

**Consumer (App main, Phase B):**

```rust
while let Some(record) = evt_ring.consumer_peek() {
  if let Some(rep) = recv_rep(record) {
    match rep {
      r if matches!(r, KernelRep::LaneFrame{..}) => {
        // reduce_report → AvCmd::Gpu(UploadFrame{slot_idx})
        gpu.upload_from_slot(rep.slot_idx(), &frame_pool);
        frame_pool.release_free(rep.slot_idx());
      }
      _ => { /* other reports */ }
    }
  }
  evt_ring.consumer_pop_advance();
}
```

---

## 8) Testing & Verification (exact coverage)

### 8.1 Unit — MsgRing

1. **Single record**: push+pop equals.
2. **Exact-fit boundary**: end aligned fill; next push triggers sentinel; consumer wraps once.
3. **Wrap-pad sentinel**: forced non-fit, sentinel written/read, payload at 0.
4. **Alignment**: assert `record_start % 8 == 0` over 10k randomized writes.
5. **Var-len stress**: random sizes ≤ 2 KiB; 10k records; hash match.
6. **Backpressure**: fill ring until `try_reserve` returns `None`; confirm no unread overwrite.
7. **rkyv round-trip**: archive+check/decode KernelCmd/KernelRep/GpuCmd; fields equal.

### 8.2 Unit — SlotPool

1. **Lifecycle**: acquire N → ready N → pop N → release N; counts reconcile.
2. **Exhaustion**: acquire when 0 free → `None`; push_ready full → `WouldBlock`.
3. **FIFO**: indices come out in order under SPSC.
4. **Churn**: 100k cycles; no leaks; final free count == N.
5. **Frame invariants**: 64B alignment; stride 160×4; slot length 128 KiB.

### 8.3 Schema Stability (CI)

* Golden bytes for representative messages; CI diff fails without `ver` bump.

### 8.4 Loom (native, `--features loom`)

* **msg_ring_small_records()**: concurrent producer/consumer; verify no lost/torn records, tail≤head.
* **msg_ring_wrap_pad()**: sentinel interleavings; single wrap; next record at 0.
* **slot_pool_fifo()**: ready/free rings maintain FIFO and no early reuse.
* **slot_pool_exhaustion()**: `WouldBlock` appears whenever consumer lags per schedule.

### 8.5 E2E — Browser (WASM + worker; headless Chrome)

1. **Worker flood**: 10k frames; main releases exactly 10k slots; zero leaks.
2. **Bursty Evt**: bursts exceed drain budget; over time, all delivered (fair RR drain).
3. **Backpressure**: pause consumer; producer sees `WouldBlock`; resume recovers to steady-state.

### 8.6 E2E — Native (threads)

* Parity with browser tests; validates atomic semantics across platforms.

### 8.7 Policy Mapping (service mocks)

* For each command class (DisplayTick, ExplorationTick, GPU display/thumbnail, Audio, FS autosave/manual):

  * Force transport `WouldBlock` → assert `SubmitOutcome` matches policy.
  * For **Lossless**: scheduler test verifies **origin Intent requeued to P0-front**.

### 8.8 Chaos/Negatives

* **Version skew**: drop + counter.
* **Closed**: propagate `Closed`; main loop enters fatal path.
* **Debug corruption**: flip byte → drop + “corrupt” metric; no panic.

**Why this is enough:**

* SPSC reduces interleavings; Loom hits the atomic hotspots exhaustively.
* Unit tests nail wrap/sentinel/boundaries (historically fragile).
* rkyv goldens freeze ABI.
* Browser+native E2E validate real Atomics implementations.
* Policy mapping proves backpressure semantics visible at the app level.

---

## 9) Implementation Milestones & Ownership

| Milestone | Work                                                                  | Owner          |
| --------- | --------------------------------------------------------------------- | -------------- |
| **T0**    | `transport/` crate scaffolding; SAB alloc helpers; mmap impl skeleton | Systems        |
| **T1**    | MsgRing + rkyv integration + unit tests                               | Systems        |
| **T2**    | SlotPool (Frame + Audio) + unit tests                                 | Systems        |
| **T3**    | Loom tests (`msg_ring_*`, `slot_pool_*`)                              | Systems        |
| **T4**    | Golden archives + CI check (rkyv ABI)                                 | Infra          |
| **T5**    | Browser E2E (worker flood/burst/backpressure)                         | Frontend       |
| **T6**    | Native E2E (threads parity)                                           | Core           |
| **T7**    | Service mapping tests + scheduler hookup                              | Core + Systems |

**CI matrix:** macOS (arm64/x86_64), Linux (x86_64); wasm/headless Chrome job.

---

## 10) Bootstrap & Integration Contracts

* **Only Services touch transport.** App/store/hub see `SubmitOutcome`, `WorkCmd/AvCmd/Report` only.
* **Cmd/Evt tags:**

  * `0x01` KernelCmd, `0x02` FsCmd, `0x03` GpuCmd, `0x04` AudioCmd
  * `0x11` KernelRep, `0x12` FsRep, `0x13` GpuRep, `0x14` AudioRep
* **Version (`ver`)** starts at `1` for all tags.
* **Frame slot format:** RGBA8, tightly packed, `stride = 160*4`.
* **Audio slot format (M1):** interleaved f32 stereo, `sample_frames = 0.01 * sample_rate` (tunable later).

---

## 11) Performance Targets (M1)

| Path                      | Target                                                  |
| ------------------------- | ------------------------------------------------------- |
| MsgRing 64B msg R→W→R     | < 1 µs native / < 5 µs wasm                             |
| Frame push/pop (128 KiB)  | < 0.05 ms CPU overhead                                  |
| Evt drain fairness        | no service starved over 100 bursts                      |
| 120 Hz rAF                | < 1% deadline miss (with thumbnails disabled initially) |
| rkyv serialize small msgs | < 10 µs typical                                         |

---

## 12) Open Items (tracked, not blocking)

* Potential batch read for EvtRing if it profiles hot.
* WebGPU persistent/mapped staging to shave a copy (post-M1).
* Audio chunk size tuning after real audio path lands.

---

This doc is ready to drop into `transport/SPEC.md` (plus code stubs). If you want, I can also generate an initial PR skeleton with:

* the MsgRing and SlotPool types,
* rkyv encode/decode helpers,
* one Loom test,
* and the golden fixture harness wired into CI.
