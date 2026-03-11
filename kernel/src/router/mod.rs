//! router.rs
//! Module responsible for ingesting messages and routing them to all
//! destinations which they should be received. Specifically, this mdoule
//! performs the following responsibilities:
//!
//! 1. Constructing routing table for where messages should be delivered.
//! 2. Computing route information for link simulation.
//! 3. Delivering messages after link simulation.

use crate::{
    KernelServer, ResolvedChannels,
    errors::KernelError,
    helpers::{flip_bits, format_u8_buf},
    router,
    sources::Source,
    types::{Channel, NodeHandle},
};
#[allow(unused_imports)] // Position is used by delivery.rs via `use super::*`
use config::ast::Position;
use config::ast::{ChannelKind, DataUnit, DistanceUnit, TimeUnit, TimestepConfig};
use rand::rngs::StdRng;
use std::rc::Rc;
use std::thread;
use std::{borrow::Cow, sync::mpsc};
use std::{cmp::Reverse, collections::BinaryHeap};
use std::{collections::HashMap, thread::JoinHandle};
use std::{collections::VecDeque, num::NonZeroU64};
use tracing::{Level, debug, event, info, instrument, warn};

mod control;
mod energy_tests;
mod posctl;
mod powerctl;
mod timectl;
use crate::types::{ChannelHandle, ChannelIdx};
use control::ControlFile;

pub type Timestep = u64;
pub type MessageQueue = BinaryHeap<(Reverse<Timestep>, usize, AddressedMsg)>;
pub type Mailbox = VecDeque<QueuedMessage>;

mod delivery;
mod errors;
mod link_simulation;
mod messages;
mod table;

use delivery::*;
pub use errors::*;
pub use messages::*;
use table::*;

type ServerHandle = JoinHandle<Result<(), KernelError>>;

impl KernelServer<ServerHandle, KernelMessage, RouterMessage> {
    /// Poll the routing server for one timestep and return energy events.
    pub fn poll(&mut self, timestep: u64) -> Result<RouterMessage, KernelError> {
        self.tx
            .send(router::KernelMessage::Poll(timestep))
            .map_err(|e| KernelError::RouterError(RouterError::KernelSendError(e)))?;
        self.rx
            .recv()
            .map_err(|e| KernelError::RouterError(RouterError::RecvError(e)))
    }
    pub fn remap_pids(&mut self, pairs: Vec<(u32, u32)>) -> Result<RouterMessage, KernelError> {
        self.tx
            .send(router::KernelMessage::RemapPids(pairs))
            .map_err(|e| KernelError::RouterError(RouterError::KernelSendError(e)))?;
        self.rx
            .recv()
            .map_err(|e| KernelError::RouterError(RouterError::RecvError(e)))
    }

    pub fn shutdown(self) -> Result<(), KernelError> {
        self.tx
            .send(router::KernelMessage::Shutdown)
            .map_err(|e| KernelError::RouterError(RouterError::KernelSendError(e)))?;
        self.handle.join().expect("thread panic!")
    }
}

#[derive(Debug)]
#[allow(dead_code)]
pub(crate) struct RoutingServer {
    /// Current simulation timestep.
    timestep: Timestep,
    /// Configuration for the timestep
    ts_config: TimestepConfig,
    /// Resolved channels, nodes, etc.
    channels: ResolvedChannels,
    /// Routing table with information with computed routes between nodes.
    routes: RoutingTable,
    /// AddressedMsgs queued to become active at a specific timestep.
    queued: MessageQueue,
    /// Mapping from channel keys used by FUSE to those used by the kernel.
    fuse_mapping: HashMap<fuse::ChannelId, usize>,
    /// Per-handle file mailbox with buffered messages ready to be read.
    /// Also contains an optional TTL which marks it as expired if it is in the
    /// past. Uses the niche optimization that the ttl for a channel cannot be
    /// 0, which means we can use an Option<T> here with no overhead!
    mailboxes: Vec<Mailbox>,
    /// Random number generator to use
    rng: StdRng,
    /// Channel which router delivers messages to file system
    tx: mpsc::Sender<fuse::KernelMessage>,
    /// Node indices that newly ran out of charge this timestep.
    newly_depleted: Vec<usize>,
    /// Node indices that recovered above their restart threshold this timestep.
    newly_recovered: Vec<usize>,
    /// Sender for (old_pid, new_pid) pairs consumed by the FUSE filesystem.
    remap_tx: mpsc::Sender<(u32, u32)>,
    /// Cached nanoseconds per timestep (constant for the simulation).
    timestep_ns: u64,
    /// Sequence counter for message ordering within a timestep.
    sequence: usize,
}

