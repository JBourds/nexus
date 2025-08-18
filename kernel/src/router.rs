use crate::{
    ChannelId,
    errors::RouterError,
    types::{Channel, Node},
};
use fuse::{PID, socket};
use std::{
    collections::{BTreeMap, VecDeque},
    num::NonZeroU64,
    os::unix::net::UnixDatagram,
};
use tracing::{Level, debug, event, instrument};

use crate::types::ChannelHandle;

pub type Timestep = u64;

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub(crate) struct Message {
    sender: PID,
    channel: ChannelHandle,
    buf: Vec<u8>,
}

#[derive(Debug)]
#[allow(dead_code)]
pub(crate) struct Router {
    /// map for messages queued in each timestep
    queued: BTreeMap<Timestep, Vec<Message>>,
    /// nodes in the simulation
    nodes: Vec<Node>,
    /// names for nodes (only used in debugging/printing)
    node_names: Vec<String>,
    /// channels in the simulation
    channels: Vec<Channel>,
    /// names for channels (only used in debugging/printing)
    channel_names: Vec<String>,
    /// per-channel vector with the indices of every handle associated with it
    routes: Vec<Vec<usize>>,
    /// actual unix domain sockets being read/written from
    endpoints: Vec<UnixDatagram>,
    /// all the unique keys for each channel file
    handles: Vec<ChannelId>,
    /// per-handle file mailbox with buffered messages
    mailboxes: Vec<VecDeque<Message>>,
    /// current simulation timestep
    timestep: Timestep,
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
    ) -> Self {
        let handles_count = handles.len();
        let routes = channels
            .iter()
            .enumerate()
            .map(|(ch_index, ch)| {
                ch.outbound
                    .iter()
                    .chain(ch.inbound.iter())
                    .copied()
                    .flat_map(|node_handle| {
                        handles.iter().enumerate().filter_map(
                            move |(index, (_, dst_node, dst_ch))| {
                                if *dst_node == node_handle && *dst_ch == ch_index {
                                    Some(index)
                                } else {
                                    None
                                }
                            },
                        )
                    })
                    .collect::<Vec<usize>>()
            })
            .collect::<Vec<_>>();

        Self {
            nodes,
            node_names,
            channels,
            channel_names,
            routes,
            handles,
            queued: BTreeMap::new(),
            mailboxes: vec![VecDeque::new(); handles_count],
            endpoints,
            timestep: 0,
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
                    let msg = Message {
                        sender: pid,
                        channel: channel_handle,
                        buf: recv_buf,
                    };
                    debug!(
                        "[TX] {src_node}.{pid}.{channel_name}: {}",
                        String::from_utf8_lossy(&msg.buf)
                    );

                    for handle in self.routes[channel_handle].iter().copied() {
                        let dst_node = self.handles[handle].1;
                        if dst_node != src_node || channel.r#type.delivers_to_self() {
                            self.mailboxes[handle].push_back(msg.clone());
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

    pub fn outbound(&mut self) -> Result<(), RouterError> {
        for (index, mailbox) in self.mailboxes.iter_mut().enumerate() {
            let endpoint = &mut self.endpoints[index];
            let (pid, node_handle, channel_handle) = self.handles[index];
            let channel_name = &self.channel_names[channel_handle];
            let timestep = self.timestep;
            while let Some(msg) = mailbox.pop_front() {
                debug!(
                    "{pid}.{channel_name} [RX]: {}",
                    String::from_utf8_lossy(&msg.buf)
                );
                match Self::send_msg(
                    endpoint,
                    &msg.buf,
                    pid,
                    timestep,
                    channel_handle,
                    channel_name,
                ) {
                    Ok(_) => {}
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
        }
        Ok(())
    }

    pub fn step(&mut self) -> Result<(), RouterError> {
        self.timestep += 1;
        Ok(())
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
