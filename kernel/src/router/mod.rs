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
    router::{self, timectl::SleepAlarm},
    sources::Source,
    types::{Channel, NodeHandle},
};
#[allow(unused_imports)] // Position is used by delivery.rs via `use super::*`
use config::ast::Position;
use config::{
    ast::{ChannelKind, DataUnit, DistanceUnit, TimeUnit, TimestepConfig},
    units::DecimalScaled,
};
use fuse::{SleepEvent, ctrl_files::ControlFile};
use rand::rngs::StdRng;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::{borrow::Cow, sync::mpsc};
use std::{cmp::Reverse, collections::BinaryHeap};
use std::{collections::HashMap, thread::JoinHandle};
use std::{collections::VecDeque, num::NonZeroU64};
use tracing::{Level, debug, event, info, instrument, warn};

mod energy;
mod energy_tests;
mod posctl;
mod powerctl;
mod timectl;
use crate::types::{ChannelHandle, ChannelIdx};

pub type Timestep = u64;
// Tuple ordering: (Reverse<Timestep>, Reverse<usize>, AddressedMsg).
// BinaryHeap is a max-heap, so Reverse(Timestep) ensures earlier
// timesteps pop first. The Reverse around the sequence number is the
// non-obvious bit: without it, two messages enqueued in the same
// timestep would pop in reverse insertion order (a max-heap on raw
// usize), silently scrambling multi-write payloads from one publisher.
// AddressedMsg is never compared (heap stops at the second tuple field
// because it's a strict total order on the prefix).
pub type MessageQueue = BinaryHeap<(Reverse<Timestep>, Reverse<usize>, AddressedMsg)>;
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

impl KernelServer<ServerHandle, RouterInput, RouterMessage> {
    /// Wake the router. Fire-and-forget: the router reads the current
    /// timestep from the shared atomic and processes any per-step
    /// bookkeeping. Energy events are pushed asynchronously on a separate
    /// channel.
    pub fn tick(&mut self) -> Result<(), KernelError> {
        self.tx
            .send(router::RouterInput::Tick)
            .map_err(|e| KernelError::RouterError(RouterError::KernelSendError(e)))
    }

    pub fn remap_pids(&mut self, pairs: Vec<(u32, u32)>) -> Result<RouterMessage, KernelError> {
        self.tx
            .send(router::RouterInput::RemapPids(pairs))
            .map_err(|e| KernelError::RouterError(RouterError::KernelSendError(e)))?;
        self.rx
            .recv()
            .map_err(|e| KernelError::RouterError(RouterError::RecvError(e)))
    }

