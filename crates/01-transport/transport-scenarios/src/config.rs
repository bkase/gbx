#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScenarioType {
    Flood = 0,
    Burst = 1,
    Backpressure = 2,
}

impl ScenarioType {
    pub fn from_u32(value: u32) -> Option<Self> {
        match value {
            0 => Some(ScenarioType::Flood),
            1 => Some(ScenarioType::Burst),
            2 => Some(ScenarioType::Backpressure),
            _ => None,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct TestConfig {
    pub test_type: u32,
    pub param1: u32,
    pub param2: u32,
}

impl TestConfig {
    pub fn flood(frame_count: u32) -> Self {
        Self {
            test_type: ScenarioType::Flood as u32,
            param1: frame_count,
            param2: 0,
        }
    }

    pub fn burst(bursts: u32, burst_size: u32) -> Self {
        Self {
            test_type: ScenarioType::Burst as u32,
            param1: bursts,
            param2: burst_size,
        }
    }

    pub fn backpressure(frames: u32) -> Self {
        Self {
            test_type: ScenarioType::Backpressure as u32,
            param1: frames,
            param2: 0,
        }
    }

    pub fn scenario_kind(&self) -> Option<ScenarioKind> {
        let ty = ScenarioType::from_u32(self.test_type)?;
        Some(match ty {
            ScenarioType::Flood => ScenarioKind::Flood {
                frame_count: self.param1,
            },
            ScenarioType::Burst => ScenarioKind::Burst {
                bursts: self.param1,
                burst_size: self.param2,
            },
            ScenarioType::Backpressure => ScenarioKind::Backpressure {
                frames: self.param1,
            },
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScenarioKind {
    Flood { frame_count: u32 },
    Burst { bursts: u32, burst_size: u32 },
    Backpressure { frames: u32 },
}
