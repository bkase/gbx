use crate::stats::ScenarioStats;

/// Borrowed view over drained frame/event data for verification helpers.
pub struct DrainReport<'a> {
    pub frames: &'a [u32],
    pub events: &'a [u32],
    pub max_ready_depth: Option<usize>,
}

pub type CheckResult = Result<(), String>;

pub fn verify_flood(
    drain: &DrainReport<'_>,
    stats: &ScenarioStats,
    expected_frames: u32,
) -> CheckResult {
    if drain.frames.len() as u32 != expected_frames {
        return Err(format!(
            "drained {} frames (expected {})",
            drain.frames.len(),
            expected_frames
        ));
    }
    if drain.events.len() as u32 != expected_frames {
        return Err(format!(
            "drained {} events (expected {})",
            drain.events.len(),
            expected_frames
        ));
    }
    if drain.frames != drain.events {
        return Err("frame/event ordering mismatch".into());
    }
    if stats.produced != expected_frames {
        return Err(format!(
            "stats produced {} frames (expected {})",
            stats.produced, expected_frames
        ));
    }
    if stats.would_block_ready != 0 {
        return Err(format!(
            "ready ring reported {} WouldBlock occurrences (expected 0)",
            stats.would_block_ready
        ));
    }
    if stats.would_block_evt != 0 {
        return Err(format!(
            "event ring reported {} WouldBlock occurrences (expected 0)",
            stats.would_block_evt
        ));
    }
    Ok(())
}

pub fn verify_burst(
    drain: &DrainReport<'_>,
    stats: &ScenarioStats,
    expected_frames: u32,
    slot_budget: usize,
) -> CheckResult {
    verify_flood(drain, stats, expected_frames)?;
    if let Some(depth) = drain.max_ready_depth {
        if depth > slot_budget {
            return Err(format!(
                "ready ring depth {} exceeded slot budget {}",
                depth, slot_budget
            ));
        }
    }
    if stats.would_block_evt != 0 {
        return Err(format!(
            "event ring reported {} WouldBlock occurrences (expected 0)",
            stats.would_block_evt
        ));
    }
    Ok(())
}

pub fn verify_backpressure(
    drain: &DrainReport<'_>,
    stats: &ScenarioStats,
    expected_frames: u32,
) -> CheckResult {
    verify_flood(drain, stats, expected_frames)?;
    if stats.would_block_ready == 0 && stats.free_waits == 0 {
        return Err(
            "backpressure scenario expected WouldBlock or free waits, observed neither".into(),
        );
    }
    Ok(())
}
