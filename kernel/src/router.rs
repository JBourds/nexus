use crate::{
    ChannelId,
    errors::RouterError,
    helpers::{flip_bits, format_u8_buf},
    types::{Channel, Node, NodeHandle},
};
use config::ast::{ChannelType, DataUnit, DistanceProbVar, DistanceUnit, Position};
use fuse::{fs::ControlSignal, socket};
use rand::rngs::StdRng;
use std::borrow::Cow;
use std::collections::HashMap;
use std::rc::Rc;
use std::{cmp::Reverse, collections::BinaryHeap};
use std::{collections::VecDeque, num::NonZeroU64, os::unix::net::UnixDatagram};
use tracing::{Level, debug, event, info, instrument, warn};

use crate::types::ChannelHandle;

pub type Timestep = u64;

#[derive(Clone, Debug, Eq, PartialOrd, Ord, PartialEq)]
pub(crate) struct AddressedMsg {
    handle_ptr: usize,
    msg: Msg,
}

#[derive(Clone, Debug, Eq, PartialOrd, Ord, PartialEq)]
pub(crate) struct Msg {
    src: NodeHandle,
    buf: Rc<[u8]>,
    expiration: Option<NonZeroU64>,
}

#[derive(Clone, Debug)]
pub(crate) struct Route {
    handle_ptr: usize,
    distance: f64,
    unit: DistanceUnit,
}

#[derive(Debug)]
#[allow(dead_code)]
pub(crate) struct Router {
    /// Current simulation timestep.
    timestep: Timestep,
    /// Nodes in the simulation.
    nodes: Vec<Node>,
    /// Names for nodes (only used in debugging/printing).
    node_names: Vec<String>,
    /// Channels in the simulation.
    channels: Vec<Channel>,
    /// Names for channels (only used in debugging/printing).
    channel_names: Vec<String>,
    /// Per-channel vector with the pre-computed route information,
    /// Maps each publisher from the channel to the map of subscribers -> routes.
    routes: Vec<HashMap<NodeHandle, Vec<Route>>>,
    /// Actual unix domain sockets being read/written from.
    endpoints: Vec<UnixDatagram>,
    /// All the unique keys for each channel file.
    handles: Vec<ChannelId>,
    /// AddressedMsgs queued to become active at a specific timestep.
    queued: BinaryHeap<(Reverse<Timestep>, AddressedMsg)>,
    /// Per-handle file mailbox with buffered messages ready to be read.
    /// Also contains an optional TTL which marks it as expired if it is in the
    /// past. Uses the niche optimization that the ttl for a channel cannot be
    /// 0, which means we can use an Option<T> here with no overhead!
    mailboxes: Vec<VecDeque<Msg>>,
    /// Random number generator to use
    rng: StdRng,
}

