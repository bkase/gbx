//! Integration tests covering P0 ≻ P2 priority queue semantics.

use app::priority::PQueues;
use hub::IntentPriority;

/// Ensures higher priorities always pop before lower ones.
#[test]
fn priority_ordering_respected() {
    let mut queues = PQueues::new();

    queues.enqueue(IntentPriority::P1, "middle-1");
    queues.enqueue(IntentPriority::P2, "low");
    queues.enqueue(IntentPriority::P0, "high-1");
    queues.enqueue(IntentPriority::P0, "high-2");
    queues.enqueue(IntentPriority::P1, "middle-2");

    assert_eq!(queues.pop_next(), Some("high-1"));
    assert_eq!(queues.pop_next(), Some("high-2"));
    assert_eq!(queues.pop_next(), Some("middle-1"));
    assert_eq!(queues.pop_next(), Some("middle-2"));
    assert_eq!(queues.pop_next(), Some("low"));
    assert_eq!(queues.pop_next(), None);
}

/// Verifies FIFO stability for intents in the same priority class.
#[test]
fn fifo_stability_within_priority() {
    let mut queues = PQueues::new();

    queues.enqueue(IntentPriority::P1, 1);
    queues.enqueue(IntentPriority::P1, 2);
    queues.enqueue(IntentPriority::P1, 3);

    assert_eq!(queues.pop_next(), Some(1));
    assert_eq!(queues.pop_next(), Some(2));
    assert_eq!(queues.pop_next(), Some(3));
}

/// Confirms empty checks and length accounting behave as expected.
#[test]
fn empty_behavior_and_len_tracking() {
    let mut queues = PQueues::with_capacity(2);
    assert!(queues.is_empty());
    assert_eq!(queues.len_per_priority(), [0, 0, 0]);

    queues.enqueue(IntentPriority::P2, 'a');
    queues.enqueue(IntentPriority::P0, 'b');

    assert_eq!(queues.len_per_priority(), [1, 0, 1]);
    assert_eq!(queues.pop_next(), Some('b'));
    assert_eq!(queues.pop_next(), Some('a'));
    assert!(queues.is_empty());
    assert_eq!(queues.pop_next(), None);
}
