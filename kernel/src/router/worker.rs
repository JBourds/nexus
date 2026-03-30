//! worker.rs
//! A `Worker` owns a subset of channels and handles message queuing,
//! delivery, and expiry for those channels independently of other workers.
//!
//! Workers are the unit of parallelism in distributed simulation. Each worker
//! runs on its own thread and communicates with the coordinator via channels.

use std::borrow::Cow;
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashSet, VecDeque};
use std::sync::Arc;

use config::ast::{ChannelKind, Position, TimestepConfig};
use rand::rngs::StdRng;
use tracing::{Level, debug, event, info, warn};

use super::delivery::{AddressedMsg, QueuedMessage};
use super::errors::RouterError;
use super::link_simulation::{message_timesteps, send_through_channel};
use super::table::ChannelRoutes;
use super::{Mailbox, MessageQueue, SignalInfo, Timestep};
use crate::helpers::format_u8_buf;
use crate::types::{ChannelHandle, ChannelIdx, NodeHandle};
use crate::ResolvedChannels;

/// An energy delta produced by a worker during a step, to be applied to the
/// canonical node state by the coordinator.
#[derive(Debug, Clone)]
pub(crate) struct EnergyDelta {
    pub node_idx: usize,
    /// Negative values represent drain (TX/RX costs).
    pub delta_nj: i64,
}

/// A worker owns a disjoint subset of channels and processes message queuing,
/// delivery, and expiry for those channels.
#[derive(Debug)]
#[allow(dead_code)]
pub(crate) struct Worker {
    /// Worker identifier (0-based).
    pub id: usize,
    /// Channel indices owned by this worker.
    pub owned_channels: HashSet<ChannelIdx>,
    /// Handle indices that belong to owned channels.
    pub owned_handles: HashSet<usize>,
    /// Local priority queue for messages on owned channels.
    pub queued: MessageQueue,
    /// Per-handle mailboxes (full-size vec indexed by handle_ptr; only owned
    /// handles are used, others remain empty).
    pub mailboxes: Vec<Mailbox>,
    /// Per-handle signal quality from the last RX.
    pub signal_info: Vec<SignalInfo>,
    /// Sequence counter for message ordering within a timestep.
    pub sequence: usize,
    /// Monotonic counter for unique message IDs.
    pub next_msg_id: u64,
    /// Random number generator (deterministically seeded per worker).
    pub rng: StdRng,
    /// Energy deltas accumulated during the current step.
    pub energy_deltas: Vec<EnergyDelta>,
}

#[allow(dead_code)]
impl Worker {
    /// Create a new worker for the given set of channels.
    pub fn new(
        id: usize,
        owned_channels: HashSet<ChannelIdx>,
        channels: &ResolvedChannels,
        rng: StdRng,
    ) -> Self {
        let handles_count = channels.handles.len();
        // Determine which handles belong to our channels.
        let owned_handles: HashSet<usize> = channels
            .handles
            .iter()
            .enumerate()
            .filter(|(_, (_, _, ch))| owned_channels.contains(ch))
            .map(|(i, _)| i)
            .collect();

        Self {
            id,
            owned_channels,
            owned_handles,
            queued: BinaryHeap::new(),
            mailboxes: vec![VecDeque::new(); handles_count],
            signal_info: vec![SignalInfo::default(); handles_count],
            sequence: 0,
            next_msg_id: 0,
            rng,
            energy_deltas: Vec::new(),
        }
    }

    /// Returns true if this worker owns the given channel.
    pub fn owns_channel(&self, ch: ChannelIdx) -> bool {
        self.owned_channels.contains(&ch)
    }

    /// Returns true if this worker owns the given handle index.
    pub fn owns_handle(&self, handle_ptr: usize) -> bool {
        self.owned_handles.contains(&handle_ptr)
    }

    /// Allocate a unique message ID for this worker.
    pub fn alloc_msg_id(&mut self) -> u64 {
        let id = self.next_msg_id;
        self.next_msg_id += 1;
        id
    }

