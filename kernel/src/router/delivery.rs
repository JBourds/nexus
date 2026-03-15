use super::*;

/// Internal struct which marks a queued message as being targeted at
/// `handle_ptr`, the specific endpoint it should be delivered to in the FS.
#[derive(Clone, Debug)]
pub(crate) struct AddressedMsg {
    pub(super) handle_ptr: usize,
    pub(super) msg: QueuedMessage,
}

// Ordering for BinaryHeap: only used as tiebreaker after (Reverse<Timestep>, seq).
// The sequence counter guarantees unique ordering, so this is never actually
// reached, but the trait is required by BinaryHeap's element constraint.
impl PartialEq for AddressedMsg {
    fn eq(&self, other: &Self) -> bool {
        self.handle_ptr == other.handle_ptr && self.msg.msg_id == other.msg.msg_id
    }
}
impl Eq for AddressedMsg {}
impl PartialOrd for AddressedMsg {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for AddressedMsg {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.msg
            .msg_id
            .cmp(&other.msg.msg_id)
            .then(self.handle_ptr.cmp(&other.handle_ptr))
    }
}

/// Internal struct used for keeping track of where a queued message is from and
/// its expiration.
#[derive(Clone, Debug)]
pub(crate) struct QueuedMessage {
    pub(super) src: NodeHandle,
    pub(super) buf: Rc<[u8]>,
    pub(super) expiration: Option<NonZeroU64>,
    /// Whether the data was corrupted by bit errors during link simulation.
    pub(super) bit_errors: bool,
    /// Unique identifier for correlating TX/RX events in the trace.
    pub(super) msg_id: u64,
    /// RSSI at the receiver in dBm (computed during link simulation).
    pub(super) rssi_dbm: f64,
    /// SNR at the receiver in dB (computed during link simulation).
    pub(super) snr_db: f64,
}

impl RoutingServer {
    /// Take a message along the channel indicated by `channel_handle` from
    /// `src_node` and post it to the queue along the precomputed route.
    ///
    /// For shared channels, the raw message bytes are queued as-is; link
    /// simulation (packet loss, bit errors) runs later at delivery time so
    /// that collisions can be modelled.
    ///
    /// For exclusive channels, link simulation runs here at queue time and
    /// only surviving messages enter the queue.
    pub fn queue_message(
        &mut self,
        src_node: NodeHandle,
        channel_handle: ChannelHandle,
        msg: Vec<u8>,
        msg_id: u64,
    ) -> Result<(), RouterError> {
        let sz: u64 = msg.len().try_into().expect("usize fits u64");
        let channel = &self.channels.channels[channel_handle.0];
        let is_shared = matches!(channel.r#type.kind, ChannelKind::Shared);
        let timestep = self.timestep;
        let ts_config = self.ts_config;

        // For shared channels we share one Rc across all recipients.
        let shared_buf: Rc<[u8]> = if is_shared {
            Rc::from(msg.as_slice())
        } else {
            Rc::from([])
        };

        for route in self.routes.entries[channel_handle.0].nodes[&src_node].iter() {
            let handle_ptr = route.handle_ptr;
            let dst_node = self.channels.handles[handle_ptr].1;
            if dst_node == src_node && !channel.r#type.delivers_to_self() {
                continue;
            }

            debug!(
                "Delivering from {} to {}",
                &self.channels.node_names[src_node.0], &self.channels.node_names[dst_node.0]
            );

            let (distance, distance_unit) = Position::distance(
                &self.channels.nodes[src_node.0].position,
                &self.channels.nodes[dst_node.0].position,
            );

            // For exclusive channels, run link simulation now; drop the
            // message if it doesn't survive.
            let (buf, msg_bit_errors, rssi_dbm, snr_db): (Rc<[u8]>, bool, f64, f64) =
                if is_shared {
                    (Rc::clone(&shared_buf), false, 0.0, 0.0)
                } else {
                    match Self::send_through_channel(
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
                Self::message_timesteps(channel, sz, ts_config, timestep, distance, distance_unit);

            let msg = AddressedMsg {
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
            self.queued.push((Reverse(becomes_active_at), num, msg));
        }

        Ok(())
    }

    pub fn deliver_msg(&mut self, index: usize) -> Result<bool, RouterError> {
        let (_, _, channel_handle) = self.channels.handles[index];
        let channel = &mut self.channels.channels[channel_handle.0];
        match &channel.r#type.kind {
            ChannelKind::Shared => self.deliver_shared_msg(index),
            ChannelKind::Exclusive { .. } => self.deliver_exclusive_msg(index),
        }
    }

    fn deliver_shared_msg(&mut self, index: usize) -> Result<bool, RouterError> {
        let (pid, node_handle, channel_handle) = self.channels.handles[index];
        let channel = &self.channels.channels[channel_handle.0];
        let channel_name = &self.channels.channel_names[channel_handle.0];
        let node_name = &self.channels.node_names[node_handle.0];
        let timestep = self.timestep;

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
                    &self.channels.nodes[msg.src.0].position,
                    &self.channels.nodes[node_handle.0].position,
                );

                if let Some((buf, bit_errors, rssi_dbm, snr_db)) = Self::send_through_channel(
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
                    self.tx
                        .send(fuse::KernelMessage::Shared(msg))
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
                        &self.channels.nodes[msg.src.0].position,
                        &self.channels.nodes[node_handle.0].position,
                    );
                    Self::send_through_channel(
                        channel,
                        Cow::from(msg.buf.as_ref()),
                        distance,
                        unit,
                        &mut self.rng,
                    )
                });