impl RoutingServer {
    /// Build the routing table during initialization.
    #[instrument]
    pub fn serve(
        tx: mpsc::Sender<fuse::KernelMessage>,
        channels: ResolvedChannels,
        ts_config: TimestepConfig,
        rng: StdRng,
        mut source: Source,
        remap_tx: mpsc::Sender<(u32, u32)>,
    ) -> Result<KernelServer<ServerHandle, KernelMessage, RouterMessage>, KernelError> {
        let (kernel_tx, kernel_rx) = mpsc::channel::<KernelMessage>();
        let (router_tx, router_rx) = mpsc::channel::<RouterMessage>();
        thread::Builder::new()
            .name("nexus_router".to_string())
            .spawn(move || {
                let fuse_mapping = channels.make_fuse_mapping();
                let handles_count = channels.handles.len();
                let routes = RoutingTable::new(&channels);
                let timestep_ns = ts_config.length.get() * ts_config.unit.to_ns_factor();
                let mut router = Self {
                    // This makes all the `NonZeroU64`s happy
                    timestep: 1,
                    channels,
                    routes,
                    queued: BinaryHeap::new(),
                    mailboxes: vec![VecDeque::new(); handles_count],
                    fuse_mapping,
                    ts_config,
                    rng,
                    tx,
                    newly_depleted: Vec::new(),
                    newly_recovered: Vec::new(),
                    remap_tx,
                    timestep_ns,
                    sequence: 0,
                };
                let mut last_polled_ts: u64 = u64::MAX;
                loop {
                    match kernel_rx.recv() {
                        Ok(KernelMessage::Shutdown) => {
                            return Ok(());
                        }
                        Ok(KernelMessage::RemapPids(pairs)) => {
                            router.apply_pid_remaps(&pairs);
                            if router_tx.send(RouterMessage::PidsRemapped).is_err() {
                                break Err(KernelError::RouterError(RouterError::RouteError));
                            }
                        }
                        Ok(KernelMessage::Poll(timestep)) => {
                            let ts_advanced = timestep != last_polled_ts;
                            last_polled_ts = timestep;
                            if let Err(e) = source.poll(&mut router, timestep, ts_advanced) {
                                break Err(KernelError::SourceError(e));
                            }
                            let depleted = router
                                .newly_depleted
                                .drain(..)
                                .map(|i| router.channels.node_names[i].clone())
                                .collect();
                            let recovered = router
                                .newly_recovered
                                .drain(..)
                                .map(|i| router.channels.node_names[i].clone())
                                .collect();
                            if router_tx
                                .send(RouterMessage::EnergyEvents {
                                    depleted,
                                    recovered,
                                })
                                .is_err()
                            {
                                break Err(KernelError::RouterError(RouterError::RouteError));
                            }
                        }
                        Err(e) => {
                            break Err(KernelError::RouterError(RouterError::RecvError(e)));
                        }
                    };
                }
            })
            .map_err(|e| KernelError::RouterError(RouterError::ThreadCreation(e)))
            .map(|handle| KernelServer::new(handle, kernel_tx, router_rx))
    }