    /// Queue a message from `src_node` on `channel_handle` to all subscribers.
    ///
    /// This is the worker-local equivalent of `RoutingServer::queue_message`.
    pub fn queue_message(
        &mut self,
        src_node: NodeHandle,
        channel_handle: ChannelHandle,
        msg: Vec<u8>,
        msg_id: u64,
        channels: &ResolvedChannels,
        routes: &[ChannelRoutes],
        timestep: Timestep,
        ts_config: TimestepConfig,
    ) -> Result<(), RouterError> {
        let sz: u64 = msg.len().try_into().expect("usize fits u64");
        let channel = &channels.channels[channel_handle.0];
        let is_shared = matches!(channel.r#type.kind, ChannelKind::Shared);

        // For shared channels we share one Arc across all recipients.
        let shared_buf: Arc<[u8]> = if is_shared {
            Arc::from(msg.as_slice())
        } else {
            Arc::from([])
        };

        for route in routes[channel_handle.0].nodes[&src_node].iter() {
            let handle_ptr = route.handle_ptr;
            let dst_node = channels.handles[handle_ptr].1;
            if dst_node == src_node && !channel.r#type.delivers_to_self() {
                continue;
            }

            debug!(
                "Delivering from {} to {}",
                &channels.node_names[src_node.0], &channels.node_names[dst_node.0]
            );

            let (distance, distance_unit) = Position::distance(
                &channels.nodes[src_node.0].position,
                &channels.nodes[dst_node.0].position,
            );

            // For exclusive channels, run link simulation now; drop the
            // message if it doesn't survive.
            let (buf, msg_bit_errors, rssi_dbm, snr_db): (Arc<[u8]>, bool, f64, f64) =
                if is_shared {
                    (Arc::clone(&shared_buf), false, 0.0, 0.0)
                } else {
                    match send_through_channel(
                        channel,
                        Cow::from(&msg),
                        distance,
                        distance_unit,
                        &mut self.rng,
                    ) {
                        Some((b, be, rssi, snr)) => (b.into(), be, rssi, snr),
                        None => continue,
                    }
                };

            let (becomes_active_at, expiration) =
                message_timesteps(channel, sz, ts_config, timestep, distance, distance_unit);

            let addressed = AddressedMsg {
                handle_ptr,
                msg: QueuedMessage {
                    src: src_node,
                    buf,
                    expiration,
                    bit_errors: msg_bit_errors,
                    msg_id,
                    rssi_dbm,
                    snr_db,
                },
            };

            let num = self.sequence;
            self.sequence += 1;
            self.queued.push((Reverse(becomes_active_at), num, addressed));
        }

        Ok(())
    }

    /// Remove expired messages from owned mailboxes.
    pub fn expire_messages(&mut self, timestep: Timestep) {
        for &handle_ptr in &self.owned_handles {
            let mailbox = &mut self.mailboxes[handle_ptr];
            while mailbox
                .front()
                .is_some_and(|msg| msg.expiration.is_some_and(|exp| exp.get() < timestep))
            {
                let _ = mailbox.pop_front();
            }
        }
    }

    /// Deliver all messages whose activation timestep has arrived.
    pub fn deliver_queued_messages(
        &mut self,
        timestep: Timestep,
        channels: &ResolvedChannels,
    ) -> Result<(), RouterError> {
        while self
            .queued
            .peek()
            .is_some_and(|(ts, _, _)| ts.0 <= timestep)
        {
            let Some((_, _, frame)) = self.queued.pop() else {
                return Err(RouterError::StepError);
            };
            let (_, dst_node, channel_handle) = channels.handles[frame.handle_ptr];
            let mailbox = &mut self.mailboxes[frame.handle_ptr];
            let channel = &channels.channels[channel_handle.0];

            if channel
                .r#type
                .max_buffered()
                .is_none_or(|n| n.get() > mailbox.len())
            {
                mailbox.push_back(frame.msg);

                // Track RX energy delta instead of draining directly
                let rx_cost_nj: u64 = channels.nodes[dst_node.0]
                    .channel_energy
                    .get(&channel_handle)
                    .and_then(|ce| ce.rx.as_ref())
                    .map(|e| e.unit.to_nj(e.quantity))
                    .unwrap_or(0);
                if rx_cost_nj > 0 {
                    self.energy_deltas.push(EnergyDelta {
                        node_idx: dst_node.0,
                        delta_nj: -(rx_cost_nj as i64),
                    });
                }
            } else {
                warn!("Message dropped due to full queue!");
                event!(
                    target: "drop", Level::WARN,
                    timestep = timestep,
                    channel = channel_handle.0,
                    node = frame.msg.src.0,
                    msg_id = frame.msg.msg_id,
                    reason = "buffer_full"
                );
            }
        }
        Ok(())
    }

