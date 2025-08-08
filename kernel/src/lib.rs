pub mod errors;
mod helpers;
mod types;
use helpers::{make_handles, unzip};

use std::{collections::HashMap, os::unix::net::UnixDatagram};

use config::ast::{self, Params};
use runner::RunMode;
use types::*;

use crate::errors::{ConversionError, KernelError};

pub type LinkId = (fuse::PID, LinkHandle);

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
        files: HashMap<fuse::LinkId, UnixDatagram>,
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
        let files = files
            .into_iter()
            .map(|((pid, link_name), file)| {
                link_handles
                    .get(&link_name)
                    .ok_or(KernelError::KernelInit(
                        ConversionError::LinkHandleConversion(link_name),
                    ))
                    .map(|handle| ((pid, *handle), file))
            })
            .collect::<Result<_, KernelError>>()?;
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