    /// Apply PID remaps: update handles and fuse_mapping entries, clear
    /// mailboxes for affected handles, and push remaps to the shared FUSE queue.
    fn apply_pid_remaps(&mut self, pairs: &[(u32, u32)]) {
        for &(old_pid, new_pid) in pairs {
            for (idx, handle) in self.channels.handles.iter_mut().enumerate() {
                if handle.0 == old_pid {
                    handle.0 = new_pid;
                    self.mailboxes[idx].clear();
                    // Update fuse_mapping: remove old key, insert new one
                    let channel_name = self.channels.channel_names[handle.2.0].clone();
                    self.fuse_mapping.remove(&(old_pid, channel_name.clone()));
                    self.fuse_mapping.insert((new_pid, channel_name), idx);
                }
            }
        }
        // Send remaps to FUSE filesystem
        for &pair in pairs {
            if let Err(e) = self.remap_tx.send(pair) {
                warn!("remap channel disconnected, PID remap lost: {e}");
                break;
            }
        }
    }

    /// Map the ID communicated by the FUSE FS to a handle index
    fn get_handle_index(&self, id: &fuse::ChannelId) -> Option<usize> {
        self.fuse_mapping.get(id).copied()
    }

    pub fn write_control_file(
        &mut self,
        handle_index: usize,
        msg: fuse::Message,
    ) -> Result<(), RouterError> {
        let (_, node_index, _) = self.channels.handles[handle_index];
        let ni = node_index.0;
        let Some(ctl) = ControlFile::parse(&msg.id.1) else {
            return Err(RouterError::UnknownFile(msg.id.1.clone()));
        };
        match ctl {
            ControlFile::TimeUs | ControlFile::TimeMs | ControlFile::TimeS => {
                self.update_time(ni, msg)
            }
            ControlFile::EnergyState => {
                let state = String::from_utf8_lossy(&msg.data).trim().to_string();
                if let Some(energy) = &mut self.channels.nodes[ni].energy
                    && energy.power_states_nj.contains_key(&state)
                {
                    energy.current_state = Some(state);
                }
                Ok(())
            }
            ControlFile::PosDx | ControlFile::PosDy | ControlFile::PosDz => {
                self.write_pos_delta(ni, msg)
            }
            ControlFile::PosMotion => self.write_pos_motion(ni, msg),
            ControlFile::PosX
            | ControlFile::PosY
            | ControlFile::PosZ
            | ControlFile::PosAz
            | ControlFile::PosEl
            | ControlFile::PosRoll => self.write_pos(ni, msg),
            ControlFile::PowerFlows => self.write_power_flows(ni, msg),
            // Read-only files cannot be written
            ControlFile::EnergyLeft
            | ControlFile::ElapsedUs
            | ControlFile::ElapsedMs
            | ControlFile::ElapsedS => Err(RouterError::UnknownFile(msg.id.1.clone())),
        }
    }

    pub fn write_channel_file(
        &mut self,
        index: usize,
        msg: fuse::Message,
    ) -> Result<(), RouterError> {
        let (pid, src_node, channel_handle) = self.channels.handles[index];
        let channel_name = &self.channels.channel_names[channel_handle.0];
        let timestep = self.timestep;
        if tracing::enabled!(Level::INFO) {
            info!(
                "{:<30} [TX]: {}",
                format!(
                    "{}.{pid}.{channel_name}",
                    self.channels.node_names[src_node.0]
                ),
                format_u8_buf(&msg.data)
            );
        }
        event!(target: "tx", Level::INFO, timestep, channel = channel_handle.0, node = src_node.0, tx = true, data = msg.data.as_slice());

        // Queue the message first; only drain TX energy on success so that
        // a failed queue does not silently consume charge (BUG-9).
        self.queue_message(src_node, channel_handle, msg.data)?;

        let tx_cost_nj: u64 = self.channels.nodes[src_node.0]
            .channel_energy
            .get(&channel_handle)
            .and_then(|ce| ce.tx.as_ref())
            .map(|e| e.unit.to_nj(e.quantity))
            .unwrap_or(0);
        if tx_cost_nj > 0
            && let Some(energy) = &mut self.channels.nodes[src_node.0].energy
        {
            energy.charge_nj = energy.charge_nj.saturating_sub(tx_cost_nj);
        }

        Ok(())
    }