                // Combine signals; track whether any contributing message had
                // bit errors, and keep the last RSSI/SNR.
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
                // Collisions always corrupt the signal.
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
                self.tx
                    .send(fuse::KernelMessage::Shared(msg))
                    .map(|_| true)
                    .map_err(RouterError::FuseSendError)
            }
        }
    }

    fn deliver_exclusive_msg(&mut self, index: usize) -> Result<bool, RouterError> {
        let (pid, node_handle, channel_handle) = self.channels.handles[index];
        let mailbox = &mut self.mailboxes[index];
        if let Some(msg) = mailbox.pop_front() {
            // Store signal quality from queue-time link simulation
            self.signal_info[index].rssi_dbm = msg.rssi_dbm;
            self.signal_info[index].snr_db = msg.snr_db;
            if msg.expiration.is_some_and(|e| e.get() < self.timestep) {
                warn!(
                    "Message dropped due to timeout (Now: {}, Expiration: {})",
                    self.timestep,
                    msg.expiration.unwrap().get()
                );
                event!(
                    target: "drop", Level::WARN,
                    timestep = self.timestep,
                    channel = channel_handle.0,
                    node = node_handle.0,
                    msg_id = msg.msg_id,
                    reason = "ttl_expired"
                );
                return Ok(false);
            }
            let node_name = &self.channels.node_names[node_handle.0];
            let channel_name = &self.channels.channel_names[channel_handle.0];
            if tracing::enabled!(Level::INFO) {
                info!(
                    "{:<30} [RX]: {} <Now: {}, Expiration: {:?}>",
                    format!("{}.{}.{}", node_name, pid, channel_name),
                    format_u8_buf(&msg.buf),
                    self.timestep,
                    msg.expiration,
                );
            }
            let bit_errors = msg.bit_errors;
            let mid = msg.msg_id;
            let msg = fuse::Message {
                id: (pid, self.channels.node_names[node_handle.0].to_string()),
                data: msg.buf.to_vec(),
            };
            event!(
                target: "rx", Level::INFO, timestep = self.timestep, channel = channel_handle.0,
                node = node_handle.0, tx = false, bit_errors, msg_id = mid, data = msg.data.as_slice()
            );

            self.tx
                .send(fuse::KernelMessage::Exclusive(msg))
                .map(|_| true)
                .map_err(RouterError::FuseSendError)
        } else {
            Ok(false)
        }
    }
}
