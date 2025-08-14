//! This module translates the `config` crate's AST types into ones better
//! suited for high performance simulation and augments them with kernel
//! specific functionality.
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
};

use crate::helpers::unzip;
use crate::{errors::ConversionError, helpers::make_handles};

use config::ast::{self, Cmd};
use tracing::instrument;

pub type LinkHandle = usize;
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
    pub accepts: HashSet<LinkHandle>,
    pub direct: HashMap<NodeHandle, HashSet<LinkHandle>>,
    pub indirect: HashSet<LinkHandle>,
}

#[derive(Clone, Debug, Default, PartialEq)]
#[allow(unused)]
pub struct Link {
    pub next: Option<LinkHandle>,
    pub intermediaries: u32,
    pub signal: ast::Signal,
    pub transmission: ast::Rate,
    pub bit_error: ast::ProbabilityVar,
    pub packet_loss: ast::ProbabilityVar,
    pub delays: ast::Delays,
}

impl Node {
    #[instrument]
    pub(super) fn from_ast(
        node: ast::Node,
        handle: NodeHandle,
        link_handles: &HashMap<ast::LinkHandle, LinkHandle>,
        node_handles: &HashMap<ast::NodeHandle, LinkHandle>,
    ) -> Result<(Self, Vec<ast::LinkHandle>), ConversionError> {
        // Internal have their own namespace, copy the hashmap
        // and overwrite any existing links with internal names.
        let new_handles = node.internal_names;
        let link_handles = if !new_handles.is_empty() {
            &link_handles
                .clone()
                .into_iter()
                .chain(
                    make_handles(new_handles.clone())
                        .into_iter()
                        .map(|(name, handle)| (name, handle + link_handles.len())),
                )
                .collect::<HashMap<ast::LinkHandle, LinkHandle>>()
        } else {
            link_handles
        };

        let (_, protocols) = unzip(node.protocols);
        let protocols = protocols
            .into_iter()
            .map(|protocol| NodeProtocol::from_ast(protocol, handle, link_handles, node_handles))
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
        link_handles: &HashMap<ast::LinkHandle, LinkHandle>,
        node_handles: &HashMap<ast::NodeHandle, LinkHandle>,
    ) -> Result<Self, ConversionError> {
        let map_link_handles = |handles: HashSet<ast::LinkHandle>| -> Result<_, ConversionError> {
            handles
                .into_iter()
                .map(|name| {
                    link_handles
                        .get(&name)
                        .copied()
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
                let key = if matches!(key.as_str(), ast::Node::SELF) {
                    handle
                } else {
                    node_handles
                        .get(&key)
                        .copied()
                        .ok_or(ConversionError::NodeHandleConversion(key))?
                };
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
    #[instrument]
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
            delays: node.delays,
        })
    }
}