    pub fn shutdown(self) -> Result<(), KernelError> {
        self.tx
            .send(router::RouterInput::Shutdown)
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
    /// Nested by PID so the inner map can be queried with `&str`, avoiding
    /// the String allocation a flat `HashMap<(PID, String), usize>` requires
    /// on every read/write request.
    fuse_mapping: HashMap<fuse::PID, HashMap<String, usize>>,
    /// Per-handle file mailbox with buffered messages ready to be read.
    /// Also contains an optional TTL which marks it as expired if it is in the
    /// past. Uses the niche optimization that the ttl for a channel cannot be
    /// 0, which means we can use an Option<T> here with no overhead!
    mailboxes: Vec<Mailbox>,
    /// Indices of mailboxes that currently hold ≥ 1 queued message. Lets the
    /// per-timestep TTL sweep skip the (potentially tens of thousands of)
    /// idle mailboxes that dominate cost at high node counts. Entries may be
    /// stale (mailbox already drained between sweeps); the sweep prunes those
    /// via `retain`. `mailbox_active` is the dedup bitmap that prevents the
    /// same mailbox from being pushed twice.
    nonempty_mailboxes: Vec<usize>,
    mailbox_active: Vec<bool>,
    /// Random number generator to use
    rng: StdRng,
    /// Channel which router delivers messages to file system
    tx: mpsc::Sender<fuse::KernelMessage>,
    /// Energy subsystem: tracks battery nodes, death/recovery transitions.
    energy_mgr: energy::EnergyManager,
    /// Sender for (old_pid, new_pid) pairs consumed by the FUSE filesystem.
    remap_tx: mpsc::Sender<(u32, u32)>,
    /// Cached nanoseconds per timestep (constant for the simulation).
    timestep_ns: u64,
    /// Sequence counter for message ordering within a timestep.
    sequence: usize,
    /// Monotonic counter for unique message IDs across the simulation.
    next_msg_id: u64,
    /// Per-handle signal quality from the last RX on each channel endpoint.
    signal_info: Vec<SignalInfo>,
    // Min-heap by `(timestep, pid)`: the `Reverse` is what makes
    // `peek`/`pop` return the *earliest* deadline first.
    sleep_alarms: BinaryHeap<Reverse<SleepAlarm>>,
}

/// Last-received signal quality for a (destination_node, channel) pair.
#[derive(Debug, Default, Clone)]
pub(crate) struct SignalInfo {
    pub(crate) rssi_dbm: f64,
    pub(crate) snr_db: f64,
}

impl RoutingServer {
    /// Build the routing table during initialization.
    ///
    /// `current_ts` is shared with the kernel main thread. The kernel writes
    /// it before sending Tick; the router reads it on Tick to learn the
    /// current simulated timestep without needing the value embedded in the
    /// message. `energy_tx` is the asynchronous push channel for depletion
    /// and recovery events; the kernel main thread drains its receiver once
    /// per tick.
    #[instrument(skip(
        tx,
        channels,
        rng,
        source,
        remap_tx,
        current_ts,
        energy_tx,
        kernel_tx,
        kernel_rx
    ))]
    pub fn serve(
        tx: mpsc::Sender<fuse::KernelMessage>,
        channels: ResolvedChannels,
        ts_config: TimestepConfig,
        rng: StdRng,
        mut source: Source,
        remap_tx: mpsc::Sender<(u32, u32)>,
        current_ts: Arc<AtomicU64>,
        energy_tx: mpsc::Sender<EnergyEvents>,
        kernel_tx: mpsc::Sender<RouterInput>,
        kernel_rx: mpsc::Receiver<RouterInput>,
    ) -> Result<KernelServer<ServerHandle, RouterInput, RouterMessage>, KernelError> {
        let (router_tx, router_rx) = mpsc::channel::<RouterMessage>();
        thread::Builder::new()
            .name("nexus_router".to_string())
            .spawn(move || {
                let fuse_mapping = channels.make_fuse_mapping();
                let handles_count = channels.handles.len();
                let routes = RoutingTable::new(&channels);
                let timestep_ns = ts_config.length.get() * ts_config.unit.to_ns_factor();
                let energy_mgr = energy::EnergyManager::new(&channels.nodes);
                let mut router = Self {
                    // This makes all the `NonZeroU64`s happy
                    timestep: 1,
                    channels,
                    routes,
                    queued: BinaryHeap::new(),
                    mailboxes: vec![VecDeque::new(); handles_count],
                    nonempty_mailboxes: Vec::new(),
                    mailbox_active: vec![false; handles_count],
                    fuse_mapping,
                    ts_config,
                    rng,
                    tx,
                    energy_mgr,
                    remap_tx,
                    timestep_ns,
                    sequence: 0,
                    next_msg_id: 0,
                    signal_info: vec![SignalInfo::default(); handles_count],
                    sleep_alarms: BinaryHeap::new(),
                };
                let mut last_polled_ts: u64 = u64::MAX;
                loop {
                    match kernel_rx.recv() {
                        Ok(RouterInput::Shutdown) => {
                            return Ok(());
                        }
                        Ok(RouterInput::RemapPids(pairs)) => {
                            router.apply_pid_remaps(&pairs);
                            if router_tx.send(RouterMessage::PidsRemapped).is_err() {
                                break Err(KernelError::RouterError(RouterError::RouteError));
                            }
                        }
                        Ok(RouterInput::Fs(fs_msg)) => {
                            // Direct dispatch of the FUSE event. Replaces the
                            // try_iter drain that used to happen on each Poll;
                            // the router now wakes on every FUSE op so reads
                            // get their reply in one mpsc round-trip rather
                            // than waiting for the next Tick.
                            let res = match fs_msg {
                                fuse::FsMessage::Write(msg) => router.receive_write(msg),
                                fuse::FsMessage::Read(msg) => router.request_read(msg),
                                fuse::FsMessage::Sleep(event) => {
                                    router.request_sleep(event);
                                    Ok(())
                                }
                            };
                            if let Err(e) = res {
                                break Err(KernelError::RouterError(e));
                            }
                        }
                        Ok(RouterInput::Tick) => {
                            let timestep = current_ts.load(Ordering::Acquire);
                            let ts_advanced = timestep != last_polled_ts;
                            last_polled_ts = timestep;
                            // Source::Simulated is now a no-op on poll
                            // because FUSE messages flow via RouterInput::Fs;
                            // for replay paths source.poll injects log records.
                            if let Err(e) = source.poll(&mut router, timestep, ts_advanced) {
                                break Err(KernelError::SourceError(e));
                            }
                            let depleted: Vec<String> = router
                                .energy_mgr
                                .newly_depleted
                                .drain(..)
                                .map(|i| router.channels.node_names[i].to_string())
                                .collect();
                            let recovered: Vec<String> = router
                                .energy_mgr
                                .newly_recovered
                                .drain(..)
                                .map(|i| router.channels.node_names[i].to_string())
                                .collect();
                            if !depleted.is_empty() || !recovered.is_empty() {
                                let events = EnergyEvents {
                                    depleted,
                                    recovered,
                                };
                                // Receiver is the kernel main thread; if it
                                // dropped we are shutting down anyway.
                                let _ = energy_tx.send(events);
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
            // Move the entire inner map under the new PID, then update the
            // handles whose PID matches.
            if let Some(inner) = self.fuse_mapping.remove(&old_pid) {
                self.fuse_mapping.insert(new_pid, inner);
            }
            for (idx, handle) in self.channels.handles.iter_mut().enumerate() {
                if handle.0 == old_pid {
                    handle.0 = new_pid;
                    self.mailboxes[idx].clear();
                    self.mailbox_active[idx] = false;
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

    /// Map the (PID, channel-name-as-str) pair communicated by the FUSE FS
    /// to a handle index. The lookup is borrowed-string, no allocation.
    fn get_handle_index(&self, pid: fuse::PID, name: &str) -> Option<usize> {
        self.fuse_mapping
            .get(&pid)
            .and_then(|m| m.get(name))
            .copied()
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
            ControlFile::Time(_) => self.update_time(ni, msg),
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
            ControlFile::EnergyLeft | ControlFile::Elapsed(_) => {
                Err(RouterError::UnknownFile(msg.id.1.clone()))
            }
            // Sleep variants are routed through `FsMessage::Sleep` rather
            // than the control-write path because they need to defer the
            // FUSE reply until the deadline. If we ever land here it means
            // the FS routed incorrectly; report unknown-file rather than
            // panicking the routing thread.
            ControlFile::SleepRelative(_) | ControlFile::SleepAbsolute(_) => {
                Err(RouterError::UnknownFile(msg.id.1.clone()))
            }
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

        // Queue the message first; only drain TX energy on success so that
        // a failed queue does not silently consume charge (BUG-9).
        self.queue_message(src_node, channel_handle, msg.data, msg_id)?;

        energy::EnergyManager::drain_tx(&mut self.channels.nodes, src_node.0, &channel_handle);

        Ok(())
    }

    /// Receive a message from the FS and post it to the mailboxes of any
    /// nodes listening on the channel.
    pub fn receive_write(&mut self, msg: fuse::Message) -> Result<(), RouterError> {
        let path = msg.id.1.as_str();
        // Strip "/channel" suffix for data channel writes; the lookup is
        // borrowed against the path slice with no allocation.
        let lookup_name: &str = path.strip_suffix("/channel").unwrap_or(path);
        let Some(channel_index) = self.get_handle_index(msg.id.0, lookup_name) else {
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
            ControlFile::Time(_) => self.send_time(ni, msg),
            ControlFile::Elapsed(_) => self.send_elapsed(msg),
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
            ControlFile::PosDx
            | ControlFile::PosDy
            | ControlFile::PosDz
            | ControlFile::SleepAbsolute(_)
            | ControlFile::SleepRelative(_) => Err(RouterError::UnknownFile(msg.id.1.clone())),
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
        let path = msg.id.1.as_str();

        // Handle signal quality reads (e.g., "lora/rssi", "lora/snr")
        if let Some(channel_name) = path.strip_suffix("/rssi") {
            let name = channel_name.to_string();
            return self.read_signal_file(&name, msg, |si| si.rssi_dbm);
        }
        if let Some(channel_name) = path.strip_suffix("/snr") {
            let name = channel_name.to_string();
            return self.read_signal_file(&name, msg, |si| si.snr_db);
        }

        // Strip "/channel" suffix for data channel reads
        let lookup_name: &str = path.strip_suffix("/channel").unwrap_or(path);
        let Some(channel_index) = self.get_handle_index(msg.id.0, lookup_name) else {
            return Err(RouterError::UnknownFile(msg.id.1.clone()));
        };
        if ControlFile::parse(path).is_some() {
            self.read_control_file(channel_index, msg)
        } else {
            self.read_channel_file(channel_index, msg)
        }
    }

    pub fn request_sleep(&mut self, req: SleepEvent) {
        // Convert the user-supplied value into the simulator's timestep unit.
        let (should_scale_down, ratio) = TimeUnit::ratio(req.unit, self.ts_config.unit);
        let scalar = 10u64
            .checked_pow(ratio.try_into().unwrap())
            .expect("Exponentiation overflow.");
        let sleep_val = if should_scale_down {
            req.val / scalar
        } else {
            req.val * scalar
        };
        let wakeup_timestep = if req.is_relative {
            sleep_val.saturating_add(self.timestep)
        } else {
            sleep_val
        };

        if wakeup_timestep <= self.timestep {
            req.reply.written(req.bytes_consumed);
        } else {
            self.sleep_alarms.push(Reverse(SleepAlarm {
                timestep: wakeup_timestep,
                pid: req.pid,
                bytes_consumed: req.bytes_consumed,
                reply: req.reply,
            }));
        }
    }

    /// Read the last-received signal quality value for a channel endpoint.
    fn read_signal_file(
        &mut self,
        channel_name: &str,
        mut msg: fuse::Message,
        extractor: impl Fn(&SignalInfo) -> f64,
    ) -> Result<(), RouterError> {
        let Some(handle_index) = self.get_handle_index(msg.id.0, channel_name) else {
            return Err(RouterError::UnknownFile(msg.id.1.clone()));
        };
        let value = extractor(&self.signal_info[handle_index]);
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
        self.energy_mgr
            .tick(&mut self.channels.nodes, self.timestep, self.timestep_ns);
        self.apply_all_motions_and_log();
        self.expire_messages();
        self.send_wakeups();
        self.deliver_queued_messages()
    }

    /// Remove expired messages from active mailboxes only. The
    /// `nonempty_mailboxes` index is rebuilt by `retain` so empty entries
    /// (drained by reads between sweeps) drop out and the sweep stays O(active)
    /// rather than O(total mailboxes).
    fn expire_messages(&mut self) {
        let timestep = self.timestep;
        let mailboxes = &mut self.mailboxes;
        let active = &mut self.mailbox_active;
        self.nonempty_mailboxes.retain(|&i| {
            let mailbox = &mut mailboxes[i];
            while mailbox
                .front()
                .is_some_and(|msg| msg.expiration.is_some_and(|exp| exp.get() < timestep))
            {
                let _ = mailbox.pop_front();
            }
            if mailbox.is_empty() {
                active[i] = false;
                false
            } else {
                true
            }
        });
    }

    fn send_wakeups(&mut self) {
        while let Some(Reverse(alarm)) = self.sleep_alarms.peek()
            && alarm.timestep <= self.timestep
        {
            let Reverse(alarm) = self.sleep_alarms.pop().unwrap();
            alarm.reply.written(alarm.bytes_consumed);
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
                let was_empty = mailbox.is_empty();
                mailbox.push_back(frame.msg);
                if was_empty && !self.mailbox_active[frame.handle_ptr] {
                    self.mailbox_active[frame.handle_ptr] = true;
                    self.nonempty_mailboxes.push(frame.handle_ptr);
                }

                // Deduct RX channel energy cost on delivery
                energy::EnergyManager::drain_rx(
                    &mut self.channels.nodes,
                    dst_node.0,
                    &channel_handle,
                );
            } else {
                warn!("Message dropped due to full queue!");
                event!(
                    target: "drop", Level::WARN,
                    timestep = self.timestep,
                    channel = channel_handle.0,
                    node = frame.msg.src.0,
                    msg_id = frame.msg.msg_id,
                    reason = "buffer_full"
                );
            }
        }
        Ok(())
    }
}
