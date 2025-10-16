# Game Boy Emulator Architecture

**SIMD kernel + Elm-style core with typed services**
**Version:** v0.5 — 2025-10-11

## 0) Goals

1. **Ultra-high performance**
   SIMD-wide emulation (default lanes = 16) across N workers; 120 Hz present; low-latency A/V.

2. **Elm clarity under concurrency**
   Unidirectional data flow; reducers are pure; side effects isolated behind typed services.

3. **Explicit backpressure**
   No callbacks; pull-based drains; **command-kind default policies**; visible `SubmitOutcome`s; services are **strictly non-blocking**.

4. **Testable “every inch”**
   Unit, integration, property, fuzz, determinism, latency, perf, and chaos testing.

5. **Cross-platform**
   Native (macOS Apple Silicon/Intel, Linux) and Web (Chrome + WebGPU + SharedArrayBuffer).

---

## 1) System Overview

```
rAF(120 Hz) / UI → Intent
  Phase A: reduce_intent(World, Intent) → [WorkCmd]
            └─ ServicesHub.try_submit_work(&cmd)   // uses cmd.default_policy()
  Phase B: ServicesHub.drain_all(shared_budget, rr) → [Report]
            └─ reduce_report(World, Report) → FollowUps
                 ├─ immediate [AvCmd]  → try_submit_av(&cmd)  // uses cmd.default_policy()
                 └─ deferred [Intent] → PRIORITY intent queues (P0>P1>P2)
```

- **World**: the single authoritative state (emulation/session control, UI settings, perf counters). **Only reducers mutate it.**
- **Two-phase model & latency note**: Reports can trigger **immediate A/V** in Phase B, but any **new work** (e.g., FS→Kernel) is **deferred as an Intent** for the **next frame**. This preserves simplicity and prevents re-entrant loops at the cost of **≤1 frame latency** for report-driven work.
- **Services**: typed, strictly non-blocking; return `SubmitOutcome`.
- **Hub**: routes commands and merges reports with a **shared, round-robin drain budget**.
- **Scheduler**: rAF-driven loop with **priority intent queues** and **explicit `SubmitOutcome` handling**.

---

## 2) Messages & Phases

### 2.1 Intent (UI/time/deferrals)

```rust
pub enum Intent {
  PumpFrame,
  TogglePause,
  SetSpeed(f32),
  LoadRom { bytes: Arc<[u8]> },
  SelectDisplayLane(u16),
  // … snapshots, devtools, etc.
}
```

### 2.2 Report (service facts)

```rust
pub enum Report {
  Kernel(KernelRep),
  Gpu(GpuRep),
  Audio(AudioRep),
  Fs(FsRep),
}
```

### 2.3 Commands (phase ability only — no policy args)

```rust
pub enum WorkCmd {
  Kernel(KernelCmd),
  Fs(FsCmd),
}
pub enum AvCmd {
  Gpu(GpuCmd),
  Audio(AudioCmd),
}
```

### 2.4 Follow-ups (from report reducer)

```rust
pub struct FollowUps {
  pub immediate_av: SmallVec<[AvCmd; 8]>,       // submit now
  pub deferred_intents: SmallVec<[Intent; 8]>,  // enqueue by priority
}
```

---

