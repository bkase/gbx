use crate::bus::BusScalar;
use crate::bus_simd::BusSimd;
use crate::core::CoreConfig;
use crate::exec_simd::SimdExec;
use crate::{Core, Scalar};
use core::simd::{LaneCount, SupportedLaneCount};
use std::num::NonZeroUsize;
use std::sync::Arc;

type SerialLog = String;

fn backend_core_config_scalar() -> CoreConfig {
    CoreConfig::default()
}

fn backend_core_config_simd(lanes: usize) -> CoreConfig {
    CoreConfig {
        lanes: NonZeroUsize::new(lanes).expect("lanes must be non-zero"),
        ..CoreConfig::default()
    }
}

fn backend_new_bus_scalar(rom: Arc<[u8]>) -> BusScalar {
    BusScalar::new(rom, None)
}

fn backend_new_bus_simd<const LANES: usize>(rom: Arc<[u8]>) -> BusSimd<LANES>
where
    LaneCount<LANES>: SupportedLaneCount,
{
    BusSimd::new(rom, None)
}

fn backend_take_serial_scalar(core: &mut Core<Scalar, BusScalar>) -> SerialLog {
    core.bus.take_serial()
}

fn backend_take_serial_simd<const LANES: usize>(
    core: &mut Core<SimdExec<LANES>, BusSimd<LANES>>,
) -> SerialLog
where
    LaneCount<LANES>: SupportedLaneCount,
{
    core.bus.lane_mut(0).take_serial()
}

fn backend_bus_mut_scalar(core: &mut Core<Scalar, BusScalar>) -> &mut BusScalar {
    &mut core.bus
}

fn backend_bus_mut_simd<const LANES: usize>(
    core: &mut Core<SimdExec<LANES>, BusSimd<LANES>>,
) -> &mut BusScalar
where
    LaneCount<LANES>: SupportedLaneCount,
{
    core.bus.lane_mut(0)
}

fn backend_bus_scalar(core: &Core<Scalar, BusScalar>) -> &BusScalar {
    &core.bus
}

fn backend_bus_simd<const LANES: usize>(core: &Core<SimdExec<LANES>, BusSimd<LANES>>) -> &BusScalar
where
    LaneCount<LANES>: SupportedLaneCount,
{
    core.bus.lane(0)
}

macro_rules! shared_backend_module {
    ($mod_name:ident, $exec:ty, $bus:ty, $lanes:expr, $config_fn:expr, $new_bus_fn:expr, $take_serial_fn:expr, $bus_fn:expr, $bus_mut_fn:expr) => {
        mod $mod_name {
            use super::*;
            type BackendExec = $exec;
            type BackendBus = $bus;
            const BACKEND_LANES: usize = $lanes;

            fn backend_core_config() -> CoreConfig {
                $config_fn
            }

            fn backend_new_bus(rom: Arc<[u8]>) -> BackendBus {
                $new_bus_fn(rom)
            }

            fn backend_take_serial(core: &mut Core<BackendExec, BackendBus>) -> SerialLog {
                $take_serial_fn(core)
            }

            fn backend_bus(core: &Core<BackendExec, BackendBus>) -> &BusScalar {
                $bus_fn(core)
            }

            fn backend_bus_mut(core: &mut Core<BackendExec, BackendBus>) -> &mut BusScalar {
                $bus_mut_fn(core)
            }

            include!("tests_common.rs");
        }
    };
}

shared_backend_module!(
    scalar,
    Scalar,
    BusScalar,
    1,
    backend_core_config_scalar(),
    backend_new_bus_scalar,
    backend_take_serial_scalar,
    backend_bus_scalar,
    backend_bus_mut_scalar
);

shared_backend_module!(
    simd,
    SimdExec<4>,
    BusSimd<4>,
    4,
    backend_core_config_simd(4),
    backend_new_bus_simd::<4>,
    backend_take_serial_simd::<4>,
    backend_bus_simd::<4>,
    backend_bus_mut_simd::<4>
);