impl Router {
    /// Build the routing table during initialization.
    #[instrument]
    pub fn new(
        nodes: Vec<Node>,
        node_names: Vec<String>,
        channels: Vec<Channel>,
        channel_names: Vec<String>,
        handles: Vec<ChannelId>,
        endpoints: Vec<UnixDatagram>,
        rng: StdRng,
    ) -> Self {
        let handles_count = handles.len();
        let routes = channels
            .iter()
            .enumerate()
            .map(|(ch_index, ch)| {
                // For every channel, map every publishing node to the set of
                // precomputed routes it has with every receiving node
                ch.publishers
                    .iter()
                    .map(|src_node| {
                        (
                            *src_node,
                            handles
                                .iter()
                                .enumerate()
                                .filter_map(|(handle_ptr, (_, dst_node, dst_ch))| {
                                    if ch_index == *dst_ch
                                        && (ch.subscribers.contains(dst_node)
                                            || *src_node == *dst_node
                                                && ch.r#type.delivers_to_self())
                                    {
                                        let src = &nodes[*src_node];
                                        let dst = &nodes[*dst_node];
                                        let (distance, unit) =
                                            Position::distance(&src.position, &dst.position);
                                        Some(Route {
                                            handle_ptr,
                                            distance,
                                            unit,
                                        })
                                    } else {
                                        None
                                    }
                                })
                                .collect::<Vec<_>>(),
                        )
                    })
                    .collect::<HashMap<_, _>>()
            })
            .collect::<Vec<_>>();

        Self {
            timestep: 0,
            nodes,
            node_names,
            channels,
            channel_names,
            routes,
            handles,
            queued: BinaryHeap::new(),
            mailboxes: vec![VecDeque::new(); handles_count],
            endpoints,
            rng,
        }
    }

    pub fn receive_write(&mut self, index: usize) -> Result<(), RouterError> {
        let (pid, src_node, channel_handle) = self.handles[index];
        let channel_name = &self.channel_names[channel_handle];
        let channel = &mut self.channels[channel_handle];
        let buf_sz = channel.r#type.max_buf_size();
        let endpoint = &mut self.endpoints[index];
        loop {
            match Self::recv_msg(
                endpoint,
                buf_sz,
                pid,
                self.timestep,
                channel_handle,
                channel_name,
            ) {
                Ok(recv_buf) => {
                    info!(
                        "{:<30} [TX]: {}",
                        format!("{}.{pid}.{channel_name}", self.node_names[src_node]),
                        format_u8_buf(&recv_buf)
                    );
                    let sz: u64 = recv_buf
                        .len()
                        .try_into()
                        .expect("usize should be able to become a u64");
                    match channel.r#type {
                        // Use a "lazy" message where we clone the RC and only
                        // simulate the link when a read request is made for
                        // a shared link. The mailbox in this case is used as
                        // a list of messages which are active at once.
                        ChannelType::Shared { .. } => {
                            let buf: Rc<[u8]> = recv_buf.into();
                            for Route {
                                handle_ptr,
                                distance,
                                unit: distance_unit,
                            } in self.routes[channel_handle][&src_node].iter()
                            {
                                let dst_node = self.handles[*handle_ptr].1;
                                if dst_node != src_node || channel.r#type.delivers_to_self() {
                                    debug!(
                                        "Delivering from {} to {}",
                                        &self.node_names[src_node], &self.node_names[dst_node]
                                    );
                                    let (becomes_active_at, expiration) = Self::message_timesteps(
                                        channel,
                                        self.timestep,
                                        sz,
                                        *distance,
                                        *distance_unit,
                                    );
                                    let msg = AddressedMsg {
                                        handle_ptr: *handle_ptr,
                                        msg: Msg {
                                            src: src_node,
                                            buf: Rc::clone(&buf),
                                            expiration,
                                        },
                                    };
                                    self.queued.push((Reverse(becomes_active_at), msg));
                                }
                            }
                        }
                        // The message must be delivered to every subscriber, so
                        // make copies of the data now to apply link simulation
                        ChannelType::Exclusive { .. } => {
                            for Route {
                                handle_ptr,
                                distance,
                                unit: distance_unit,
                            } in self.routes[channel_handle][&src_node].iter()
                            {
                                let dst_node = self.handles[*handle_ptr].1;
                                if dst_node != src_node || channel.r#type.delivers_to_self() {
                                    debug!(
                                        "Delivering from {} to {}",
                                        &self.node_names[src_node], &self.node_names[dst_node]
                                    );
                                    if let Some(buf) = Self::send_through_channel(
                                        channel,
                                        Cow::from(&recv_buf),
                                        *distance,
                                        *distance_unit,
                                        &mut self.rng,
                                    ) {
                                        let (becomes_active_at, expiration) =
                                            Self::message_timesteps(
                                                channel,
                                                self.timestep,
                                                sz,
                                                *distance,
                                                *distance_unit,
                                            );
                                        let msg = AddressedMsg {
                                            handle_ptr: *handle_ptr,
                                            msg: Msg {
                                                src: src_node,
                                                buf: buf.into(),
                                                expiration,
                                            },
                                        };
                                        self.queued.push((Reverse(becomes_active_at), msg));
                                    }
                                }
                            }
                        }
                    }
                }
                Err(e) if e.recoverable() => {
                    break;
                }
                Err(e) => {
                    return Err(e);
                }
            }
        }
        Ok(())
    }

    pub fn deliver_msg(&mut self, index: usize) -> Result<ControlSignal, RouterError> {
        let mailbox = &mut self.mailboxes[index];
        let endpoint = &mut self.endpoints[index];
        let (pid, node_handle, channel_handle) = self.handles[index];
        let channel = &mut self.channels[channel_handle];
        let channel_name = &self.channel_names[channel_handle];
        let timestep = self.timestep;

        match &channel.r#type {
            // Query the current data present in the medium.
            ChannelType::Shared { max_size, .. } => {
                if mailbox.is_empty() {
                    return Ok(ControlSignal::Nothing);
                }

                match mailbox.len().cmp(&1) {
                    std::cmp::Ordering::Less => Ok(ControlSignal::Nothing),
                    std::cmp::Ordering::Equal => {
                        let msg = mailbox.front().unwrap();
                        let Route { distance, unit, .. } =
                            self.routes[channel_handle][&msg.src][node_handle];
                        if let Some(buf) = Self::send_through_channel(
                            channel,
                            Cow::from(msg.buf.as_ref()),
                            distance,
                            unit,
                            &mut self.rng,
                        ) {
                            match Self::send_msg(
                                endpoint,
                                &buf,
                                pid,
                                timestep,
                                channel_handle,
                                channel_name,
                            ) {
                                Ok(_) => Ok(ControlSignal::Exclusive),
                                Err(e) if e.recoverable() => Ok(ControlSignal::Nothing),
                                Err(e) => Err(RouterError::SendError {
                                    sender: pid,
                                    node_name: self.node_names[node_handle].clone(),
                                    channel_name: self.channel_names[channel_handle].clone(),
                                    timestep,
                                    base: Box::new(e),
                                }),
                            }
                        } else {
                            Ok(ControlSignal::Nothing)
                        }
                    }
                    std::cmp::Ordering::Greater => {
                        // See what messages reach the requester
                        let filtered = mailbox.iter().filter_map(|msg| {
                            let Route { distance, unit, .. } =
                                self.routes[channel_handle][&msg.src][node_handle];
                            Self::send_through_channel(
                                channel,
                                Cow::from(msg.buf.as_ref()),
                                distance,
                                unit,
                                &mut self.rng,
                            )
                        });
                        // Combine all the signals together
                        let buf = filtered.fold(
                            Vec::with_capacity(max_size.get().try_into().unwrap()),
                            |mut v, msg| {
                                let smaller_index = std::cmp::min(v.len(), msg.len());
                                for i in 0..smaller_index {
                                    v[i] |= msg[i];
                                }
                                v.extend_from_slice(&msg[smaller_index..]);
                                v
                            },
                        );
                        match Self::send_msg(
                            endpoint,
                            &buf,
                            pid,
                            timestep,
                            channel_handle,
                            channel_name,
                        ) {
                            Ok(_) => Ok(ControlSignal::Exclusive),
                            Err(e) if e.recoverable() => Ok(ControlSignal::Nothing),
                            Err(e) => Err(RouterError::SendError {
                                sender: pid,
                                node_name: self.node_names[node_handle].clone(),
                                channel_name: self.channel_names[channel_handle].clone(),
                                timestep,
                                base: Box::new(e),
                            }),
                        }
                    }
                }
            }
            ChannelType::Exclusive { .. } => {
                // Keep trying to send until we either get an unexpired message or error
                while let Some(msg) = mailbox.pop_front() {
                    info!(
                        "{:<30} [RX]: {} <Now: {}, Expiration: {:?}>",
                        format!("{}.{pid}.{channel_name}", self.node_names[node_handle]),
                        format_u8_buf(&msg.buf),
                        self.timestep,
                        msg.expiration,
                    );
                    if msg.expiration.is_some_and(|exp| exp.get() < self.timestep) {
                        warn!(
                            "AddressedMsg dropped due to timeout (Now: {}, Expiration: {})!",
                            self.timestep,
                            msg.expiration.unwrap().get()
                        );
                        continue;
                    }
                    match Self::send_msg(
                        endpoint,
                        &msg.buf,
                        pid,
                        timestep,
                        channel_handle,
                        channel_name,
                    ) {
                        Ok(_) => {
                            return Ok(ControlSignal::Exclusive);
                        }
                        Err(e) if e.recoverable() => {
                            mailbox.push_front(msg);
                            break;
                        }
                        Err(e) => {
                            return Err(RouterError::SendError {
                                sender: pid,
                                node_name: self.node_names[node_handle].clone(),
                                channel_name: self.channel_names[channel_handle].clone(),
                                timestep,
                                base: Box::new(e),
                            });
                        }
                    }
                }
                Ok(ControlSignal::Nothing)
            }
        }
    }

    /// Take a single step in the simulation, moving all queued messages to
    /// their destination. Check for whether a channel's queue is full before
    /// placing it in the mailbox.
    pub fn step(&mut self) -> Result<(), RouterError> {
        self.timestep += 1;
        while self
            .queued
            .peek()
            .is_some_and(|(ts, _)| ts.0 <= self.timestep)
        {
            let Some((_, frame)) = self.queued.pop() else {
                return Err(RouterError::StepError);
            };
            let (_, _, channel_index) = self.handles[frame.handle_ptr];
            // TODO: Better way to make sure we don't count old messages without
            // requiring a linear operation every timestep on every message
            let mut mailbox = std::mem::take(&mut self.mailboxes[frame.handle_ptr])
                .into_iter()
                .filter(|msg| msg.expiration.is_none_or(|exp| exp.get() >= self.timestep))
                .collect::<VecDeque<_>>();

            // Once the write to a shared channel has finished simulating the
            // link delays, it resolves what should be in the medium
            let channel = &mut self.channels[channel_index];
            if channel
                .r#type
                .max_buffered()
                .is_none_or(|n| n.get() as usize > mailbox.len())
            {
                mailbox.push_back(frame.msg);
            } else {
                warn!("AddressedMsg dropped due to full queue!");
            }
            let _ = std::mem::replace(&mut self.mailboxes[frame.handle_ptr], mailbox);
        }
        Ok(())
    }

    /// Perform link simulation for:
    /// - dropped packets
    /// - bit errors
    fn send_through_channel<'a>(
        channel: &Channel,
        mut buf: Cow<'a, [u8]>,
        distance: f64,
        distance_unit: DistanceUnit,
        rng: &mut StdRng,
    ) -> Option<Cow<'a, [u8]>> {
        let sz: u64 = buf
            .len()
            .try_into()
            .expect("usize should be able to become a u64");
        let mut sample =
            |var: &DistanceProbVar| var.sample(distance, distance_unit, sz, DataUnit::Byte, rng);
        if sample(&channel.link.packet_loss) {
            warn!("Packet dropped");
            return None;
        }

        let bit_error_prob =
            channel
                .link
                .bit_error
                .probability(distance, distance_unit, sz, DataUnit::Byte);
        if bit_error_prob != 0.0 {
            let flips = (0..buf.len() * usize::try_from(u8::BITS).unwrap())
                .map(|_| unsafe { channel.link.bit_error.sample_unchecked(bit_error_prob, rng) });
            let _ = flip_bits(buf.to_mut(), flips);
        }
        Some(buf)
    }

    /// Calculate the timesteps at which the message should be moved to its
    /// destination and, optionally (if ttl is specified), its expiration.
    fn message_timesteps(
        channel: &Channel,
        timestep: u64,
        sz: u64,
        distance: f64,
        distance_unit: DistanceUnit,
    ) -> (Timestep, Option<NonZeroU64>) {
        let unit = DataUnit::Byte;
        let delays = &channel.link.delays;
        let becomes_active_at = timestep
            + delays.transmission_timesteps_f64(sz, unit).round() as u64
            + delays
                .propagation_timesteps_f64(distance, distance_unit)
                .round() as u64
            + delays.processing_timesteps_f64(sz, unit).round() as u64;
        let expiration = channel
            .r#type
            .ttl()
            .map(|ttl| ttl.saturating_add(becomes_active_at));
        (becomes_active_at, expiration)
    }

    #[instrument(skip(socket, data), err)]
    fn send_msg<A: AsRef<str> + std::fmt::Debug>(
        socket: &mut UnixDatagram,
        data: &[u8],
        pid: fuse::PID,
        timestep: u64,
        channel: ChannelHandle,
        channel_name: &A,
    ) -> Result<usize, RouterError> {
        match socket::send(socket, data, pid, channel_name).map_err(RouterError::FileError) {
            Ok(n_sent) => {
                event!(target: "tx", Level::INFO, timestep, channel, pid, tx = true, data);
                Ok(n_sent)
            }
            err => err,
        }
    }

    #[instrument(skip(socket))]
    fn recv_into<A: AsRef<str> + std::fmt::Debug>(
        socket: &mut UnixDatagram,
        buf: &mut Vec<u8>,
        pid: fuse::PID,
        timestep: u64,
        channel: ChannelHandle,
        channel_name: &A,
    ) -> Result<(), RouterError> {
        let nread = socket::recv(socket, buf, pid, channel_name).map_err(RouterError::FileError)?;
        buf.truncate(nread);
        event!(target: "rx", Level::INFO, timestep, channel, pid, tx = false, data = buf.as_slice());
        Ok(())
    }

    #[instrument(skip(socket))]
    fn recv_msg<A: AsRef<str> + std::fmt::Debug>(
        socket: &mut UnixDatagram,
        buf_sz: NonZeroU64,
        pid: fuse::PID,
        timestep: u64,
        channel: ChannelHandle,
        channel_name: &A,
    ) -> Result<Vec<u8>, RouterError> {
        let mut recv_buf = vec![0; buf_sz.get() as usize];
        Self::recv_into(socket, &mut recv_buf, pid, timestep, channel, channel_name)
            .map(|_| recv_buf)
    }
}
