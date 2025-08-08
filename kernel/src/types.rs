//! This module translates the `config` crate's AST types into ones better
//! suited for high performance simulation and augments them with kernel
//! specific functionality.
use std::hash::Hash;
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
};

use crate::errors::ConversionError;
use crate::helpers::unzip;

use config::ast::{self, Cmd};

pub type LinkHandle = usize;
pub type ProtocolHandle = usize;
pub type NodeHandle = usize;

#[derive(Clone, Debug)]
pub struct Node {
    pub position: ast::Position,
    pub protocols: Vec<NodeProtocol>,
}

#[derive(Clone, Debug)]
pub struct NodeProtocol {
    pub root: PathBuf,
    pub runner: Cmd,
    pub accepts: HashSet<LinkHandle>,
    pub direct: HashMap<NodeHandle, HashSet<LinkHandle>>,
    pub indirect: HashSet<LinkHandle>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Link {
    pub next: Option<LinkHandle>,
    pub intermediaries: u32,
    pub signal: ast::Signal,
    pub transmission: ast::Rate,
    pub bit_error: ast::DistanceVar,
    pub packet_loss: ast::DistanceVar,
    pub queue_delay: ast::DistanceVar,
    pub processing_delay: ast::DistanceVar,
    pub connection_delay: ast::DistanceVar,
    pub propagation_delay: ast::DistanceVar,
}

impl Node {
    pub(super) fn from_ast(
        node: ast::Node,
        link_handles: &HashMap<ast::LinkHandle, LinkHandle>,
        node_handles: &HashMap<ast::NodeHandle, LinkHandle>,
    ) -> Result<Self, ConversionError> {
        let (_, protocols) = unzip(node.protocols);
        let protocols = protocols
            .into_iter()
            .map(|protocol| NodeProtocol::from_ast(protocol, link_handles, node_handles))
            .collect::<Result<_, ConversionError>>()?;
        Ok(Self {
            position: node.position,
            protocols,
        })
    }
}

impl NodeProtocol {
    pub(super) fn from_ast(
        node: ast::NodeProtocol,
        link_handles: &HashMap<ast::LinkHandle, LinkHandle>,
        node_handles: &HashMap<ast::NodeHandle, LinkHandle>,
    ) -> Result<Self, ConversionError> {
        let map_link_handles = |handles: HashSet<ast::LinkHandle>| -> Result<_, ConversionError> {
            handles
                .into_iter()
                .map(|name| {
                    link_handles
                        .get(&name)
                        .map(|handle| *handle)
                        .ok_or(ConversionError::LinkHandleConversion(name))
                })
                .collect::<Result<_, ConversionError>>()
        };
        let accepts = map_link_handles(node.accepts)?;
        let indirect = map_link_handles(node.indirect)?;
        let direct = node
            .direct
            .into_iter()
            .map(|(key, handles)| {
                let links = map_link_handles(handles)?;
                let key = node_handles
                    .get(&key)
                    .map(|handle| *handle)
                    .ok_or(ConversionError::NodeHandleConversion(key))?;
                Ok((key, links))
            })
            .collect::<Result<_, ConversionError>>()?;
        Ok(Self {
            root: node.root,
            runner: node.runner,
            accepts,
            direct,
            indirect,
        })
    }
}

impl Link {
    pub(super) fn from_ast(
        node: ast::Link,
        handles: &HashMap<ast::LinkHandle, LinkHandle>,
    ) -> Result<Self, ConversionError> {
        let next = if let Some(name) = node.next {
            handles
                .get(&name)
                .map(|v| Option::Some(*v))
                .ok_or(ConversionError::LinkHandleConversion(name))?
        } else {
            None
        };
        Ok(Self {
            next,
            intermediaries: node.intermediaries,
            signal: node.signal,
            transmission: node.transmission,
            bit_error: node.bit_error,
            packet_loss: node.packet_loss,
            queue_delay: node.queue_delay,
            processing_delay: node.processing_delay,
            connection_delay: node.connection_delay,
            propagation_delay: node.propagation_delay,
        })
    }
}