    /// Receive a message from the FS and post it to the mailboxes of any
    /// nodes listening on the channel.
    pub fn receive_write(&mut self, msg: fuse::Message) -> Result<(), RouterError> {
        let Some(channel_index) = self.get_handle_index(&msg.id) else {
            return Err(RouterError::UnknownFile(msg.id.1));
        };
        if ControlFile::parse(&msg.id.1).is_some() {
            self.write_control_file(channel_index, msg)
        } else {
            self.write_channel_file(channel_index, msg)
        }
    }

    pub fn read_control_file(
        &mut self,
        handle_index: usize,
        msg: fuse::Message,
    ) -> Result<(), RouterError> {
        let (_, node_index, _) = self.channels.handles[handle_index];
        let ni = node_index.0;
        let Some(ctl) = ControlFile::parse(&msg.id.1) else {
            return Err(RouterError::UnknownFile(msg.id.1.clone()));
        };
        match ctl {
            ControlFile::TimeUs | ControlFile::TimeMs | ControlFile::TimeS => {
                self.send_time(ni, msg)
            }
            ControlFile::ElapsedUs | ControlFile::ElapsedMs | ControlFile::ElapsedS => {
                self.send_elapsed(msg)
            }
            ControlFile::EnergyLeft => {
                let charge_nj = self.channels.nodes[ni]
                    .energy
                    .as_ref()
                    .map_or(0, |e| e.charge_nj);
                let mut msg = msg;
                msg.data = charge_nj.to_string().into_bytes();
                self.tx
                    .send(fuse::KernelMessage::Exclusive(msg))
                    .map_err(RouterError::FuseSendError)
            }
            ControlFile::EnergyState => {
                let state = self.channels.nodes[ni]
                    .energy
                    .as_ref()
                    .and_then(|e| e.current_state.clone())
                    .unwrap_or_default();
                let mut msg = msg;
                msg.data = state.into_bytes();
                self.tx
                    .send(fuse::KernelMessage::Exclusive(msg))
                    .map_err(RouterError::FuseSendError)
            }
            ControlFile::PosMotion => self.read_pos_motion(ni, msg),
            ControlFile::PosX
            | ControlFile::PosY
            | ControlFile::PosZ
            | ControlFile::PosAz
            | ControlFile::PosEl
            | ControlFile::PosRoll => self.read_pos(ni, msg),
            ControlFile::PowerFlows => self.read_power_flows(ni, msg),
            // Write-only files cannot be read
            ControlFile::PosDx | ControlFile::PosDy | ControlFile::PosDz => {
                Err(RouterError::UnknownFile(msg.id.1.clone()))
            }
        }
    }

    pub fn read_channel_file(
        &mut self,
        index: usize,
        msg: fuse::Message,
    ) -> Result<(), RouterError> {
        match self.deliver_msg(index) {
            Ok(true) => Ok(()),
            Ok(false) => self
                .tx
                .send(fuse::KernelMessage::Empty(msg))
                .map_err(RouterError::FuseSendError),
            Err(e) => Err(e),
        }
    }

    /// Wrapper function which will attempt to deliver any available messages
    /// to the ID identified in the message, but will send an "Empty" message
    /// if none is found.
    pub fn request_read(&mut self, msg: fuse::Message) -> Result<(), RouterError> {
        let Some(channel_index) = self.get_handle_index(&msg.id) else {
            return Err(RouterError::UnknownFile(msg.id.1));
        };
        if ControlFile::parse(&msg.id.1).is_some() {
            self.read_control_file(channel_index, msg)
        } else {
            self.read_channel_file(channel_index, msg)
        }
    }

    /// Microseconds per simulation step (derived from cached `timestep_ns`).
    fn us_per_step(&self) -> u64 {
        self.timestep_ns / 1000
    }

    /// Take a single step in the simulation.
    pub fn step(&mut self) -> Result<(), RouterError> {
        self.timestep += 1;
        self.tick_energy();
        self.apply_all_motions_and_log();
        self.expire_messages();
        self.deliver_queued_messages()
    }

