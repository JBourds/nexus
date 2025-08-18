use crate::{
    ChannelId,
    errors::RouterError,
    types::{Channel, Node},
};
use fuse::{PID, socket};
use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    os::unix::net::UnixDatagram,
    rc::Rc,
};
use tracing::{Level, debug, error, event, instrument};

use crate::types::ChannelHandle;

#[derive(Debug)]
#[allow(dead_code)]
pub(crate) struct Message {
    sender: PID,
    channel: ChannelHandle,
    buf: Vec<u8>,
}

/// Route information computed based on channel parameters and number of
/// intermediaries representative of the entire route.
#[derive(Debug, Default)]
#[allow(dead_code)]
struct Route {
    index_pointer: usize,
    delay_avg: u64,
    delay_std: u64,
    packet_loss_prob: f64,
    bit_error_rate: f64,
}

#[derive(Debug)]
#[allow(dead_code)]
pub(crate) struct Router {
    queued: BTreeMap<u64, Message>,
    nodes: Vec<Node>,
    node_names: Vec<String>,
    channels: Vec<Channel>,
    channel_names: Vec<String>,
    endpoints: Vec<UnixDatagram>,
    handles: Vec<ChannelId>,
    routing_table: HashMap<ChannelId, Vec<Route>>,
    mailboxes: Vec<VecDeque<Rc<Vec<u8>>>>,
    timestep: u64,
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
        // TODO: Build an actual routing table here instead of delivering to
        // everything else
        let handles_count = handles.len();
        let routing_table = (0..handles_count)
            .map(|i| {
                (
                    handles[i],
                    (0..handles_count)
                        .filter(|j| i != *j)
                        .map(|j| Route {
                            index_pointer: j,
                            ..Default::default()
                        })
                        .collect(),
                )
            })
            .collect();

        Self {
            nodes,
            node_names,
            channels,
            channel_names,
            handles,
            queued: BTreeMap::new(),
            routing_table,
            mailboxes: vec![VecDeque::new(); handles_count],
            endpoints,
            timestep: 0,
        }
    }

    #[instrument(skip_all)]
    pub fn inbound(&mut self, index: usize) -> Result<(), RouterError> {
        let (pid, node_handle, channel_handle) = self.handles[index];
        let channel_name = &self.channel_names[channel_handle];
        let endpoint = &mut self.endpoints[index];
        loop {
            match Self::recv_msg(endpoint, pid, self.timestep, channel_handle, channel_name) {
                Ok(recv_buf) => {
                    let msg = Rc::new(recv_buf);
                    let Some(recipients) =
                        self.routing_table.get(&(pid, node_handle, channel_handle))
                    else {
                        error!(
                            "Couldn't find key {:?} in {:#?}",
                            (pid, channel_handle),
                            self.routing_table
                        );
                        return Ok(());
                    };
                    debug!(
                        "[TX] {node_handle}.{pid}.{channel_name}: {}",
                        String::from_utf8_lossy(&msg)
                    );
                    // TODO: Use other route information to determine delays
                    // and mutations/drops
                    for Route { index_pointer, .. } in recipients {
                        self.mailboxes[*index_pointer].push_back(Rc::clone(&msg));
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
                    String::from_utf8_lossy(&msg)
                );
                match Self::send_msg(endpoint, &msg, pid, timestep, channel_handle, channel_name) {
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
        pid: fuse::PID,
        timestep: u64,
        channel: ChannelHandle,
        channel_name: &A,
    ) -> Result<Vec<u8>, RouterError> {
        // TODO: Replace the hardcoded vector
        let mut recv_buf = vec![0; 4096];
        let nread = socket::recv(socket, &mut recv_buf, pid, channel_name)
            .map_err(RouterError::FileError)?;
        recv_buf.truncate(nread);
        event!(target: "rx", Level::INFO, timestep, channel, pid, tx = false, data = recv_buf.as_slice());
        Ok(recv_buf)
    }
}
