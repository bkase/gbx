use hub::SlotSpan;
use std::cell::RefCell;
use std::sync::Arc;
use transport::{SlotPoolHandle, SlotPush};

struct TransportLease {
    slot_idx: u32,
    ptr: *mut [u8],
}

// SAFETY: TransportLease holds a raw pointer to a heap-allocated buffer that it owns exclusively.
// The buffer is created via Box and converted to raw pointer, ensuring proper ownership transfer.
unsafe impl Send for TransportLease {}

/// Frame sink backed by a transport slot pool.
pub struct TransportFrameSink {
    pool: Arc<SlotPoolHandle>,
    width: u16,
    height: u16,
    active: RefCell<Option<TransportLease>>,
}

impl TransportFrameSink {
    pub fn new(pool: Arc<SlotPoolHandle>, width: u16, height: u16) -> Self {
        Self {
            pool,
            width,
            height,
            active: RefCell::new(None),
        }
    }

    fn frame_len(&self) -> usize {
        usize::from(self.width)
            .saturating_mul(usize::from(self.height))
            .saturating_mul(4)
    }

    pub fn dimensions(&self) -> (u16, u16) {
        (self.width, self.height)
    }

    #[allow(clippy::mut_from_ref)]
    pub fn acquire_frame(&self) -> Option<(u32, &mut [u8])> {
        let slot_idx = self.pool.with_mut(|pool| pool.try_acquire_free())?;
        let slot_size = self.pool.with_ref(|pool| pool.slot_size());

        let len = self.frame_len().min(slot_size).max(1);
        let buffer = vec![0u8; len].into_boxed_slice();
        let ptr = Box::into_raw(buffer);

        if self.active.borrow().is_some() {
            // SAFETY: ptr was created via Box::into_raw above, so it's valid to reconstruct and drop.
            unsafe {
                drop(Box::from_raw(ptr));
            }
            return None;
        }

        // SAFETY: pointer originates from `Box::into_raw` above.
        unsafe {
            let slice = &mut *ptr;
            self.active
                .borrow_mut()
                .replace(TransportLease { slot_idx, ptr });
            Some((slot_idx, slice))
        }
    }

    pub fn publish(&self, slot_idx: u32, written_len: usize) -> Option<SlotSpan> {
        let lease = self
            .active
            .borrow_mut()
            .take()
            .expect("publish without matching acquire");
        assert_eq!(lease.slot_idx, slot_idx, "slot mismatch in publish");

        // SAFETY: lease pointer is owned by the frame sink and not used elsewhere.
        let buffer = unsafe { Box::from_raw(lease.ptr) };
        let copy_len = written_len.min(buffer.len());

        let push_outcome = self.pool.with_mut(|pool| {
            {
                let slot = pool.slot_mut(slot_idx);
                let len = copy_len.min(slot.len());
                slot[..len].copy_from_slice(&buffer[..len]);
                if len < slot.len() {
                    slot[len..].fill(0);
                }
            }
            pool.push_ready(slot_idx)
        });

        match push_outcome {
            SlotPush::Ok => Some(SlotSpan {
                start_idx: slot_idx,
                count: 1,
            }),
            SlotPush::WouldBlock => {
                self.pool.with_mut(|pool| pool.release_free(slot_idx));
                None
            }
        }
    }

    pub fn produce_frame(
        &self,
        expected_len: usize,
        mut write: impl FnMut(&mut [u8]),
    ) -> Option<SlotSpan> {
        let (slot_idx, buffer) = self.acquire_frame()?;
        let usable_len = expected_len.min(buffer.len());
        write(&mut buffer[..usable_len]);
        self.publish(slot_idx, usable_len)
    }
}