    /// Apply per-timestep energy sources, sinks, and power-state drain.
    /// Detects death (charge == 0) and recovery (charge >= threshold).
    fn tick_energy(&mut self) {
        let current_time_us = self.timestep * self.timestep_ns / 1000;
        let timestep = self.timestep;

        for (node_idx, node) in self.channels.nodes.iter_mut().enumerate() {
            let Some(energy) = &mut node.energy else {
                continue;
            };
            let was_dead = energy.is_dead;

            // Sources always apply (even when dead, e.g. solar charging)
            let source_nj: u64 = energy
                .power_sources
                .iter()
                .map(|(_, flow)| flow.nj_per_timestep(current_time_us))
                .sum();
            energy.charge_nj += source_nj;

            // Sinks always apply (saturating keeps charge at 0)
            let sink_nj: u64 = energy
                .power_sinks
                .iter()
                .map(|(_, flow)| flow.nj_per_timestep(current_time_us))
                .sum();
            energy.charge_nj = energy.charge_nj.saturating_sub(sink_nj);

            // Power state drain only when alive
            if !was_dead {
                let drain = energy
                    .current_state
                    .as_deref()
                    .and_then(|s| energy.power_states_nj.get(s).copied())
                    .unwrap_or(0);
                energy.charge_nj = energy.charge_nj.saturating_sub(drain);
            }
            energy.charge_nj = energy.charge_nj.min(energy.max_nj);

            // Detect transitions
            if !was_dead && energy.charge_nj == 0 {
                energy.is_dead = true;
                self.newly_depleted.push(node_idx);
            } else if was_dead
                && energy
                    .restart_threshold_nj
                    .is_some_and(|t| energy.charge_nj >= t)
            {
                energy.is_dead = false;
                self.newly_recovered.push(node_idx);
            }

            let charge_nj = energy.charge_nj;
            event!(target: "battery", Level::INFO, timestep, node = node_idx as u64, charge_nj);
        }
    }

    /// Remove expired messages from all mailboxes.
    fn expire_messages(&mut self) {
        let timestep = self.timestep;
        for mailbox in self.mailboxes.iter_mut() {
            while mailbox
                .front()
                .is_some_and(|msg| msg.expiration.is_some_and(|exp| exp.get() < timestep))
            {
                let _ = mailbox.pop_front();
            }
        }
    }

    /// Deliver all messages whose activation timestep has arrived.
    fn deliver_queued_messages(&mut self) -> Result<(), RouterError> {
        while self
            .queued
            .peek()
            .is_some_and(|(ts, _, _)| ts.0 <= self.timestep)
        {
            let Some((_, _, frame)) = self.queued.pop() else {
                return Err(RouterError::StepError);
            };
            let (_, dst_node, channel_handle) = self.channels.handles[frame.handle_ptr];
            let mailbox = &mut self.mailboxes[frame.handle_ptr];
            let channel = &mut self.channels.channels[channel_handle.0];

            if channel
                .r#type
                .max_buffered()
                .is_none_or(|n| n.get() > mailbox.len())
            {
                mailbox.push_back(frame.msg);

                // Deduct RX channel energy cost on delivery
                let rx_cost_nj: u64 = self.channels.nodes[dst_node.0]
                    .channel_energy
                    .get(&channel_handle)
                    .and_then(|ce| ce.rx.as_ref())
                    .map(|e| e.unit.to_nj(e.quantity))
                    .unwrap_or(0);
                if rx_cost_nj > 0
                    && let Some(energy) = &mut self.channels.nodes[dst_node.0].energy
                {
                    energy.charge_nj = energy.charge_nj.saturating_sub(rx_cost_nj);
                }
            } else {
                warn!("Message dropped due to full queue!");
                event!(
                    target: "drop", Level::WARN,
                    timestep = self.timestep,
                    channel = channel_handle.0,
                    node = frame.msg.src.0,
                    reason = "buffer_full"
                );
            }
        }
        Ok(())
    }
}
