use std::collections::HashMap;

use config::ast;
use fuse::PID;

use crate::{
    errors::{ConversionError, KernelError},
    helpers::{make_handles, unzip},
    types::{self, Channel, ChannelHandle, Node, NodeHandle},
};

/// Struct which resolves all strings within AST structs to integers.
#[derive(Debug)]
pub(crate) struct ResolvedChannels {
    pub(crate) nodes: Vec<types::Node>,
    pub(crate) node_names: Vec<String>,
    pub(crate) channels: Vec<types::Channel>,
    pub(crate) channel_names: Vec<String>,
    pub(crate) handles: Vec<(PID, NodeHandle, ChannelHandle)>,
}

impl ResolvedChannels {
    /// This function resolves channel names to indices used to reference them.
    /// Internal channels can shadow global channels, and will be automatically
    /// treated as though they have an ideal link connecting them to all
    /// protocols on that node.
    pub(super) fn try_resolve(
        channels: HashMap<String, config::ast::Channel>,
        node_names: Vec<String>,
        nodes: Vec<config::ast::Node>,
        node_handles: &HashMap<String, usize>,
        file_handles: Vec<(PID, ast::NodeHandle, ast::ProtocolHandle)>,
    ) -> Result<Self, KernelError> {
        let (mut channel_names, channels) = unzip(channels);
        let channel_handles = make_handles(channel_names.clone());

        // Validate nodes so they all use indices rather than strings
        let mut new_nodes = vec![];
        // Internal channels created with ideal links
        let mut internal_channels = vec![];
        // Mapping from a node's string name and the channel name to index
        let mut internal_node_channel_handles = HashMap::new();

        for (handle, (node_name, node)) in node_names.iter().zip(nodes.into_iter()).enumerate() {
            // Validate each node and convert to use indices rather than strings
            // Get all (name, object) pairs for internal channels
            let (new_node, new_internals) =
                Node::from_ast(node, handle, &channel_handles, node_handles)
                    .map_err(KernelError::KernelInit)?;

            // Extract vec of tuples to tuple of vecs
            let (new_internal_names, new_internal_channels) = unzip(new_internals);

            // Save new node
            new_nodes.push(new_node);

            // Extend vectors with the new channel objects and their matching
            // names (FIXME: Use an RC here to avoid a bunch of string clones)
            channel_names.extend(new_internal_names.clone());
            internal_channels.extend(new_internal_channels);

            // Update record mapping (node name, channel name) pairs to vector
            // index where its info can be found
            for (handle, internal_name) in
                (channel_names.len() - 1..).zip(new_internal_names.into_iter())
            {
                internal_node_channel_handles.insert((node_name.clone(), internal_name), handle);
            }
        }

        // Resolve handles to use integer handles
        let handles = file_handles
            .into_iter()
            .map(|(pid, node, channel)| {
                let node_handle = *node_handles.get(&node).unwrap();
                internal_node_channel_handles
                    .get(&(node.clone(), channel.clone()))
                    .or(channel_handles.get(&channel))
                    .ok_or(KernelError::KernelInit(
                        ConversionError::ChannelHandleConversion(channel),
                    ))
                    .map(|channel_handle| (pid, node_handle, *channel_handle))
            })
            .collect::<Result<Vec<_>, KernelError>>()?;

        Channel::from_ast(channels, internal_channels, &new_nodes)
            .map_err(KernelError::KernelInit)
            .map(|channels| Self {
                nodes: new_nodes,
                node_names,
                channels,
                channel_names,
                handles,
            })
    }

    /// Produce a map for the FUSE filesystem to use which maps the process ID
    /// and channel handle to the handle index which should be used.
    ///
    /// This is used in a "patch" step during startup. At the beginning, FUSE
    /// modules only identify the process ID of a running node protocol and the
    /// string name for the channel it reads/writes to. The kernel module
    /// resolves all string names to flattened integer handles to resolve
    /// ambiguity with internal node channels (and to speed up routine lookup
    /// operations by removing the need for a hash map). This function creates
    /// the mapping between the key used by the FUSE module and the handle
    /// used by the kernel. This gets sent over as a message to the FUSE fs
    /// during startup and is used to resolve all indices within the kernel.
    pub fn make_fuse_mapping(&self) -> HashMap<fuse::ChannelId, usize> {
        self.handles
            .iter()
            .enumerate()
            .map(|(index, (pid, _, channel))| ((*pid, self.channel_names[*channel].clone()), index))
            .collect()
    }
}
