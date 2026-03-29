//! TAP networking integration for the simulation kernel.
//!
//! When a simulation contains a `Network` channel, this module creates TAP
//! devices and network namespaces for each node that has a `NetworkConfig`,
//! and starts a [`TapRouter`] to shuttle Ethernet frames between them with
//! link simulation applied.

use std::borrow::Cow;
use std::collections::HashMap;

use anyhow::{Context, Result};
use config::ast::{ChannelKind, NetworkConfig, Position};
use netdev::{Namespace, TapDevice, TapRouter, TapRouterHandle};
use rand::rngs::StdRng;
use tracing::info;

use crate::resolver::ResolvedChannels;
use crate::types::Channel;

/// The TAP interface name prefix used inside each namespace.
const TAP_IF_NAME: &str = "eth0";

/// Holds all networking state for a simulation that uses `Network` channels.
pub(crate) struct NetworkingState {
    /// The TapRouter handle (owns the router thread).
    pub handle: TapRouterHandle,
    /// Map from TAP device index to resolved node index (into `channels.nodes`).
    tap_to_node: Vec<usize>,
    /// Map from resolved node index to TAP device index.
    node_to_tap: HashMap<usize, usize>,
    /// Clone of the network channel (link config for simulation).
    channel: Channel,
    /// Clone of each node's position, indexed by resolved node index.
    positions: Vec<Position>,
    /// RNG for link simulation (seeded from the main kernel RNG).
    rng: StdRng,
}

impl NetworkingState {
    /// Set up TAP devices, namespaces, and a TapRouter for all nodes that
    /// participate in a `Network` channel.
    ///
    /// `node_network_configs` is a list of `(node_name, resolved_node_index, config)`.
    ///
    /// Returns `None` if no network channels exist in the simulation.
    /// On success, returns the networking state and a list of namespace names
    /// for the runner to clean up.
    pub fn setup(
        channels: &ResolvedChannels,
        node_network_configs: &[(String, usize, NetworkConfig)],
        rng: StdRng,
    ) -> Result<Option<(Self, Vec<String>)>> {
        // Find the first network channel (we support one for now).
        let network_channel_idx = channels
            .channels
            .iter()
            .position(|ch| matches!(ch.r#type.kind, ChannelKind::Network));

        let Some(network_channel_idx) = network_channel_idx else {
            return Ok(None);
        };

        if node_network_configs.is_empty() {
            return Ok(None);
        }

        info!(
            "Setting up TAP networking for {} nodes",
            node_network_configs.len()
        );

        let mut devices = Vec::new();
        let mut tap_to_node = Vec::new();
        let mut node_to_tap = HashMap::new();
        let mut namespace_names = Vec::new();

        for (node_name, node_idx, net_cfg) in node_network_configs {
            let ns_name = format!("nexus_{node_name}");
            let tap_name = format!("nxs_{}", &node_name[..node_name.len().min(12)]);

            // Create the TAP device (in the host namespace initially).
            // The fd opened here remains valid even after the interface is
            // moved into a network namespace — fds follow the process, not
            // the namespace.
            let tap = TapDevice::create(&tap_name)
                .with_context(|| format!("Failed to create TAP for node {node_name}"))?;

            // Create the network namespace.
            let ns = Namespace::create(&ns_name)
                .with_context(|| format!("Failed to create namespace for node {node_name}"))?;

            // Move TAP into the namespace.
            ns.move_interface(&tap_name)
                .with_context(|| format!("Failed to move {tap_name} into {ns_name}"))?;

            // Rename the interface to "eth0" inside the namespace for clarity.
            let _ = std::process::Command::new("ip")
                .args([
                    "netns", "exec", &ns_name, "ip", "link", "set", &tap_name,
                    "name", TAP_IF_NAME,
                ])
                .output();

            ns.configure_address(TAP_IF_NAME, &net_cfg.address)
                .with_context(|| {
                    format!(
                        "Failed to configure address {} in {ns_name}",
                        net_cfg.address
                    )
                })?;
            ns.set_interface_up(TAP_IF_NAME)?;
            ns.set_loopback_up()?;

            // If a gateway is configured, add a default route.
            if let Some(gw) = &net_cfg.gateway {
                let _ = std::process::Command::new("ip")
                    .args([
                        "netns", "exec", &ns_name, "ip", "route", "add", "default",
                        "via", gw,
                    ])
                    .output();
            }

            // Namespace cleanup is handled by the runner's CgroupController.
            std::mem::forget(ns);

            let tap_idx = devices.len();
            devices.push((node_name.clone(), tap));
            tap_to_node.push(*node_idx);
            node_to_tap.insert(*node_idx, tap_idx);
            namespace_names.push(ns_name);
        }

        // Clone channel and position data for link simulation.
        let channel = channels.channels[network_channel_idx].clone();
        let positions: Vec<Position> = channels.nodes.iter().map(|n| n.position.clone()).collect();

        let router = TapRouter::new(devices);
        let handle = router.spawn().context("Failed to spawn TapRouter")?;

        Ok(Some((
            NetworkingState {
                handle,
                tap_to_node,
                node_to_tap,
                channel,
                positions,
                rng,
            },
            namespace_names,
        )))
    }

    /// Process incoming TAP frames: apply link simulation and deliver to
    /// destination nodes.  Called from the kernel main loop.
    pub fn poll_and_route(&mut self) {
        let frames = self.handle.drain_frames();
        if frames.is_empty() {
            return;
        }

        for frame in frames {
            let src_node_idx = self.tap_to_node[frame.src];

            // Deliver to all other TAP-enabled nodes (broadcast at Layer 2).
            for (&dst_node_idx, &dst_tap_idx) in &self.node_to_tap {
                if dst_node_idx == src_node_idx {
                    continue;
                }

                // Apply link simulation using the same model as FUSE channels.
                let (distance, distance_unit) = Position::distance(
                    &self.positions[src_node_idx],
                    &self.positions[dst_node_idx],
                );

                let result = crate::router::RoutingServer::send_through_channel(
                    &self.channel,
                    Cow::Borrowed(&frame.data),
                    distance,
                    distance_unit,
                    &mut self.rng,
                );

                if let Some((data, _bit_errors, _rssi, _snr)) = result {
                    if let Err(e) = self.handle.deliver(dst_tap_idx, data.into_owned()) {
                        tracing::warn!("Failed to deliver TAP frame: {e}");
                    }
                }
            }
        }
    }

    /// Shut down the TAP router thread.
    pub fn shutdown(self) -> Result<()> {
        self.handle.shutdown()
    }
}
