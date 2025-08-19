use crate::{
    ChannelId,
    errors::RouterError,
    helpers::format_u8_buf,
    types::{Channel, Node},
};
use config::ast::{DataUnit, DistanceProbVar, DistanceUnit, Position};
use fuse::socket;
use rand::rngs::StdRng;
use std::{cmp::Reverse, collections::BinaryHeap};
use std::{collections::VecDeque, num::NonZeroU64, os::unix::net::UnixDatagram};
use tracing::{Level, debug, event, info, instrument, warn};

use crate::types::ChannelHandle;

pub type Timestep = u64;

#[derive(Clone, Debug, Eq, PartialOrd, Ord, PartialEq)]
pub(crate) struct Message {
    handle_ptr: usize,
    buf: Vec<u8>,
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
    routes: Vec<Vec<Route>>,
    /// Actual unix domain sockets being read/written from.
    endpoints: Vec<UnixDatagram>,
    /// All the unique keys for each channel file.
    handles: Vec<ChannelId>,
    /// Messages in the "transmitting" stage. Contains the timestep
    /// the message should be removed from the queue and the message.
    /// Also contains the timestep the message should be removed from
    /// the next two queues.
    transmitting: BinaryHeap<(
        Reverse<Timestep>,
        Reverse<Timestep>,
        Reverse<Timestep>,
        Option<NonZeroU64>,
        Message,
    )>,
    /// Messages in the "propagating" stage. Contains the timestep
    /// the message should be removed from the queue and the message.
    /// Also contains the timestep the message should be removed from
    /// the processing queue.
    propagating: BinaryHeap<(
        Reverse<Timestep>,
        Reverse<Timestep>,
        Option<NonZeroU64>,
        Message,
    )>,
    /// Messages in the "processing" stage. Simulates delay after
    /// reception from destination. Contains the timestep
    /// the message should be removed from the queue and the message.
    processing: BinaryHeap<(Reverse<Timestep>, Option<NonZeroU64>, Message)>,
    /// Per-handle file mailbox with buffered messages ready to be read.
    /// Also contains an optional TTL which marks it as expired if it is in the
    /// past. Uses the niche optimization that the ttl for a channel cannot be
    /// 0, which means we can use an Option<T> here with no overhead!
    mailboxes: Vec<VecDeque<(Option<NonZeroU64>, Vec<u8>)>>,
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
                ch.inbound
                    .iter()
                    .flat_map(|node_handle| {
                        handles
                            .iter()
                            .enumerate()
                            .filter_map(|(index, (_, dst_node, dst_ch))| {
                                if *dst_node == *node_handle && *dst_ch == ch_index {
                                    let src = &nodes[*node_handle];
                                    let dst = &nodes[*dst_node];
                                    let (distance, unit) =
                                        Position::distance(&src.position, &dst.position);
                                    Some(Route {
                                        handle_ptr: index,
                                        distance,
                                        unit,
                                    })
                                } else {
                                    None
                                }
                            })
                    })
                    .collect::<Vec<_>>()
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
            transmitting: BinaryHeap::new(),
            propagating: BinaryHeap::new(),
            processing: BinaryHeap::new(),
            mailboxes: vec![VecDeque::new(); handles_count],
            endpoints,
            rng,
        }
    }

    #[instrument(skip_all)]
    pub fn inbound(&mut self, index: usize) -> Result<(), RouterError> {
        let (pid, src_node, channel_handle) = self.handles[index];
        let channel_name = &self.channel_names[channel_handle];
        let channel = &self.channels[channel_handle];
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
                        "[TX] {src_node}.{pid}.{channel_name}: {}",
                        format_u8_buf(&recv_buf)
                    );
                    match channel.r#type {
                        config::ast::ChannelType::Shared { .. } => unimplemented!(),
                        config::ast::ChannelType::Exclusive { .. } => {
                            for Route {
                                handle_ptr,
                                distance,
                                unit: distance_unit,
                            } in self.routes[channel_handle].iter()
                            {
                                let msg = Message {
                                    handle_ptr: *handle_ptr,
                                    buf: recv_buf.clone(),
                                };
                                let dst_node = self.handles[*handle_ptr].1;
                                if dst_node != src_node || channel.r#type.delivers_to_self() {
                                    debug!(
                                        "Delivering from {} to {}",
                                        &self.node_names[src_node], &self.node_names[dst_node]
                                    );
                                    if let Some(entry) = Self::prepare_message(
                                        channel,
                                        self.timestep,
                                        msg.clone(),
                                        *distance,
                                        *distance_unit,
                                        &mut self.rng,
                                    ) {
                                        self.transmitting.push(entry);
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

    pub fn outbound(&mut self, index: usize) -> Result<(), RouterError> {
        let mailbox = &mut self.mailboxes[index];
        let endpoint = &mut self.endpoints[index];
        let (pid, node_handle, channel_handle) = self.handles[index];
        let channel_name = &self.channel_names[channel_handle];
        let timestep = self.timestep;
        // Keep trying to send until we either get an unexpired message or error
        while let Some((expiration, msg)) = mailbox.pop_front() {
            info!(
                "{pid}.{channel_name} <Now: {}, Expiration: {expiration:?}> [RX]: {}",
                self.timestep,
                format_u8_buf(&msg)
            );
            if expiration.is_some_and(|exp| exp.get() < self.timestep) {
                warn!(
                    "Message dropped due to timeout (Now: {}, Expiration: {})!",
                    self.timestep,
                    expiration.unwrap().get()
                );
                continue;
            }
            match Self::send_msg(endpoint, &msg, pid, timestep, channel_handle, channel_name) {
                Ok(_) => {
                    break;
                }
                Err(e) if e.recoverable() => {
                    mailbox.push_front((expiration, msg));
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
        Ok(())
    }

    pub fn step(&mut self) -> Result<(), RouterError> {
        self.timestep += 1;
        while self
            .transmitting
            .peek()
            .is_some_and(|(ts, _, _, _, _)| ts.0 <= self.timestep)
        {
            let Some((_, prop_ts, proc_ts, expiration, msg)) = self.transmitting.pop() else {
                return Err(RouterError::StepError);
            };
            self.propagating.push((prop_ts, proc_ts, expiration, msg));
        }
        while self
            .propagating
            .peek()
            .is_some_and(|(ts, _, _, _)| ts.0 <= self.timestep)
        {
            let Some((_, proc_ts, expiration, msg)) = self.propagating.pop() else {
                return Err(RouterError::StepError);
            };
            self.processing.push((proc_ts, expiration, msg));
        }
        while self
            .processing
            .peek()
            .is_some_and(|(ts, _, _)| ts.0 <= self.timestep)
        {
            let Some((_, expiration, msg)) = self.processing.pop() else {
                return Err(RouterError::StepError);
            };
            let (_, _, channel_index) = self.handles[msg.handle_ptr];
            // TODO: Better way to make sure we don't count old messages without
            // requiring a linear operation every timestep on every message
            let mut mailbox = std::mem::take(&mut self.mailboxes[msg.handle_ptr])
                .into_iter()
                .filter(|(exp, _)| exp.is_none_or(|exp| exp.get() >= self.timestep))
                .collect::<VecDeque<_>>();
            if self.channels[channel_index]
                .r#type
                .max_buffered()
                .is_none_or(|n| n.get() as usize > mailbox.len())
            {
                mailbox.push_back((expiration, msg.buf));
            } else {
                warn!("Message dropped due to full queue!");
            }
            let _ = std::mem::replace(&mut self.mailboxes[msg.handle_ptr], mailbox);
        }
        Ok(())
    }

    /// Calculate the timestamps the message should be moved from one queue to
    /// another. Perform link simulation to simulate:
    ///   - bit errors
    ///   - dropped packets
    fn prepare_message(
        channel: &Channel,
        timestep: u64,
        mut msg: Message,
        distance: f64,
        distance_unit: DistanceUnit,
        rng: &mut StdRng,
    ) -> Option<(
        Reverse<u64>,
        Reverse<u64>,
        Reverse<u64>,
        Option<NonZeroU64>,
        Message,
    )> {
        let sz: u64 = msg
            .buf
            .len()
            .try_into()
            .expect("usize should be able to become a u64");
        let mut sample =
            |var: &DistanceProbVar| var.sample(distance, distance_unit, sz, DataUnit::Byte, rng);
        if sample(&channel.link.packet_loss) {
            info!("Packet dropped");
            return None;
        }

        let bit_error_prob =
            channel
                .link
                .bit_error
                .probability(distance, distance_unit, sz, DataUnit::Byte);
        if bit_error_prob != 0.0 {
            for byte in msg.buf.iter_mut() {
                for index in 0..u8::BITS {
                    if unsafe { channel.link.bit_error.sample_unchecked(bit_error_prob, rng) } {
                        *byte ^= 1 << index;
                    }
                }
            }
        }

        let unit = DataUnit::Byte;
        let delays = &channel.link.delays;
        let trans_deadline = timestep + delays.transmission_timesteps_f64(sz, unit).round() as u64;
        let prop_deadline = trans_deadline
            + delays
                .propagation_timesteps_f64(distance, distance_unit)
                .round() as u64;
        let proc_deadline =
            prop_deadline + delays.processing_timesteps_f64(sz, unit).round() as u64;
        let expiration = channel
            .r#type
            .ttl()
            .map(|ttl| ttl.saturating_add(proc_deadline));
        Some((
            Reverse(trans_deadline),
            Reverse(prop_deadline),
            Reverse(proc_deadline),
            expiration,
            msg,
        ))
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
    fn recv_msg<A: AsRef<str> + std::fmt::Debug>(
        socket: &mut UnixDatagram,
        buf_sz: NonZeroU64,
        pid: fuse::PID,
        timestep: u64,
        channel: ChannelHandle,
        channel_name: &A,
    ) -> Result<Vec<u8>, RouterError> {
        let mut recv_buf = vec![0; buf_sz.get() as usize];
        let nread = socket::recv(socket, &mut recv_buf, pid, channel_name)
            .map_err(RouterError::FileError)?;
        recv_buf.truncate(nread);
        event!(target: "rx", Level::INFO, timestep, channel, pid, tx = false, data = recv_buf.as_slice());
        Ok(recv_buf)
    }
}
