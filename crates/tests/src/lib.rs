//! Test suite for the Game Boy emulator.

#[cfg(target_arch = "wasm32")]
mod wasm;

#[cfg(all(test, not(target_arch = "wasm32")))]
mod native_e2e;

#[cfg(all(test, not(target_arch = "wasm32")))]
mod schema_golden;

#[cfg(test)]
mod tests {
    use app::Scheduler;
    use hub::{Intent, IntentPriority};
    use mock::{make_hub, make_hub_with_capacities};
    use services_transport::TransportServices;
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
                group: 0,
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
                group: 0,
                bytes: Arc::clone(&rom_bytes),
            },
        );

        scheduler.enqueue_intent(
            IntentPriority::P0,
            Intent::LoadRom {
                group: 0,
                bytes: rom_bytes_b,
            },
        );

        scheduler.run_once();

        let pending = scheduler.pending_intents();
        assert_eq!(pending[0], 1, "lossless intent should be requeued to P0");
        assert_eq!(scheduler.world().rom_events(), 1);

        scheduler.run_once();
        assert_eq!(scheduler.world().rom_events(), 2);
    }

    // Example slow test: deep exploration or heavy workload
    // Must be marked #[ignore] and prefixed with "slow_"
    #[test]
    #[ignore]
    fn slow_stress_many_intents() {
        let world = World::new();
        let hub = make_hub();
        let mut scheduler = Scheduler::new(world, hub);
        scheduler.world_mut().set_auto_pump(false);

        // Load a ROM first
        let rom_bytes = arc_bytes(4);
        scheduler.enqueue_intent(
            IntentPriority::P0,
            Intent::LoadRom {
                group: 0,
                bytes: Arc::clone(&rom_bytes),
            },
        );
        scheduler.run_once();
        assert!(scheduler.world().rom_loaded());

        // Enqueue and process many intents to stress the system
        for _ in 0..1000 {
            scheduler.enqueue_intent(IntentPriority::P1, Intent::PumpFrame);
            scheduler.run_once();
        }

        // Just verify the system still works after heavy load
        assert!(scheduler.world().rom_loaded());
    }

    #[test]
    fn transport_services_build_smoke() {
        let services = TransportServices::new().expect("build transport services");
        let _worker = services.worker;
        let _hub = services.hub;
    }
}
