use crate::{
    LinkId,
    errors::RouterError,
    types::{Link, Node},
};
use fuse::{PID, errors::SocketError, socket};
use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    io,
    os::unix::net::UnixDatagram,
    rc::Rc,
};
use tracing::{Level, debug, error, event, instrument};

use crate::types::LinkHandle;

#[derive(Debug)]
#[allow(dead_code)]
pub(crate) struct Message {
    sender: PID,
    link: LinkHandle,
    buf: Vec<u8>,
}

/// Route information computed based on link parameters and number of
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
    links: Vec<Link>,
    link_names: Vec<String>,
    endpoints: Vec<UnixDatagram>,
    handles: Vec<LinkId>,
    routing_table: HashMap<LinkId, Vec<Route>>,
    mailboxes: Vec<VecDeque<Rc<Vec<u8>>>>,
    timestep: u64,
}

impl Router {
    /// Build the routing table during initialization.
    #[instrument]
    pub fn new(
        nodes: Vec<Node>,
        node_names: Vec<String>,
        links: Vec<Link>,
        link_names: Vec<String>,
        handles: Vec<LinkId>,
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
            links,
            link_names,
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
        let (pid, node_handle, link_handle) = self.handles[index];
        let link_name = &self.link_names[link_handle];
        let endpoint = &mut self.endpoints[index];
        loop {
            match Self::recv_msg(endpoint, pid, self.timestep, link_handle, link_name) {
                Ok(recv_buf) => {
                    let msg = Rc::new(recv_buf);
                    let Some(recipients) = self.routing_table.get(&(pid, node_handle, link_handle))
                    else {
                        error!(
                            "Couldn't find key {:?} in {:#?}",
                            (pid, link_handle),
                            self.routing_table
                        );
                        return Ok(());
                    };
                    debug!(
                        "[TX] {node_handle}.{pid}.{link_name}: {}",
                        String::from_utf8_lossy(&msg)
                    );
                    // TODO: Use other route information to determine delays
                    // and mutations/drops
                    for Route { index_pointer, .. } in recipients {
                        self.mailboxes[*index_pointer].push_back(Rc::clone(&msg));
                    }
                }
                Err(RouterError::FileError(SocketError::NothingToRead)) => {
                    break;
                }
                Err(RouterError::FileError(SocketError::SocketReadError { ioerr, .. }))
                    if ioerr.kind() == io::ErrorKind::WouldBlock =>
                {
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
            let (pid, node_handle, link_handle) = self.handles[index];
            let link_name = &self.link_names[link_handle];
            let timestep = self.timestep;
            while let Some(msg) = mailbox.pop_front() {
                debug!("{pid}.{link_name} [RX]: {}", String::from_utf8_lossy(&msg));
                Self::send_msg(endpoint, &msg, pid, timestep, link_handle, link_name).map_err(
                    |_| RouterError::SendError {
                        sender: pid,
                        node_name: self.node_names[node_handle].clone(),
                        link_name: self.link_names[link_handle].clone(),
                        timestep,
                    },
                )?;
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
        link: LinkHandle,
        link_name: &A,
    ) -> Result<usize, RouterError> {
        let len = data.len();
        debug!("Sending {len} byte message");
        let msg_len = len.to_ne_bytes();
        Self::send(socket, &msg_len, pid, link_name)?;

        match Self::send(socket, data, pid, link_name) {
            Ok(n_sent) => {
                event!(target: "tx", Level::INFO, timestep, link, pid, tx = true, data);
                Ok(n_sent)
            }
            err => err,
        }
    }

    fn send<A: AsRef<str> + std::fmt::Debug>(
        socket: &mut UnixDatagram,
        data: &[u8],
        pid: fuse::PID,
        link_name: A,
    ) -> Result<usize, RouterError> {
        socket::send(socket, data, pid, link_name).map_err(RouterError::FileError)
    }

    #[instrument(skip(socket))]
    fn recv_msg<A: AsRef<str> + std::fmt::Debug>(
        socket: &mut UnixDatagram,
        pid: fuse::PID,
        timestep: u64,
        link: LinkHandle,
        link_name: &A,
    ) -> Result<Vec<u8>, RouterError> {
        let mut msg_len = [0u8; core::mem::size_of::<usize>()];
        Self::recv(socket, &mut msg_len, pid, link_name)?;
        let required_capacity = usize::from_ne_bytes(msg_len);
        debug!("Receiving {required_capacity} byte message");
        let mut recv_buf = vec![0; required_capacity];
        let data = recv_buf.as_mut_slice();
        Self::recv(socket, data, pid, link_name)?;
        event!(target: "rx", Level::INFO, timestep, link, pid, tx = false, data);
        Ok(recv_buf)
    }

    fn recv<A: AsRef<str> + std::fmt::Debug>(
        socket: &mut UnixDatagram,
        data: &mut [u8],
        pid: fuse::PID,
        link_name: &A,
    ) -> Result<usize, RouterError> {
        socket::recv(socket, data, pid, link_name).map_err(RouterError::FileError)
    }
}
