use rkyv::{Archive, Deserialize, Serialize};
use transport::wasm::{
    MailboxLayout as WasmMailboxLayout, MsgRingLayout as WasmMsgRingLayout,
    SlotPoolLayout as WasmSlotPoolLayout,
};

/// Role assigned to a transport port or pool within an endpoint.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Archive, Deserialize, Serialize)]
pub enum PortRole {
    CmdLossless,
    CmdBestEffort,
    CmdMailbox,
    Replies,
    SlotPool(usize),
}

/// Platform-agnostic layout description for a port or pool.
#[derive(Clone, Copy, Debug, Archive, Deserialize, Serialize)]
pub enum PortLayout {
    MsgRing(WasmMsgRingLayout),
    Mailbox(WasmMailboxLayout),
    SlotPool(WasmSlotPoolLayout),
}

/// Layout describing an endpoint exposed to a worker.
#[derive(Clone, Debug, Default, Archive, Deserialize, Serialize)]
pub struct EndpointLayout {
    pub ports: Vec<(PortRole, PortLayout)>,
}

impl EndpointLayout {
    pub fn push_port(&mut self, role: PortRole, layout: PortLayout) {
        self.ports.push((role, layout));
    }
}

/// Aggregate layout for a built fabric. Used when initialising web workers.
#[derive(Clone, Debug, Default, Archive, Deserialize, Serialize)]
pub struct FabricLayout {
    pub endpoints: Vec<EndpointLayout>,
}

impl FabricLayout {
    pub fn add_endpoint(&mut self, endpoint: EndpointLayout) {
        self.endpoints.push(endpoint);
    }
}
