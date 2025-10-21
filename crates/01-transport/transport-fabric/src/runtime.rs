use std::time::Duration;

pub trait ServiceEngine: Send {
    fn poll(&mut self) -> usize;
    fn name(&self) -> &'static str;
}

pub struct WorkerRuntime {
    engines: Vec<Box<dyn ServiceEngine>>,
}

impl Default for WorkerRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl WorkerRuntime {
    pub fn new() -> Self {
        Self {
            engines: Vec::new(),
        }
    }

    pub fn register<E>(&mut self, engine: E)
    where
        E: ServiceEngine + 'static,
    {
        self.engines.push(Box::new(engine));
    }

    pub fn run_tick(&mut self) -> usize {
        let mut work = 0;
        for engine in self.engines.iter_mut() {
            work += engine.poll();
        }
        work
    }

    pub fn run_until_idle(&mut self, idle_threshold: Duration) {
        let deadline = std::time::Instant::now() + idle_threshold;
        loop {
            let work = self.run_tick();
            if work == 0 && std::time::Instant::now() >= deadline {
                break;
            }
        }
    }
}
