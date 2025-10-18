use transport::wasm::{MailboxLayout as WasmMailboxLayout, MsgRingLayout as WasmMsgRingLayout};

/// Role assigned to a transport port within an endpoint.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PortRole {
    CmdLossless,
    CmdBestEffort,
    CmdMailbox,
    Replies,
}

/// Platform-agnostic layout description for a port.
#[derive(Clone, Copy, Debug)]
pub enum PortLayout {
    MsgRing(WasmMsgRingLayout),
    Mailbox(WasmMailboxLayout),
}

/// Layout describing an endpoint exposed to a worker.
#[derive(Clone, Debug, Default)]
pub struct EndpointLayout {
    pub ports: Vec<(PortRole, PortLayout)>,
}

impl EndpointLayout {
    pub fn push_port(&mut self, role: PortRole, layout: PortLayout) {
        self.ports.push((role, layout));
    }
}

/// Aggregate layout for a built fabric. Used when initialising web workers.
#[derive(Clone, Debug, Default)]
pub struct FabricLayout {
    pub endpoints: Vec<EndpointLayout>,
}

impl FabricLayout {
    pub fn add_endpoint(&mut self, endpoint: EndpointLayout) {
        self.endpoints.push(endpoint);
    }
}
