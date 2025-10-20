use transport::SlotPush;

pub trait FabricHandle: Send {
    fn acquire_free_slot(&mut self) -> Option<u32>;
    fn wait_for_free_slot(&self);
    fn write_frame(&mut self, slot_idx: u32, frame_id: u32);
    fn push_ready(&mut self, slot_idx: u32) -> SlotPush;
    fn wait_for_ready_drain(&self);
    fn try_push_event(&mut self, frame_id: u32, slot_idx: u32) -> bool;
    fn wait_for_event_space(&self);
}
