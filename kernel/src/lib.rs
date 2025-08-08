pub mod errors;
mod helpers;
mod types;
use helpers::{make_handles, unzip};

use std::{collections::HashMap, os::unix::net::UnixDatagram};

use config::ast::{self, Params};
use fuse::LinkId;
use runner::RunMode;
use types::*;

use crate::errors::{ConversionError, KernelError};

#[derive(Debug)]
pub struct Kernel {
    params: Params,
    links: Vec<Link>,
    nodes: Vec<Node>,
    files: HashMap<LinkId, UnixDatagram>,
}

impl Kernel {
    pub fn new(
        sim: ast::Simulation,
        files: HashMap<LinkId, UnixDatagram>,
    ) -> Result<Self, KernelError> {
        let (node_names, nodes) = unzip(sim.nodes);
        let node_handles = make_handles(node_names);
        let (link_names, links) = unzip(sim.links);
        let link_handles = make_handles(link_names);
        let links = links
            .into_iter()
            .map(|link| Link::from_ast(link, &link_handles))
            .collect::<Result<_, ConversionError>>()
            .map_err(KernelError::KernelInit)?;
        let nodes = nodes
            .into_iter()
            .map(|node| Node::from_ast(node, &link_handles, &node_handles))
            .collect::<Result<_, ConversionError>>()
            .map_err(KernelError::KernelInit)?;
        Ok(Self {
            params: sim.params,
            links,
            nodes,
            files,
        })
    }

    pub fn run(mode: RunMode) -> ! {
        todo!()
    }
}
