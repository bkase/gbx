#![cfg(all(test, not(target_arch = "wasm32")))]
//! Integration test that records NDJSON inspector snapshots and compares them to goldens.
use app::Scheduler;
use hub::{IntentPriority, ServicesHub};
use mock::make_hub;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use testdata::bytes as rom_bytes;
use world::{types::MemSpace, Intent, World};

const ROM_PATH: &str = "blargg/cpu_instrs/individual/03-op sp,hl.gb";
const GOLDEN_PATH: &str = "tests/golden/ndjson/inspector_blargg03.ndjson";

fn scheduler() -> (Scheduler, Arc<[u8]>) {
    let world = World::new();
    let hub: ServicesHub = make_hub();
    let mut scheduler = Scheduler::new(world, hub);
    scheduler.world_mut().set_auto_pump(false);
    let rom = rom_bytes(ROM_PATH);
    (scheduler, rom)
}

fn capture_ndjson(scheduler: &mut Scheduler, intent: Intent) -> String {
    let priority = intent.priority();
    scheduler.enqueue_intent(priority, intent);
    scheduler.run_once();
    scheduler
        .world()
        .inspector
        .vm
        .to_ndjson_line()
        .expect("serialize inspector vm")
}

#[allow(clippy::vec_init_then_push)]
fn record_script() -> Vec<String> {
    let (mut scheduler, rom) = scheduler();
    scheduler.enqueue_intent(
        IntentPriority::P0,
        Intent::LoadRom {
            group: 0,
            bytes: Arc::clone(&rom),
        },
    );
    scheduler.run_once();

    let mut lines = Vec::with_capacity(5);
    lines.push(capture_ndjson(&mut scheduler, Intent::DebugSnapshot(0)));
    lines.push(capture_ndjson(
        &mut scheduler,
        Intent::DebugStepInstruction { group: 0, count: 1 },
    ));
    lines.push(capture_ndjson(
        &mut scheduler,
        Intent::DebugStepInstruction { group: 0, count: 1 },
    ));
    lines.push(capture_ndjson(
        &mut scheduler,
        Intent::DebugMem {
            group: 0,
            space: MemSpace::Vram,
            base: 0x8000,
            len: 0x20,
        },
    ));
    lines.push(capture_ndjson(&mut scheduler, Intent::DebugStepFrame(0)));
    lines
}

#[test]
fn inspector_ndjson_matches_golden() {
    let actual = record_script().join("");
    let golden_path: PathBuf = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(GOLDEN_PATH);

    if std::env::var("UPDATE_GOLDEN").as_deref() == Ok("1") {
        fs::create_dir_all(golden_path.parent().unwrap()).expect("create golden directory");
        fs::write(&golden_path, &actual).expect("write golden fixture");
    }

    let expected = fs::read_to_string(&golden_path).expect("read golden ndjson");
    assert_eq!(actual, expected, "inspector NDJSON differs from golden");
}
