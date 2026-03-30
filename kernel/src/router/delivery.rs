use super::*;

/// Internal struct which marks a queued message as being targeted at
/// `handle_ptr`, the specific endpoint it should be delivered to in the FS.
#[derive(Clone, Debug)]
pub(crate) struct AddressedMsg {
    pub(super) handle_ptr: usize,
    pub(super) msg: QueuedMessage,
}

// Ordering for BinaryHeap: only used as tiebreaker after (Reverse<Timestep>, seq).
// The sequence counter guarantees unique ordering, so this is never actually
// reached, but the trait is required by BinaryHeap's element constraint.
impl PartialEq for AddressedMsg {
    fn eq(&self, other: &Self) -> bool {
        self.handle_ptr == other.handle_ptr && self.msg.msg_id == other.msg.msg_id
    }
}
impl Eq for AddressedMsg {}
impl PartialOrd for AddressedMsg {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for AddressedMsg {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.msg
            .msg_id
            .cmp(&other.msg.msg_id)
            .then(self.handle_ptr.cmp(&other.handle_ptr))
    }
}

/// Internal struct used for keeping track of where a queued message is from and
/// its expiration.
#[derive(Clone, Debug)]
pub(crate) struct QueuedMessage {
    pub(super) src: NodeHandle,
    pub(super) buf: Arc<[u8]>,
    pub(super) expiration: Option<NonZeroU64>,
    /// Whether the data was corrupted by bit errors during link simulation.
    pub(super) bit_errors: bool,
    /// Unique identifier for correlating TX/RX events in the trace.
    pub(super) msg_id: u64,
    /// RSSI at the receiver in dBm (computed during link simulation).
    pub(super) rssi_dbm: f64,
    /// SNR at the receiver in dB (computed during link simulation).
    pub(super) snr_db: f64,
}

impl RoutingServer {
    /// Queue a message by delegating to the appropriate worker via the coordinator.
    ///
    /// This is the public API used by sources (simulated, replay, trace) and tests.
    pub fn queue_message(
        &mut self,
        src_node: NodeHandle,
        channel_handle: ChannelHandle,
        msg: Vec<u8>,
        msg_id: u64,
    ) -> Result<(), RouterError> {
        // Find the first handle for this channel to determine the owning worker.
        // All handles for a channel map to the same worker.
        let worker_idx = self
            .channels
            .handles
            .iter()
            .enumerate()
            .find(|(_, (_, _, ch))| *ch == channel_handle)
            .map(|(i, _)| self.coordinator.worker_for_handle(i))
            .unwrap_or(0);
        let worker = &mut self.coordinator.workers[worker_idx];
        worker.queue_message(
            src_node,
            channel_handle,
            msg,
            msg_id,
            &self.channels,
            &self.routes.entries,
            self.timestep,
            self.ts_config,
        )
    }
}
