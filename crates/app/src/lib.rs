use hub::{
    Intent, IntentPriority, IntentReducer, ReportReducer, ServicesHub, SubmitOutcome, SubmitPolicy,
    DEFAULT_INTENT_BUDGET, DEFAULT_REPORT_BUDGET,
};
use std::collections::VecDeque;
use world::World;

pub struct Scheduler {
    world: World,
    hub: ServicesHub,
    intent_queues: [VecDeque<Intent>; 3],
    intent_budget: usize,
    report_budget: usize,
}

impl Scheduler {
    pub fn new(world: World, hub: ServicesHub) -> Self {
        Self::with_budgets(world, hub, DEFAULT_INTENT_BUDGET, DEFAULT_REPORT_BUDGET)
    }

    pub fn with_budgets(
        world: World,
        hub: ServicesHub,
        intent_budget: usize,
        report_budget: usize,
    ) -> Self {
        Self {
            world,
            hub,
            intent_queues: [
                VecDeque::with_capacity(16),
                VecDeque::with_capacity(16),
                VecDeque::with_capacity(16),
            ],
            intent_budget,
            report_budget,
        }
    }

    pub fn enqueue_intent(&mut self, priority: IntentPriority, intent: Intent) {
        self.intent_queues[priority.index()].push_back(intent);
    }

    pub fn enqueue_front_p0(&mut self, intent: Intent) {
        self.intent_queues[IntentPriority::P0.index()].push_front(intent);
    }

    pub fn pending_intents(&self) -> [usize; 3] {
        [
            self.intent_queues[0].len(),
            self.intent_queues[1].len(),
            self.intent_queues[2].len(),
        ]
    }

    pub fn world(&self) -> &World {
        &self.world
    }

    pub fn world_mut(&mut self) -> &mut World {
        &mut self.world
    }

    fn next_priority(&self) -> Option<IntentPriority> {
        if !self.intent_queues[0].is_empty() {
            Some(IntentPriority::P0)
        } else if !self.intent_queues[1].is_empty() {
            Some(IntentPriority::P1)
        } else if !self.intent_queues[2].is_empty() {
            Some(IntentPriority::P2)
        } else {
            None
        }
    }

    fn process_intents(&mut self) {
        let mut budget = self.intent_budget;
        while budget > 0 {
            let Some(priority) = self.next_priority() else {
                break;
            };
            let intent = self.intent_queues[priority.index()]
                .pop_front()
                .expect("queue not empty");
            budget -= 1;
            let commands = self.world.reduce_intent(intent.clone());

            let mut needs_retry_front = false;
            for cmd in commands {
                let policy = cmd.default_policy();
                let outcome = self.hub.try_submit_work(cmd);
                match outcome {
                    SubmitOutcome::WouldBlock => {
                        if matches!(policy, SubmitPolicy::Must | SubmitPolicy::Lossless) {
                            needs_retry_front = true;
                            break;
                        }
                    }
                    SubmitOutcome::Closed => {
                        needs_retry_front = true;
                        break;
                    }
                    _ => {}
                }
            }

            if needs_retry_front {
                self.enqueue_front_p0(intent);
            }
        }
    }

    fn process_reports(&mut self) {
        let reports = self.hub.drain_reports(self.report_budget);
        for report in reports {
            let follow_ups = self.world.reduce_report(report);
            for av in follow_ups.immediate_av {
                let policy = av.default_policy();
                let outcome = self.hub.try_submit_av(av);
                if matches!(outcome, SubmitOutcome::WouldBlock | SubmitOutcome::Closed)
                    && matches!(policy, SubmitPolicy::Must | SubmitPolicy::Lossless)
                {
                    // No retry path for A/V yet; a real implementation would surface this via health flags.
                }
            }
            for (priority, intent) in follow_ups.deferred_intents {
                self.enqueue_intent(priority, intent);
            }
        }
    }

    pub fn run_once(&mut self) {
        self.process_intents();
        self.process_reports();
    }
}
