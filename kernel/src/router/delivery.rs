use super::*;

/// Internal struct which marks a queued message as being targeted at
/// `handle_ptr`, the specific endpoint it should be delivered to in the FS.
#[derive(Clone, Debug, Eq, PartialOrd, Ord, PartialEq)]
pub(crate) struct AddressedMsg {
    pub(super) handle_ptr: usize,
    pub(super) msg: QueuedMessage,
}

/// Internal struct used for keeping track of where a queued messag is from and
/// its expiration.
#[derive(Clone, Debug, Eq, PartialOrd, Ord, PartialEq)]
pub(crate) struct QueuedMessage {
    pub(super) src: NodeHandle,
    pub(super) buf: Rc<[u8]>,
    pub(super) expiration: Option<NonZeroU64>,
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
            let buf: Rc<[u8]> = if is_shared {
                Rc::clone(&shared_buf)
            } else {
                match Self::send_through_channel(
                    channel,
                    Cow::from(&msg),
                    distance,
                    distance_unit,
                    &mut self.rng,
                ) {
                    Some(b) => b.into(),
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

                if let Some(buf) = Self::send_through_channel(
                    channel,
                    Cow::from(msg.buf.as_ref()),
                    distance,
                    unit,
                    &mut self.rng,
                ) {
                    info!(
                        "{:<30} [RX]: {} <Now: {}, Expiration: {:?}>",
                        format!("{}.{}.{}", node_name, pid, channel_name),
                        format_u8_buf(&buf),
                        timestep,
                        msg.expiration,
                    );
                    let msg = fuse::Message {
                        id: (pid, node_name.clone()),
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

                // Combine signals
                let buf = filtered.fold(Vec::with_capacity(max_size.get()), |mut v, msg| {
                    let small = v.len().min(msg.len());
                    for i in 0..small {
                        v[i] |= msg[i];
                    }
                    v.extend_from_slice(&msg[small..]);
                    v
                });
                event!(
                    target: "rx", Level::INFO, timestep, channel = channel_handle.0,
                    node = node_handle.0, tx = false, data = buf.as_slice()
                );
                let msg = fuse::Message {
                    id: (pid, node_name.clone()),
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
                    reason = "ttl_expired"
                );
                return Ok(false);
            }
            let node_name = &self.channels.node_names[node_handle.0];
            let channel_name = &self.channels.channel_names[channel_handle.0];
            info!(
                "{:<30} [RX]: {} <Now: {}, Expiration: {:?}>",
                format!("{}.{}.{}", node_name, pid, channel_name),
                format_u8_buf(&msg.buf),
                self.timestep,
                msg.expiration,
            );
            let msg = fuse::Message {
                id: (pid, self.channels.node_names[node_handle.0].clone()),
                data: msg.buf.to_vec(),
            };
            event!(
                target: "rx", Level::INFO, timestep = self.timestep, channel = channel_handle.0,
                node = node_handle.0, tx = false, data = msg.data.as_slice()
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
