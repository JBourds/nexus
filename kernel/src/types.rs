//! This module translates the `config` crate's AST types into ones better
//! suited for high performance simulation and augments them with kernel
//! specific functionality.
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
};

use crate::helpers::unzip;
use crate::{errors::ConversionError, helpers::make_handles};
use config::ast::{self, ChannelType, Cmd, Link};
use tracing::instrument;

pub type ChannelHandle = usize;
pub type NodeHandle = usize;

#[derive(Debug)]
pub struct Channel {
    #[allow(unused)]
    pub link: Link,
    #[allow(unused)]
    pub r#type: ChannelType,
    pub subscribers: HashSet<NodeHandle>,
    pub publishers: HashSet<NodeHandle>,
}

impl Channel {
    /// Combine top-level channels and internal channels into one vector
    /// which can be indexed by the usize handles from the resolver.
    ///
    /// * `channels`: Top-level channels from config.
    /// * `internal_channels`: All internal channels created from nodes.
    /// * `nodes`: Set of nodes to use when constructing pub/sub lists.
    #[instrument]
    pub(super) fn from_ast(
        channels: Vec<ast::Channel>,
        internal_channels: Vec<Self>,
        nodes: &[Node],
    ) -> Result<Vec<Self>, ConversionError> {
        let mut channels = channels
            .into_iter()
            .map(|ch| Channel {
                link: ch.link,
                r#type: ch.r#type,
                subscribers: HashSet::new(),
                publishers: HashSet::new(),
            })
            .chain(internal_channels.into_iter())
            .collect::<Vec<_>>();
        for (node_handle, node) in nodes.iter().enumerate() {
            for protocol in node.protocols.iter() {
                for channel_index in protocol.subscribers.iter().copied() {
                    channels[channel_index].subscribers.insert(node_handle);
                }
                for channel_index in protocol.publishers.iter().copied() {
                    channels[channel_index].publishers.insert(node_handle);
                }
            }
        }
        Ok(channels)
    }

    pub(super) fn new_internal(handle: NodeHandle) -> Self {
        let set = HashSet::from([handle]);
        Self {
            link: Link::default(),
            r#type: ChannelType::new_internal(),
            subscribers: set.clone(),
            publishers: set,
        }
    }
}

#[derive(Clone, Debug)]
#[allow(unused)]
pub struct Node {
    pub position: ast::Position,
    pub protocols: Vec<NodeProtocol>,
}

#[derive(Clone, Debug)]
#[allow(unused)]
pub struct NodeProtocol {
    pub root: PathBuf,
    pub runner: Cmd,
    pub subscribers: HashSet<ChannelHandle>,
    pub publishers: HashSet<ChannelHandle>,
}

impl Node {
    #[instrument]
    pub(super) fn from_ast(
        node: ast::Node,
        handle: NodeHandle,
        channel_handles: &HashMap<ast::ChannelHandle, ChannelHandle>,
        node_handles: &HashMap<ast::NodeHandle, ChannelHandle>,
    ) -> Result<(Self, Vec<(ast::ChannelHandle, Channel)>), ConversionError> {
        // Internal have their own namespace, copy the hashmap
        // and overwrite any existing links with internal names.
        let new_handles = node
            .internal_names
            .clone()
            .into_iter()
            .map(|name| (name, Channel::new_internal(handle)))
            .collect::<Vec<_>>();
        let channel_handles = if !new_handles.is_empty() {
            &channel_handles
                .clone()
                .into_iter()
                .chain(
                    make_handles(node.internal_names)
                        .into_iter()
                        .map(|(name, handle)| (name, handle + channel_handles.len())),
                )
                .collect::<HashMap<ast::ChannelHandle, ChannelHandle>>()
        } else {
            channel_handles
        };

        let (_, protocols) = unzip(node.protocols);
        let protocols = protocols
            .into_iter()
            .map(|protocol| NodeProtocol::from_ast(protocol, handle, channel_handles, node_handles))
            .collect::<Result<_, ConversionError>>()?;
        Ok((
            Self {
                position: node.position,
                protocols,
            },
            new_handles,
        ))
    }
}

impl NodeProtocol {
    #[instrument]
    pub(super) fn from_ast(
        node: ast::NodeProtocol,
        handle: NodeHandle,
        channel_handles: &HashMap<ast::ChannelHandle, ChannelHandle>,
        node_handles: &HashMap<ast::NodeHandle, ChannelHandle>,
    ) -> Result<Self, ConversionError> {
        let map_channel_handles =
            |handles: HashSet<ast::ChannelHandle>| -> Result<_, ConversionError> {
                handles
                    .into_iter()
                    .map(|name| {
                        channel_handles
                            .get(&name)
                            .copied()
                            .ok_or(ConversionError::ChannelHandleConversion(name))
                    })
                    .collect::<Result<_, ConversionError>>()
            };
        let subscribers = map_channel_handles(node.subscribers)?;
        let publishers = map_channel_handles(node.publishers)?;
        Ok(Self {
            root: node.root,
            runner: node.runner,
            subscribers,
            publishers,
        })
    }
}
