//! This module translates the `config` crate's AST types into ones better
//! suited for high performance simulation and augments them with kernel
//! specific functionality.
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
};

use crate::helpers::unzip;
use crate::{errors::ConversionError, helpers::make_handles};
pub use ast::Channel;

use config::ast::{self, Cmd};
use tracing::instrument;

pub type ChannelHandle = usize;
pub type NodeHandle = usize;

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
    pub inbound: HashSet<ChannelHandle>,
    pub outbound: HashSet<ChannelHandle>,
}

impl Node {
    #[instrument]
    pub(super) fn from_ast(
        node: ast::Node,
        handle: NodeHandle,
        channel_handles: &HashMap<ast::ChannelHandle, ChannelHandle>,
        node_handles: &HashMap<ast::NodeHandle, ChannelHandle>,
    ) -> Result<(Self, Vec<ast::ChannelHandle>), ConversionError> {
        // Internal have their own namespace, copy the hashmap
        // and overwrite any existing links with internal names.
        let new_handles = node.internal_names;
        let channel_handles = if !new_handles.is_empty() {
            &channel_handles
                .clone()
                .into_iter()
                .chain(
                    make_handles(new_handles.clone())
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
        let inbound = map_channel_handles(node.inbound)?;
        let outbound = map_channel_handles(node.outbound)?;
        Ok(Self {
            root: node.root,
            runner: node.runner,
            inbound,
            outbound,
        })
    }
}
