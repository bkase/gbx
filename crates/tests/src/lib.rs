use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen_test]
fn wasm_smoke_test() {
    // Basic smoke test to ensure wasm builds work
    let sum: i32 = [1, 1].iter().copied().sum();
    assert_eq!(sum, 2);
}

#[cfg(test)]
mod tests {
    use app::Scheduler;
    use hub::{Intent, IntentPriority};
    use mock::{make_hub, make_hub_with_capacities};
    use std::sync::Arc;
    use world::World;

    fn arc_bytes(len: usize) -> Arc<[u8]> {
        Arc::from(vec![0u8; len])
    }

    #[test]
    fn load_rom_and_pump_frame_produces_gpu_work() {
        let world = World::new();
        let hub = make_hub();
        let mut scheduler = Scheduler::new(world, hub);
        scheduler.world_mut().set_auto_pump(false);

        let rom_bytes = arc_bytes(4);
        scheduler.enqueue_intent(
            IntentPriority::P0,
            Intent::LoadRom {
                bytes: Arc::clone(&rom_bytes),
            },
        );
        scheduler.run_once();

        assert!(scheduler.world().rom_loaded());
        assert_eq!(scheduler.world().rom_events(), 1);

        scheduler.enqueue_intent(IntentPriority::P1, Intent::PumpFrame);
        scheduler.run_once();

        assert!(scheduler.world().frame_id() > 0);
    }

    #[test]
    fn lossless_intent_is_requeued_on_would_block() {
        let world = World::new();
        let hub = make_hub_with_capacities(1, 1, 8, 8);
        let mut scheduler = Scheduler::new(world, hub);
        scheduler.world_mut().set_auto_pump(false);

        let rom_bytes = arc_bytes(2);
        let rom_bytes_b = Arc::clone(&rom_bytes);

        scheduler.enqueue_intent(
            IntentPriority::P0,
            Intent::LoadRom {
                bytes: Arc::clone(&rom_bytes),
            },
        );

        scheduler.enqueue_intent(IntentPriority::P0, Intent::LoadRom { bytes: rom_bytes_b });

        scheduler.run_once();

        let pending = scheduler.pending_intents();
        assert_eq!(pending[0], 1, "lossless intent should be requeued to P0");
        assert_eq!(scheduler.world().rom_events(), 1);

        scheduler.run_once();
        assert_eq!(scheduler.world().rom_events(), 2);
    }
}