## 3) Backpressure Policies & Outcomes

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubmitPolicy {
  Must,        // never drop; scheduler will retry on next rAF (no busy-waiting)
  Coalesce,    // keep only newest (replace older pending)
  BestEffort,  // ok to drop when congested
  Lossless,    // enqueue in order; never coalesce/drop
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubmitOutcome {
  Accepted,
  Coalesced,   // accepted by replacing/merging older pending work
  Dropped,     // intentionally dropped per policy
  WouldBlock,  // no capacity; non-blocking call could not accept
  Closed,      // service shut down/unhealthy
}
```

**Context-dependent policy**: Some defaults require **World** context (e.g., whether a GPU upload is for the display lane). The scheduler supplies that context when calling `default_policy()`.

---

## 4) Services (typed, pull-based, non-blocking)

### 4.1 Trait

```rust
pub trait Service {
  type Cmd: Send + 'static;
  type Rep: Send + 'static;

  /// Strictly non-blocking. Interprets command per its default policy.
  fn try_submit(&self, cmd: &Self::Cmd) -> SubmitOutcome;

  /// Strictly non-blocking. Drain up to `max` reports and RETURN them.
  fn drain(&self, max: usize) -> SmallVec<[Self::Rep; 8]>;
}
```

> **Who owns waiting?** **Scheduler only.** Services never block; they return `WouldBlock`.

### 4.2 Per-service commands & reports

```rust
pub enum TickPurpose { Display, Exploration }

pub enum KernelCmd {
  Tick { group: u16, budget: u32, purpose: TickPurpose },
  LoadRom { group: u16, rom: Arc<[u8]> },
  SetInputs { group: u16, lanes_mask: u32, joymask: u8 },
  Terminate { group: u16 },
}
pub enum KernelRep {
  TickDone { group: u16, lanes_mask: u32, cycles_done: u32 },
  LaneFrame { group: u16, lane: u16, span: FrameSpan, frame_id: u64 },
  AudioReady { group: u16, span: AudioSpan },
  DroppedThumb { group: u16, count: u32 },
}

pub enum GpuCmd { UploadFrame { lane: u16, span: FrameSpan } }
pub enum GpuRep { FrameShown { frame_id: u64 } }

pub enum AudioCmd { Submit { span: AudioSpan } }
pub enum AudioRep { Underrun }

pub enum FsCmd { Persist { path: PathBuf, bytes: Arc<[u8]> } }
pub enum FsRep { Saved { path: PathBuf, ok: bool } }
```

### 4.3 Command default policies

```rust
impl WorkCmd {
  pub fn default_policy(&self) -> SubmitPolicy {
    match self {
      WorkCmd::Kernel(KernelCmd::Tick{ purpose: TickPurpose::Display, .. }) => SubmitPolicy::Coalesce,
      WorkCmd::Kernel(KernelCmd::Tick{ purpose: TickPurpose::Exploration, .. }) => SubmitPolicy::BestEffort,
      WorkCmd::Kernel(_) => SubmitPolicy::Lossless,   // LoadRom, SetInputs, Terminate
      WorkCmd::Fs(FsCmd::Persist{..}) => SubmitPolicy::Coalesce, // autosaves
    }
  }
}
impl AvCmd {
  pub fn default_policy(&self, display_lane: u16) -> SubmitPolicy {
    match self {
      AvCmd::Gpu(GpuCmd::UploadFrame { lane, .. }) =>
        if *lane == display_lane { SubmitPolicy::Must } else { SubmitPolicy::BestEffort },
      AvCmd::Audio(_) => SubmitPolicy::Must,
    }
  }
}
```

### 4.4 ServicesHub (round-robin shared drain)

```rust
pub struct ServicesHub {
  pub kernel: Arc<dyn Service<Cmd=KernelCmd, Rep=KernelRep> + Send + Sync>,
  pub gpu:    Arc<dyn Service<Cmd=GpuCmd,    Rep=GpuRep>    + Send + Sync>,
  pub audio:  Arc<dyn Service<Cmd=AudioCmd,  Rep=AudioRep>  + Send + Sync>,
  pub fs:     Arc<dyn Service<Cmd=FsCmd,     Rep=FsRep>     + Send + Sync>,
}

impl ServicesHub {
  pub fn try_submit_work(&self, c: &WorkCmd) -> SubmitOutcome {
    match c {
      WorkCmd::Kernel(k) => self.kernel.try_submit(k),
      WorkCmd::Fs(f)     => self.fs    .try_submit(f),
    }
  }
  pub fn try_submit_av(&self, c: &AvCmd) -> SubmitOutcome {
    match c {
      AvCmd::Gpu(g)   => self.gpu  .try_submit(g),
      AvCmd::Audio(a) => self.audio.try_submit(a),
    }
  }

  /// Fair, round-robin draining with a single shared budget.
  pub fn drain_all_rr(&self, max_total: usize) -> SmallVec<[Report; 32]> {
    use Report::*;
    let mut out = smallvec::SmallVec::new();
    let mut budget = max_total;

    // Drain in RR cycles to avoid starvation.
    while budget > 0 {
      let before = out.len();
      for pull in [
        || self.audio.drain(1).into_iter().map(Audio).collect::<SmallVec<[_; 2]>>(),
        || self.kernel.drain(1).into_iter().map(Kernel).collect::<SmallVec<[_; 2]>>(),
        || self.gpu.drain(1).into_iter().map(Gpu).collect::<SmallVec<[_; 2]>>(),
        || self.fs.drain(1).into_iter().map(Fs).collect::<SmallVec<[_; 2]>>(),
      ] {
        if budget == 0 { break; }
        let batch = pull();
        if !batch.is_empty() {
          budget = budget.saturating_sub(batch.len());
          out.extend(batch);
          if budget == 0 { break; }
        }
      }
      if out.len() == before { break; } // all empty: done
    }
    out
  }
}
```

---

## 5) Reducers (type-safe phase split)

```rust
pub trait IntentReducer {
  fn reduce_intent(&mut self, intent: Intent) -> SmallVec<[WorkCmd; 8]>;
}
pub trait ReportReducer {
  fn reduce_report(&mut self, rep: Report) -> FollowUps;
}
```

**Intent examples**

- `PumpFrame` → `WorkCmd::Kernel(Tick{ purpose: Display, … })`
- Background exploration enqueue → `WorkCmd::Kernel(Tick{ purpose: Exploration, … })`
- `LoadRom` → `WorkCmd::Kernel(LoadRom{…})` (+ optional `Fs::Persist` autosave)

**Report examples**

- `Kernel::LaneFrame` → `AvCmd::Gpu(UploadFrame)` **immediate**
- `Kernel::TickDone` → **defer** `Intent::PumpFrame` (autopump)
- `Audio::Underrun` → metrics update

---

## 6) Concrete default policies (per command kind)

| Command Kind                          | **Default Policy** | Handling notes                                                                                       |
| ------------------------------------- | ------------------ | ---------------------------------------------------------------------------------------------------- |
| Kernel Tick (Display)                 | `Coalesce`         | Replace older pending tick; if `WouldBlock` (no queue), do nothing now — next rAF will submit again. |
| Kernel Tick (Exploration)             | `BestEffort`       | Drop under pressure; never blocks.                                                                   |
| Kernel non-tick (LoadRom, SetInputs…) | `Lossless`         | Must enqueue; see scheduler rule for `(Lossless, WouldBlock)`.                                       |
| GPU Upload (display lane)             | `Must`             | Scheduler **does not wait**; if `WouldBlock`, mark health and retry next rAF.                        |
| GPU Upload (thumbnail)                | `BestEffort`       | Drop/downsample; never blocks.                                                                       |
| Audio Submit (display lane)           | `Must`             | Service may coalesce small chunks or pad silence; emits `Audio::Underrun` if padded.                 |
| FS Persist (autosave)                 | `Coalesce`         | Replace older autosave; never blocks main loop.                                                      |
| FS Persist (manual save)              | `Lossless`         | Enqueue in service; preserve order.                                                                  |

---

## 7) Scheduler (rAF, priority intents, explicit outcomes)

### 7.1 Priority intent queues

Three deques: **P0** (UI & critical), **P1** (cadence/autopump), **P2** (background). Pull order is **P0 → P1 → P2**.

### 7.2 Count-based intent budget

- `intent_pull_budget = 1 + defer_count_budget` (e.g., 3)
- Always process exactly one cadence intent (`PumpFrame`) per rAF and then up to `defer_count_budget` additional intents by priority.
- No time-based budget in the loop — **count-based only** (clear and deterministic).

### 7.3 Health monitoring & recovery

- `gpu_blocked`: set when a display-lane `Must` upload returns `WouldBlock`. **Mitigation for next N frames (e.g., 10)**:
  - skip all `BestEffort` GPU uploads (thumbnails),
  - keep A/V `Must` only.
    Reset when a display-lane `Must` upload is `Accepted/Coalesced`.

- `service_pressure`: set on sustained `WouldBlock` from non-A/V services; may reduce exploration enqueues. Reset after a clean frame with no `WouldBlock`.
- `fatal`: set on `Closed`; halts ticking and surfaces error; triggers restart flow (below).

### 7.4 rAF loop (pseudocode with explicit outcomes)

```rust
pub struct App {
  pub world: World,
  pub hub: ServicesHub,
  pub intents: PQueues<Intent>,
  pub report_budget: usize,        // e.g., 32
  pub intent_budget: usize,        // e.g., 3
  pub health: HealthFlags,         // gpu_blocked, service_pressure, fatal
  pub stall_relief_frames: u8,     // countdown for mitigation window
}

impl App {
  pub fn raf_tick(&mut self, ui_intents: impl IntoIterator<Item = Intent>) {
    for intent in ui_intents {
      self.intents.enqueue(IntentPriority::P0, intent);
    }
    self.intents.enqueue(IntentPriority::P1, Intent::PumpFrame);

    let mut pulled = 0;
    while pulled < self.intent_budget {
      let Some(intent) = self.intents.pop_next() else { break; };
      let mut needs_retry = false;

      for work in self.world.reduce_intent(intent.clone()) {
        let policy = work.default_policy();
        let outcome = self.hub.try_submit_work(work.clone());

        if matches!(
          (policy, outcome),
          (SubmitPolicy::Lossless, SubmitOutcome::WouldBlock | SubmitOutcome::Closed)
        ) {
          needs_retry = true;
          self.health.service_pressure = true;
          break;
        }
      }

      if needs_retry {
        self.intents.enqueue_front_p0(intent);
      }

      pulled += 1;
    }

    for report in self.hub.drain_reports(self.report_budget) {
      let follow_ups = self.world.reduce_report(report);

      for av in follow_ups.immediate_av {
        let policy = av.default_policy(self.world.display_lane);
        if self.health.gpu_blocked && matches!(policy, SubmitPolicy::BestEffort) {
          continue;
        }

        let outcome = self.hub.try_submit_av(av.clone());
        match (policy, outcome) {
          (SubmitPolicy::Must, SubmitOutcome::WouldBlock) => {
            self.health.gpu_blocked = true;
            self.stall_relief_frames = self.stall_relief_frames.max(10);
          }
          (SubmitPolicy::Must, SubmitOutcome::Accepted | SubmitOutcome::Coalesced) => {
            self.health.gpu_blocked = false;
            if self.stall_relief_frames > 0 {
              self.stall_relief_frames -= 1;
            }
          }
          (_, SubmitOutcome::Closed) => {
            self.health.fatal = true;
            return;
          }
          _ => {}
        }
      }

      for (priority, intent) in follow_ups.deferred_intents {
        self.intents.enqueue(priority, intent);
      }
    }

    if self.stall_relief_frames > 0 && !self.health.gpu_blocked {
      self.stall_relief_frames -= 1;
    }
  }
}

pub struct HealthFlags {
  pub gpu_blocked: bool,
  pub service_pressure: bool,
  pub fatal: bool,
}
```

---

## 8) Concurrency & Transport

- **Native**: SPSC rings or `crossbeam_channel`; each service may be a thread; queues and atomics are internal to services.
- **Web**: SharedArrayBuffer SPSC rings; Workers for kernel groups; main thread stays pull-based; Workers may block with `Atomics.wait`.
- Store/hub layers **never** see queues or atomics.

---

## 9) Testability (deep & exhaustive)

**Everything is testable.** Highlights:

### 9.1 Reducers (pure)

- Unit/table tests for each `Intent`/`Report` → exact `World` deltas, emitted commands, and deferrals.
- Properties: budget monotonicity vs `SetSpeed`; toggle idempotence; display lane bounds.
- Sequences: scripted flows → final `World` + multisets of commands/deferrals.

### 9.2 Services (policy & transport)

- Mock services implementing `Service` with in-memory queues; knobs to force `WouldBlock`, `Closed`, `Coalesced`, etc.
- Backpressure:
  - `Coalesce`: two `Tick(Display)` → only latest queued; `Coalesced` observed.
  - `BestEffort`: thumbnails drop when full.
  - `Lossless`: `(Lossless, WouldBlock)` → verify **origin Intent is re-queued P0-front**; eventual `Accepted`.

- Drain never blocks; returns ≤ `max`.

### 9.3 SAB/Rings (web)

- Wrap-around & alignment invariants; Release/Acquire fences.
- Watermarks: display lane never dropped; thumbnails first to drop.
- Fuzz record sizes/bursts; no unread overwrite.

### 9.4 Integration (no threads)

- Deterministic fake rAF (120 Hz) with mocks.
- Assertions:
  - `LaneFrame` ⇒ `Gpu::UploadFrame` **same tick**.
  - `TickDone` ⇒ **deferred** `Intent::PumpFrame`.
  - Priority: P0 preempts P1/P2 within count budget.
  - Round-robin drain prevents starvation.

### 9.5 Determinism & differential

- Scalar vs SIMD: same ROM/steps → equal state snapshots.
- Native vs WASM: index-frame hashes within tolerance; audio chunk hashes.
- Save/restore harness: K→snapshot→M→restore→M → equal.

### 9.6 Fuzzing

- Reducer fuzz over `Intent|Report` grammar; invariants (no panic; bounded queues; world sanity).
- Transport fuzz: random ring sizes/timings; outcomes coherent; no deadlock.

### 9.7 Latency & pacing

- Verify `LaneFrame → UploadFrame` ≤ 1 frame.
- Under GPU pressure: display `Must` retried next rAF; thumbnails dropped.

### 9.8 Perf & regressions

- `criterion` benches: kernel `step_budget` throughput; scheduler overhead per tick.
- CI gates: fail on ≥5% regression per platform.

### 9.9 Observability

- Feature `testhooks`: per-command `SubmitOutcome` counters; queue depths; stall flags; drain shares.
- Structured debug logs: Intent→Work submit, Report→AV submit, outcomes.

### 9.10 Chaos / Resilience

- Force `Closed` mid-run; scheduler sets `fatal` and halts gracefully; logs intact.
- **Restart/re-sync plan**: Orchestrator (outside rAF) spawns a new service instance; `World` emits a **re-sync sequence** on next tick:
  - Kernel: `LoadRom`, `SetInputs`, and optional `ReinitializeFromState{snapshot}` (if snapshotting enabled).
  - GPU/Audio: recreate pipelines/buffers as needed.
  - FS: no re-sync needed unless an in-flight persist must be retried (`Lossless` intents ensure retry).

- Tests assert that after restart the system returns to a healthy tick/present loop without corruption.

---

## 10) Invariants

1. **Only reducers mutate `World`.**
2. Reducers are **side-effect free**.
3. Services are **strictly non-blocking**; `try_submit` may return `WouldBlock`, never waits.
4. Every command kind defines a **default SubmitPolicy** (`default_policy()`); scheduler reads it, services do not accept overrides.
5. rAF loop is **bounded**: count-based intent budget + shared round-robin drain budget.
6. Display A/V path uses `Must`; thumbnails use `BestEffort`.
7. `(Lossless, WouldBlock)` → **requeue origin Intent to P0-front** immediately.
8. No callbacks; all drains are **pull**.

---

## 11) Example Flows

### Display frame (1×, 120 Hz, frame-doubling)

```
rAF → enqueue Intent::PumpFrame (P1). A previous TickDone may also have deferred PumpFrame.
Both result in at most one effective Kernel::Tick(Display) due to Coalesce default.
Phase A: reduce_intent → WorkCmd::Kernel(Tick{Display}) → try_submit_work
Kernel → Report::Kernel(LaneFrame)
Phase B: reduce_report → AvCmd::Gpu(UploadFrame for display lane) → try_submit_av (Must)
GPU accepts → frame shown same tick
```

### Manual save (lossless)

```
Intent::LoadRom/Save → WorkCmd::Fs(Persist) default=Coalesce (autosave) or Lossless (manual)
If Lossless → (WouldBlock) => scheduler requeues the origin Intent at P0-front; retry next rAF.
Fs → Report::Saved{ok:true} → reduce_report updates World/UI
```

### Exploration tick (background)

```
Intent(P2) → WorkCmd::Kernel(Tick{Exploration}) default=BestEffort
If congested → Dropped; UI/A/V never blocked
```

---

## 12) Code Layout

```
crates/
  world/          // World + reducers (intent/report) + follow-ups
  hub/            // ServicesHub + Service trait + SubmitOutcome
  services/
    kernel/       // native thread + web worker implementations
    gpu/
    audio/
    fs/
  transport/      // rings: native SPSC + web SAB
  app/            // rAF scheduler + priority intent queues + health/recovery
  mock/           // mock services + fake clocks + chaos hooks
  tests/          // unit, integration, property, fuzz, perf, chaos
```

---

## 13) Defaults & Tuning

- **SIMD lanes/group**: 16
- **Workers**: `min(hw_threads - 2, 8)`
- **Present**: 120 Hz; frame-doubling at 1×; true 120 at 2×
- **Budgets**:
  - `intent_pull_budget = 3` (1 cadence + up to 2 deferrals)
  - `report_budget = 32` total (shared, round-robin)

- **Rings/worker**: Cmd 32 KB; Evt 512 KB; Frame 256 KB (single) or 2 MB (mosaic); Audio 128 KB

---

### Appendix A — Reducer Snippets (final names & defaults)

```rust
// Intent reducer
impl IntentReducer for World {
  fn reduce_intent(&mut self, intent: Intent) -> SmallVec<[WorkCmd; 8]> {
    use WorkCmd::*;
    let mut out = smallvec::SmallVec::new();
    match intent {
      Intent::PumpFrame if self.rom_loaded && !self.paused => {
        let budget = (70_224.0 * self.speed).round() as u32;
        out.push(Kernel(KernelCmd::Tick { group: 0, budget, purpose: TickPurpose::Display }));
      }
      // Example: background exploration enqueue (P2)
      // Intent::Explore => out.push(Kernel(KernelCmd::Tick { group: 0, budget: EXP_BUDGET, purpose: TickPurpose::Exploration })),

      Intent::LoadRom { bytes } => {
        self.rom_loaded = true;
        out.push(Kernel(KernelCmd::LoadRom { group: 0, rom: bytes }));
      }
      Intent::SetSpeed(x) => self.speed = x.clamp(0.1, 10.0),
      Intent::TogglePause => self.paused = !self.paused,
      Intent::SelectDisplayLane(l) => self.display_lane = l,
      _ => {}
    }
    out
  }
}

// Report reducer
impl ReportReducer for World {
  fn reduce_report(&mut self, rep: Report) -> FollowUps {
    use Report::*;
    let mut fu = FollowUps { immediate_av: smallvec::SmallVec::new(),
                             deferred_intents: smallvec::SmallVec::new() };
    match rep {
      Kernel(KernelRep::LaneFrame { lane, span, frame_id, .. }) => {
        if lane == self.display_lane {
          self.frame_id = frame_id;
          fu.immediate_av.push(AvCmd::Gpu(GpuCmd::UploadFrame { lane, span }));
        }
      }
      Kernel(KernelRep::TickDone { .. }) => {
        if self.auto_pump { fu.deferred_intents.push(Intent::PumpFrame); }
      }
      Audio(AudioRep::Underrun) => self.audio_underruns += 1,
      Fs(FsRep::Saved { ok, .. }) => self.last_save_ok = ok,
      _ => {}
    }
    fu
  }
}
```
