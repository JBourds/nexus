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
use config::ast::{
    ChannelType, DataUnit, DistanceProbVar, DistanceUnit, Position, TimeUnit, TimestepConfig,
};
use rand::rngs::StdRng;
use std::rc::Rc;
use std::thread;
use std::{borrow::Cow, sync::mpsc};
use std::{cmp::Reverse, collections::BinaryHeap};
use std::{collections::HashMap, thread::JoinHandle};
use std::{collections::VecDeque, num::NonZeroU64};
use tracing::{Level, debug, event, info, instrument, warn};

use crate::types::ChannelHandle;

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
    pub fn poll(&mut self, timestep: u64) -> Result<(), KernelError> {
        self.tx
            .send(router::KernelMessage::Poll(timestep))
            .map_err(|e| KernelError::RouterError(RouterError::KernelSendError(e)))
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
    ) -> Result<KernelServer<ServerHandle, KernelMessage, RouterMessage>, KernelError> {
        let (kernel_tx, kernel_rx) = mpsc::channel::<KernelMessage>();
        let (_router_tx, router_rx) = mpsc::channel::<RouterMessage>();
        thread::Builder::new()
            .name("nexus_router".to_string())
            .spawn(move || {
                let fuse_mapping = channels.make_fuse_mapping();
                let handles_count = channels.handles.len();
                let routes = RoutingTable::new(&channels);
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
                };
                loop {
                    match kernel_rx.recv() {
                        Ok(KernelMessage::Shutdown) => {
                            return Ok(());
                        }
                        Ok(KernelMessage::Poll(timestep)) => {
                            router.timestep = timestep;
                            if let Err(e) = source.poll(&mut router, timestep) {
                                break Err(KernelError::SourceError(e));
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

    /// Map the ID communicated by the FUSE FS to a handle index
    fn get_handle_index(&self, id: &fuse::ChannelId) -> usize {
        self.fuse_mapping[id]
    }

    /// Receive a message from the FS and post it to the mailboxes of any
    /// nodes listening on the channel.
    pub fn receive_write(&mut self, msg: fuse::Message) -> Result<(), RouterError> {
        let index = self.get_handle_index(&msg.id);
        let (pid, src_node, channel_handle) = self.channels.handles[index];
        let channel_name = &self.channels.channel_names[channel_handle];
        let timestep = self.timestep;
        info!(
            "{:<30} [TX]: {}",
            format!(
                "{}.{pid}.{channel_name}",
                self.channels.node_names[src_node]
            ),
            format_u8_buf(&msg.data)
        );

        event!(target: "tx", Level::INFO, timestep, channel = channel_handle, node = src_node, tx = true, data = msg.data.as_slice());
        self.queue_message(src_node, channel_handle, msg.data)
    }

    fn suffix_to_time(s: &str) -> Option<TimeUnit> {
        match &s[s.len() - 2..s.len()] {
            "us" => Some(TimeUnit::Microseconds),
            "ms" => Some(TimeUnit::Milliseconds),
            "_s" => Some(TimeUnit::Seconds),
            _ => None,
        }
    }

    pub fn send_time(&mut self, mut msg: fuse::Message) -> Result<(), RouterError> {
        let unit = Self::suffix_to_time(msg.id.1.as_str()).expect("invalid time unit");
        let s = self.ts_config.time(self.timestep, unit).to_string();
        msg.data = s.bytes().collect();
        self.tx
            .send(fuse::KernelMessage::Exclusive(msg))
            .map_err(RouterError::FuseSendError)
    }

    pub fn send_elapsed(&mut self, mut msg: fuse::Message) -> Result<(), RouterError> {
        let unit = Self::suffix_to_time(msg.id.1.as_str()).expect("invalid time unit");
        let s = self.ts_config.elapsed(self.timestep, unit).to_string();
        msg.data = s.bytes().collect();
        self.tx
            .send(fuse::KernelMessage::Exclusive(msg))
            .map_err(RouterError::FuseSendError)
    }

    /// Wrapper function which will attempt to deliver any available messages
    /// to the ID identified in the message, but will send an "Empty" message
    /// if none is found.
    pub fn request_read(&mut self, msg: fuse::Message) -> Result<(), RouterError> {
        match self.deliver_msg(self.get_handle_index(&msg.id)) {
            Ok(true) => Ok(()),
            Ok(false) => self
                .tx
                .send(fuse::KernelMessage::Empty(msg))
                .map_err(RouterError::FuseSendError),
            Err(e) => Err(e),
        }
    }

    /// Take a single step in the simulation, moving all queued messages to
    /// their destination. Check for whether a channel's queue is full before
    /// placing it in the mailbox.
    pub fn step(&mut self) -> Result<(), RouterError> {
        self.timestep += 1;

        // Clear all old messages
        for mailbox in self.mailboxes.iter_mut() {
            while mailbox
                .front()
                .is_some_and(|msg| msg.expiration.is_some_and(|exp| exp.get() < self.timestep))
            {
                let _ = mailbox.pop_front();
            }
        }

        // Deliver all messages which should now be active
        while self
            .queued
            .peek()
            .is_some_and(|(ts, _, _)| ts.0 <= self.timestep)
        {
            let Some((_, _, frame)) = self.queued.pop() else {
                return Err(RouterError::StepError);
            };
            let (_, _, channel_index) = self.channels.handles[frame.handle_ptr];
            let mailbox = &mut self.mailboxes[frame.handle_ptr];

            // Once the write to a shared channel has finished simulating the
            // link delays, it resolves what should be in the medium
            let channel = &mut self.channels.channels[channel_index];
            if channel
                .r#type
                .max_buffered()
                .is_none_or(|n| n.get() > mailbox.len())
            {
                mailbox.push_back(frame.msg);
            } else {
                warn!("Message dropped due to full queue!");
            }
        }
        Ok(())
    }
}