    /// Deliver a message to a FUSE reader for an exclusive channel.
    pub fn deliver_exclusive_msg(
        &mut self,
        index: usize,
        channels: &ResolvedChannels,
        timestep: Timestep,
        tx: &std::sync::mpsc::Sender<fuse::KernelMessage>,
    ) -> Result<bool, RouterError> {
        let (pid, node_handle, channel_handle) = channels.handles[index];
        let mailbox = &mut self.mailboxes[index];
        if let Some(msg) = mailbox.pop_front() {
            self.signal_info[index].rssi_dbm = msg.rssi_dbm;
            self.signal_info[index].snr_db = msg.snr_db;
            if msg.expiration.is_some_and(|e| e.get() < timestep) {
                warn!(
                    "Message dropped due to timeout (Now: {}, Expiration: {})",
                    timestep,
                    msg.expiration.unwrap().get()
                );
                event!(
                    target: "drop", Level::WARN,
                    timestep = timestep,
                    channel = channel_handle.0,
                    node = node_handle.0,
                    msg_id = msg.msg_id,
                    reason = "ttl_expired"
                );
                return Ok(false);
            }
            let node_name = &channels.node_names[node_handle.0];
            let channel_name = &channels.channel_names[channel_handle.0];
            if tracing::enabled!(Level::INFO) {
                info!(
                    "{:<30} [RX]: {} <Now: {}, Expiration: {:?}>",
                    format!("{}.{}.{}", node_name, pid, channel_name),
                    format_u8_buf(&msg.buf),
                    timestep,
                    msg.expiration,
                );
            }
            let bit_errors = msg.bit_errors;
            let mid = msg.msg_id;
            let msg = fuse::Message {
                id: (pid, channels.node_names[node_handle.0].to_string()),
                data: msg.buf.to_vec(),
            };
            event!(
                target: "rx", Level::INFO, timestep = timestep, channel = channel_handle.0,
                node = node_handle.0, tx = false, bit_errors, msg_id = mid, data = msg.data.as_slice()
            );

            tx.send(fuse::KernelMessage::Exclusive(msg))
                .map(|_| true)
                .map_err(RouterError::FuseSendError)
        } else {
            Ok(false)
        }
    }

