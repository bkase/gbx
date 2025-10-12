use parking_lot::Mutex;
use std::collections::VecDeque;

pub struct NonBlockingQueue<T> {
    inner: Mutex<VecDeque<T>>,
    capacity: usize,
}

impl<T> NonBlockingQueue<T> {
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(VecDeque::with_capacity(capacity)),
            capacity,
        }
    }

    pub fn try_push(&self, item: T) -> Result<(), T> {
        let mut guard = self.inner.lock();
        if guard.len() >= self.capacity {
            Err(item)
        } else {
            guard.push_back(item);
            Ok(())
        }
    }

    pub fn try_pop(&self) -> Option<T> {
        self.inner.lock().pop_front()
    }

    pub fn len(&self) -> usize {
        self.inner.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.lock().is_empty()
    }
}
