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
use config::ast::{DataUnit, DistanceUnit, TimeUnit, TimestepConfig};
use rand::rngs::StdRng;
use std::sync::Arc;
use std::thread;
use std::{borrow::Cow, sync::mpsc};
use std::{cmp::Reverse, collections::BinaryHeap};
use std::{collections::HashMap, thread::JoinHandle};
use std::{collections::VecDeque, num::NonZeroU64};
use tracing::{Level, event, info, instrument, warn};

mod control;
mod energy;
mod energy_tests;
mod posctl;
mod powerctl;
mod timectl;
use crate::types::{ChannelHandle, ChannelIdx};
use control::ControlFile;

pub type Timestep = u64;
pub type MessageQueue = BinaryHeap<(Reverse<Timestep>, usize, AddressedMsg)>;
pub type Mailbox = VecDeque<QueuedMessage>;

pub(crate) mod coordinator;
mod delivery;
mod errors;
mod link_simulation;
mod messages;
mod partitioner;
mod table;
pub(crate) mod worker;

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
    /// Mapping from channel keys used by FUSE to those used by the kernel.
    fuse_mapping: HashMap<fuse::ChannelId, usize>,
    /// Random number generator to use
    rng: StdRng,
    /// Channel which router delivers messages to file system
    tx: mpsc::Sender<fuse::KernelMessage>,
    /// Sender for (old_pid, new_pid) pairs consumed by the FUSE filesystem.
    remap_tx: mpsc::Sender<(u32, u32)>,
    /// Cached nanoseconds per timestep (constant for the simulation).
    timestep_ns: u64,
    /// Monotonic counter for unique message IDs across the simulation.
    next_msg_id: u64,
    /// Coordinator managing workers for distributed message processing.
    coordinator: coordinator::Coordinator,
    /// Number of worker threads to use (1 = single-threaded, like before).
    num_workers: usize,
}

/// Last-received signal quality for a (destination_node, channel) pair.
#[derive(Debug, Default, Clone)]
pub(crate) struct SignalInfo {
    pub(crate) rssi_dbm: f64,
    pub(crate) snr_db: f64,
}

impl RoutingServer {
    /// Build the routing table during initialization (single-worker mode).
    #[allow(dead_code)]
    #[instrument]
    pub fn serve(
        tx: mpsc::Sender<fuse::KernelMessage>,
        channels: ResolvedChannels,
        ts_config: TimestepConfig,
        rng: StdRng,
        source: Source,
        remap_tx: mpsc::Sender<(u32, u32)>,
    ) -> Result<KernelServer<ServerHandle, KernelMessage, RouterMessage>, KernelError> {
        Self::serve_with_workers(tx, channels, ts_config, rng, source, remap_tx, 1)
    }

