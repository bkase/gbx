use parking_lot::Mutex;
use std::ptr::NonNull;
use std::sync::Arc;

#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct ScenarioStats {
    pub produced: u32,
    pub would_block_ready: u32,
    pub would_block_evt: u32,
    pub free_waits: u32,
}

impl ScenarioStats {
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

pub trait StatsSink: Clone + Send + 'static {
    fn with_stats<R>(&self, f: impl FnOnce(&mut ScenarioStats) -> R) -> R;
}

#[derive(Clone)]
pub struct PtrStatsSink {
    ptr: NonNull<ScenarioStats>,
}

impl PtrStatsSink {
    /// # Safety
    ///
    /// Caller must guarantee `ptr` points to valid `ScenarioStats` memory that
    /// lives for the duration of the sink usage.
    pub unsafe fn new(ptr: *mut ScenarioStats) -> Option<Self> {
        NonNull::new(ptr).map(|ptr| Self { ptr })
    }
}

unsafe impl Send for PtrStatsSink {}

impl StatsSink for PtrStatsSink {
    fn with_stats<R>(&self, f: impl FnOnce(&mut ScenarioStats) -> R) -> R {
        // SAFETY: Constructor requires pointer validity for the sink lifetime.
        unsafe {
            f(self
                .ptr
                .as_ptr()
                .as_mut()
                .expect("stats pointer must remain valid"))
        }
    }
}

#[derive(Clone, Default)]
pub struct ArcStatsSink(pub Arc<Mutex<ScenarioStats>>);

impl ArcStatsSink {
    pub fn new(stats: Arc<Mutex<ScenarioStats>>) -> Self {
        Self(stats)
    }
}

impl StatsSink for ArcStatsSink {
    fn with_stats<R>(&self, f: impl FnOnce(&mut ScenarioStats) -> R) -> R {
        let mut guard = self.0.lock();
        f(&mut *guard)
    }
}

impl StatsSink for Arc<Mutex<ScenarioStats>> {
    fn with_stats<R>(&self, f: impl FnOnce(&mut ScenarioStats) -> R) -> R {
        let mut guard = self.lock();
        f(&mut *guard)
    }
}