    /// Deliver a message to a FUSE reader for a shared channel.
    pub fn deliver_shared_msg(
        &mut self,
        index: usize,
        channels: &ResolvedChannels,
        timestep: Timestep,
        tx: &std::sync::mpsc::Sender<fuse::KernelMessage>,
    ) -> Result<bool, RouterError> {
        let (pid, node_handle, channel_handle) = channels.handles[index];
        let channel = &channels.channels[channel_handle.0];
        let channel_name = &channels.channel_names[channel_handle.0];
        let node_name = &channels.node_names[node_handle.0];

        let mailbox = &mut self.mailboxes[index];
        // remove all expired messages
        while mailbox
            .front()
            .is_some_and(|msg| msg.expiration.is_some_and(|exp| exp.get() < timestep))
        {
            mailbox.pop_front();
        }

        match mailbox.len().cmp(&1) {
            std::cmp::Ordering::Less => Ok(false),
            std::cmp::Ordering::Equal => {
                let msg = mailbox.pop_front().unwrap();
                let (distance, unit) = Position::distance(
                    &channels.nodes[msg.src.0].position,
                    &channels.nodes[node_handle.0].position,
                );

                if let Some((buf, bit_errors, rssi_dbm, snr_db)) = send_through_channel(
                    channel,
                    Cow::from(msg.buf.as_ref()),
                    distance,
                    unit,
                    &mut self.rng,
                ) {
                    self.signal_info[index].rssi_dbm = rssi_dbm;
                    self.signal_info[index].snr_db = snr_db;
                    if tracing::enabled!(Level::INFO) {
                        info!(
                            "{:<30} [RX]: {} <Now: {}, Expiration: {:?}>",
                            format!("{}.{}.{}", node_name, pid, channel_name),
                            format_u8_buf(&buf),
                            timestep,
                            msg.expiration,
                        );
                    }
                    let mid = msg.msg_id;
                    event!(
                        target: "rx", Level::INFO, timestep, channel = channel_handle.0,
                        node = node_handle.0, tx = false, bit_errors, msg_id = mid, data = buf.as_ref()
                    );
                    let msg = fuse::Message {
                        id: (pid, node_name.to_string()),
                        data: buf.to_vec(),
                    };
                    tx.send(fuse::KernelMessage::Shared(msg))
                        .map(|_| true)
                        .map_err(RouterError::FuseSendError)
                } else {
                    Ok(false)
                }
            }
            std::cmp::Ordering::Greater => {
                warn!("Detected collision on shared medium.");
                let max_size = channel.r#type.max_size;

                let filtered = mailbox.iter().filter_map(|msg| {
                    let (distance, unit) = Position::distance(
                        &channels.nodes[msg.src.0].position,
                        &channels.nodes[node_handle.0].position,
                    );
                    send_through_channel(
                        channel,
                        Cow::from(msg.buf.as_ref()),
                        distance,
                        unit,
                        &mut self.rng,
                    )
                });

                let mut any_bit_errors = false;
                let mut last_rssi = 0.0_f64;
                let mut last_snr = 0.0_f64;
                let buf = filtered.fold(
                    Vec::with_capacity(max_size.get()),
                    |mut v, (msg, bit_errors, rssi, snr)| {
                        any_bit_errors |= bit_errors;
                        last_rssi = rssi;
                        last_snr = snr;
                        let small = v.len().min(msg.len());
                        for i in 0..small {
                            v[i] |= msg[i];
                        }
                        v.extend_from_slice(&msg[small..]);
                        v
                    },
                );
                self.signal_info[index].rssi_dbm = last_rssi;
                self.signal_info[index].snr_db = last_snr;
                let bit_errors = any_bit_errors || mailbox.len() > 1;
                let mid = mailbox.front().map(|m| m.msg_id).unwrap_or(0);
                event!(
                    target: "rx", Level::INFO, timestep, channel = channel_handle.0,
                    node = node_handle.0, tx = false, bit_errors, msg_id = mid, data = buf.as_slice()
                );
                let msg = fuse::Message {
                    id: (pid, node_name.to_string()),
                    data: buf,
                };
                tx.send(fuse::KernelMessage::Shared(msg))
                    .map(|_| true)
                    .map_err(RouterError::FuseSendError)
            }
        }
    }

    /// Deliver a message to a FUSE reader, dispatching by channel type.
    pub fn deliver_msg(
        &mut self,
        index: usize,
        channels: &ResolvedChannels,
        timestep: Timestep,
        tx: &std::sync::mpsc::Sender<fuse::KernelMessage>,
    ) -> Result<bool, RouterError> {
        let (_, _, channel_handle) = channels.handles[index];
        let channel = &channels.channels[channel_handle.0];
        match &channel.r#type.kind {
            ChannelKind::Shared => self.deliver_shared_msg(index, channels, timestep, tx),
            ChannelKind::Exclusive { .. } => {
                self.deliver_exclusive_msg(index, channels, timestep, tx)
            }
        }
    }

    /// Clear mailboxes for handles that match the given PID.
    pub fn clear_mailboxes_for_pid(&mut self, pid: u32, channels: &ResolvedChannels) {
        for &handle_ptr in &self.owned_handles {
            if channels.handles[handle_ptr].0 == pid {
                self.mailboxes[handle_ptr].clear();
            }
        }
    }

    /// Drain accumulated energy deltas, returning them and clearing the buffer.
    pub fn take_energy_deltas(&mut self) -> Vec<EnergyDelta> {
        std::mem::take(&mut self.energy_deltas)
    }
}