    /// Build the routing table and spawn workers for distributed simulation.
    #[instrument]
    pub fn serve_with_workers(
        tx: mpsc::Sender<fuse::KernelMessage>,
        channels: ResolvedChannels,
        ts_config: TimestepConfig,
        mut rng: StdRng,
        mut source: Source,
        remap_tx: mpsc::Sender<(u32, u32)>,
        num_workers: usize,
    ) -> Result<KernelServer<ServerHandle, KernelMessage, RouterMessage>, KernelError> {
        let (kernel_tx, kernel_rx) = mpsc::channel::<KernelMessage>();
        let (router_tx, router_rx) = mpsc::channel::<RouterMessage>();
        thread::Builder::new()
            .name("nexus_router".to_string())
            .spawn(move || {
                let fuse_mapping = channels.make_fuse_mapping();
                let routes = RoutingTable::new(&channels);
                let timestep_ns = ts_config.length.get() * ts_config.unit.to_ns_factor();
                let coordinator = coordinator::Coordinator::new(
                    num_workers.max(1),
                    &channels,
                    &mut rng,
                );
                let mut router = Self {
                    // This makes all the `NonZeroU64`s happy
                    timestep: 1,
                    channels,
                    routes,
                    fuse_mapping,
                    ts_config,
                    rng,
                    tx,
                    remap_tx,
                    timestep_ns,
                    next_msg_id: 0,
                    coordinator,
                    num_workers: num_workers.max(1),
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
                                .coordinator
                                .energy_mgr
                                .newly_depleted
                                .drain(..)
                                .map(|i| router.channels.node_names[i].to_string())
                                .collect();
                            let recovered = router
                                .coordinator
                                .energy_mgr
                                .newly_recovered
                                .drain(..)
                                .map(|i| router.channels.node_names[i].to_string())
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
            // Clear worker mailboxes for old PID before updating handles.
            self.coordinator
                .clear_mailboxes_for_pid(old_pid, &self.channels);
            for (idx, handle) in self.channels.handles.iter_mut().enumerate() {
                if handle.0 == old_pid {
                    handle.0 = new_pid;
                    // Update fuse_mapping: remove old key, insert new one
                    let channel_name = self.channels.channel_names[handle.2.0].to_string();
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
            ControlFile::TimeUs | ControlFile::TimeMs | ControlFile::TimeS | ControlFile::TimeNs => {
                self.update_time(ni, msg)
            }
            ControlFile::EnergyState => {
                let state = String::from_utf8_lossy(&msg.data).trim().to_string();
                if let Some(energy) = &mut self.channels.nodes[ni].energy {
                    energy::EnergyManager::set_state(energy, state);
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
            | ControlFile::ElapsedS
            | ControlFile::ElapsedNs => Err(RouterError::UnknownFile(msg.id.1.clone())),
        }
    }

    pub fn alloc_msg_id(&mut self) -> u64 {
        let id = self.next_msg_id;
        self.next_msg_id += 1;
        id
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
        let msg_id = self.next_msg_id;
        self.next_msg_id += 1;
        event!(target: "tx", Level::INFO, timestep, channel = channel_handle.0, node = src_node.0, tx = true, msg_id, data = msg.data.as_slice());

        // Queue the message via the appropriate worker.
        let worker = self.coordinator.worker_for_handle_mut(index);
        worker.queue_message(
            src_node,
            channel_handle,
            msg.data,
            msg_id,
            &self.channels,
            &self.routes.entries,
            timestep,
            self.ts_config,
        )?;

        energy::EnergyManager::drain_tx(&mut self.channels.nodes, src_node.0, &channel_handle);

        Ok(())
    }

    /// Receive a message from the FS and post it to the mailboxes of any
    /// nodes listening on the channel.
    pub fn receive_write(&mut self, msg: fuse::Message) -> Result<(), RouterError> {
        let path = &msg.id.1;
        // Strip "/channel" suffix for data channel writes
        let lookup_key = if let Some(channel_name) = path.strip_suffix("/channel") {
            (msg.id.0, channel_name.to_string())
        } else {
            msg.id.clone()
        };
        let Some(channel_index) = self.get_handle_index(&lookup_key) else {
            return Err(RouterError::UnknownFile(msg.id.1.clone()));
        };
        if ControlFile::parse(path).is_some() {
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
            ControlFile::TimeUs | ControlFile::TimeMs | ControlFile::TimeS | ControlFile::TimeNs => {
                self.send_time(ni, msg)
            }
            ControlFile::ElapsedUs
            | ControlFile::ElapsedMs
            | ControlFile::ElapsedS
            | ControlFile::ElapsedNs => self.send_elapsed(msg),
            ControlFile::EnergyLeft => {
                let charge_nj = energy::EnergyManager::charge_nj(&self.channels.nodes, ni);
                let mut msg = msg;
                msg.data = charge_nj.to_string().into_bytes();
                self.tx
                    .send(fuse::KernelMessage::Exclusive(msg))
                    .map_err(RouterError::FuseSendError)
            }
            ControlFile::EnergyState => {
                let state = energy::EnergyManager::current_state(&self.channels.nodes, ni)
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
        let worker = self.coordinator.worker_for_handle_mut(index);
        match worker.deliver_msg(index, &self.channels, self.timestep, &self.tx) {
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
        let path = msg.id.1.clone();

        // Handle signal quality reads (e.g., "lora/rssi", "lora/snr")
        if let Some(channel_name) = path.strip_suffix("/rssi") {
            return self.read_signal_file(channel_name, msg, |si| si.rssi_dbm);
        }
        if let Some(channel_name) = path.strip_suffix("/snr") {
            return self.read_signal_file(channel_name, msg, |si| si.snr_db);
        }

        // Strip "/channel" suffix for data channel reads
        let lookup_key = if let Some(channel_name) = path.strip_suffix("/channel") {
            (msg.id.0, channel_name.to_string())
        } else {
            msg.id.clone()
        };
        let Some(channel_index) = self.get_handle_index(&lookup_key) else {
            return Err(RouterError::UnknownFile(path));
        };
        if ControlFile::parse(&path).is_some() {
            self.read_control_file(channel_index, msg)
        } else {
            self.read_channel_file(channel_index, msg)
        }
    }

    /// Read the last-received signal quality value for a channel endpoint.
    fn read_signal_file(
        &mut self,
        channel_name: &str,
        mut msg: fuse::Message,
        extractor: impl Fn(&SignalInfo) -> f64,
    ) -> Result<(), RouterError> {
        let key = (msg.id.0, channel_name.to_string());
        let Some(&handle_index) = self.fuse_mapping.get(&key) else {
            return Err(RouterError::UnknownFile(msg.id.1.clone()));
        };
        let worker = self.coordinator.worker_for_handle_mut(handle_index);
        let value = extractor(&worker.signal_info[handle_index]);
        msg.data = format!("{value:.2}").into_bytes();
        self.tx
            .send(fuse::KernelMessage::Exclusive(msg))
            .map_err(RouterError::FuseSendError)
    }

    /// Microseconds per simulation step (derived from cached `timestep_ns`).
    fn us_per_step(&self) -> u64 {
        self.timestep_ns / 1000
    }

    /// Take a single step in the simulation.
    pub fn step(&mut self) -> Result<(), RouterError> {
        self.timestep += 1;
        self.apply_all_motions_and_log();
        self.coordinator
            .step(self.timestep, &mut self.channels, self.timestep_ns)
    }
}
