use service_abi::SlotSpan;
use std::sync::Arc;
use transport::{SlotPoolHandle, SlotPush};

/// Frame sink backed by a transport slot pool.
pub struct TransportFrameSink {
    pool: Arc<SlotPoolHandle>,
    width: u16,
    height: u16,
}

impl TransportFrameSink {
    pub fn new(pool: Arc<SlotPoolHandle>, width: u16, height: u16) -> Self {
        let width = if width == 0 { 160 } else { width };
        let height = if height == 0 { 144 } else { height };
        Self {
            pool,
            width,
            height,
        }
    }

    pub fn dimensions(&self) -> (u16, u16) {
        (self.width, self.height)
    }

    pub fn produce_frame(
        &self,
        expected_len: usize,
        mut write: impl FnMut(&mut [u8]),
    ) -> Option<(Arc<[u8]>, Option<SlotSpan>)> {
        let slot_idx = self.pool.with_mut(|pool| pool.try_acquire_free())?;

        let written = self.pool.with_mut(|pool| {
            let slot = pool.slot_mut(slot_idx);
            let usable_len = expected_len.min(slot.len());
            write(&mut slot[..usable_len]);
            usable_len
        });

        let mut waited = false;
        loop {
            match self.pool.with_mut(|pool| pool.push_ready(slot_idx)) {
                SlotPush::Ok => {
                    let empty = Arc::<[u8]>::from(&[][..]);
                    let span = SlotSpan {
                        start_idx: slot_idx,
                        count: 1,
                    };
                    return Some((empty, Some(span)));
                }
                SlotPush::WouldBlock if !waited => {
                    self.pool.with_ref(|pool| pool.wait_for_ready_drain());
                    waited = true;
                }
                SlotPush::WouldBlock => {
                    let pixels = self.pool.with_mut(|pool| {
                        let slot = pool.slot_mut(slot_idx);
                        let copy = Vec::from(&slot[..written]);
                        pool.release_free(slot_idx);
                        Arc::<[u8]>::from(copy.into_boxed_slice())
                    });
                    return Some((pixels, None));
                }
            }
        }
    }
}
