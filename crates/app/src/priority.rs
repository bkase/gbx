use std::collections::VecDeque;

use hub::IntentPriority;

/// Fixed set of priority queues (P0 ≻ P1 ≻ P2) with O(1) enqueue/dequeue.
#[derive(Debug)]
pub struct PQueues<T> {
    p0: VecDeque<T>,
    p1: VecDeque<T>,
    p2: VecDeque<T>,
}

impl<T> Default for PQueues<T> {
    fn default() -> Self {
        Self {
            p0: VecDeque::new(),
            p1: VecDeque::new(),
            p2: VecDeque::new(),
        }
    }
}

impl<T> PQueues<T> {
    /// Creates empty priority queues with default capacity.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates empty priority queues with an initial capacity per priority bucket.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            p0: VecDeque::with_capacity(capacity),
            p1: VecDeque::with_capacity(capacity),
            p2: VecDeque::with_capacity(capacity),
        }
    }

    /// Returns `true` when all priority queues are empty.
    pub fn is_empty(&self) -> bool {
        self.p0.is_empty() && self.p1.is_empty() && self.p2.is_empty()
    }

    /// Returns the number of items in each priority bucket ordered as [P0, P1, P2].
    pub fn len_per_priority(&self) -> [usize; 3] {
        [self.p0.len(), self.p1.len(), self.p2.len()]
    }

    /// Enqueues `item` at the back of the queue matching `priority`.
    pub fn enqueue(&mut self, priority: IntentPriority, item: T) {
        match priority {
            IntentPriority::P0 => self.p0.push_back(item),
            IntentPriority::P1 => self.p1.push_back(item),
            IntentPriority::P2 => self.p2.push_back(item),
        }
    }

    /// Enqueues `item` at the front of the highest priority queue.
    pub fn enqueue_front_p0(&mut self, item: T) {
        self.p0.push_front(item);
    }

    /// Pops the next item honoring P0 ≻ P1 ≻ P2 priority ordering.
    pub fn pop_next(&mut self) -> Option<T> {
        if let Some(item) = self.p0.pop_front() {
            Some(item)
        } else if let Some(item) = self.p1.pop_front() {
            Some(item)
        } else {
            self.p2.pop_front()
        }
    }

    /// Returns the highest priority bucket that currently has items.
    pub fn current_priority(&self) -> Option<IntentPriority> {
        if !self.p0.is_empty() {
            Some(IntentPriority::P0)
        } else if !self.p1.is_empty() {
            Some(IntentPriority::P1)
        } else if !self.p2.is_empty() {
            Some(IntentPriority::P2)
        } else {
            None
        }
    }
}
