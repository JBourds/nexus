use anyhow::{Context, Result, bail, ensure};
use log::{error, info, warn};
use std::process::Command;
use std::rc::Rc;

pub fn parse(text: String) -> Result<Simulation> {
    let parsed: raw::Simulation = toml::from_str(text.as_str())
        .context("Failed to parse simulation parameters from config file.")?;
    let validated = Simulation::validate(parsed)
        .context("Failed to validate simulation parameters from config file.")?;
    Ok(validated)
}

fn expand_home(path: &str) -> std::path::PathBuf {
    if let Some(stripped) = path.strip_prefix("~/") {
        if let Some(home_dir) = home::home_dir() {
            return home_dir.join(stripped);
        }
    }
    std::path::PathBuf::from(path)
}

use std::collections::{HashMap, HashSet};

// Just use the string names as handles here since they will be small
// strings and the overhead of cloning a Rc is likely slower
type NodeHandle = String;
type LinkHandle = String;
type ProtocolHandle = String;

#[derive(Debug)]
pub struct Simulation {
    params: Params,
    links: HashMap<LinkHandle, Link>,
    nodes: HashMap<NodeHandle, Node>,
}
impl Simulation {
    /// Guaranteed to not have a cycle because it only traces links with a
    /// common ancestor to the default link, which is a sink node.
    fn topological_sort(
        node: String,
        dependencies: &HashMap<String, Vec<String>>,
        ordering: &mut Vec<String>,
    ) {
        ordering.push(node.clone());
        if let Some(work_list) = dependencies.get(&node) {
            for dependent in work_list.iter() {
                Self::topological_sort(dependent.to_string(), dependencies, ordering);
            }
        }
    }

    fn validate(val: raw::Simulation) -> Result<Self> {
        let params =
            Params::validate(val.params).context("Unable to validate simulation parameters")?;

        // Convert all the links to lowercase
        let mut links =
            val.links
                .into_iter()
                .fold(HashMap::new(), |mut map, (mut name, mut link)| {
                    name.make_ascii_lowercase();
                    if let Some(ref mut inherit) = link.inherit {
                        inherit.make_ascii_lowercase();
                    }
                    if let Some(ref mut next) = link.next {
                        next.make_ascii_lowercase();
                    }
                    map.insert(name, link);
                    map
                });

        // Create a dependency graph mapping links to children
        let mut link_dependencies = HashMap::new();
        for (name, link) in links.iter_mut() {
            // Resolve/check inheritance relation here
            let inherit = match link.inherit.as_ref() {
                Some(other) => {
                    if other == Link::DIRECT || other == Link::INDIRECT {
                        bail!("Cannot use reserved link name \"{}\"", Link::DIRECT);
                    }
                    other.to_string()
                }
                None => Link::DEFAULT.to_string(),
            };
            link.inherit = Some(inherit.clone());
            // Make sure an entry exists for both the parent and child
            let _ = link_dependencies.entry(name.to_string()).or_insert(vec![]);
            let parent = link_dependencies.entry(inherit).or_insert(vec![]);
            if *name != Link::DEFAULT {
                parent.push(name.to_string());
            }
        }

        // Create a vector with the topological ordering of inheritance
        let mut ordering = vec![];
        Self::topological_sort(Link::DEFAULT.to_string(), &link_dependencies, &mut ordering);

        // Check for a cycle - look for any inheritance chains that aren't in
        // the topological ordering since it means they had no common ancestor
        // to the "ideal" or "none" chain.
        if link_dependencies.len() != ordering.len() {
            for entry in ordering.iter() {
                let _ = link_dependencies
                    .remove(entry)
                    .expect("These should all definitely be there");
            }
            let keys = link_dependencies.keys().collect::<Vec<&String>>();
            // TODO: Make this actually find all the cycles rather than just report
            // that they exist
            bail!(
                "Detected one or more cycles in the inheritance relations found in the following keys: {keys:?}"
            );
        }

        // Now that the topological ordering is complete, process links in the
        // order we created
        let mut processed = HashMap::new();
        let _ = processed.insert(Link::DEFAULT.to_string(), Link::default());
        let link_handles = ordering
            .clone()
            .into_iter()
            .collect::<HashSet<LinkHandle>>();
        // Skip 1 for the default link we insert since that will always be first
        for key in ordering.iter().skip(1) {
            let link = links
                .remove(key)
                .expect("Topological ordering is derived from links map so this should be okay.");
            let res = Link::validate(link, &link_handles, &processed)
                .context(format!("Unable to process link \"{}\"", key))?;
            let _ = processed.insert(key.to_string(), res);
        }

        let node_handles = val
            .nodes
            .keys()
            .into_iter()
            .map(|s| s.to_string())
            .collect::<HashSet<String>>();
        let nodes = val
            .nodes
            .into_iter()
            .map(|(key, node)| {
                Node::validate(node, &node_handles, &link_handles).map(|node| (key, node))
            })
            .collect::<Result<HashMap<NodeHandle, Node>>>()
            .context("Failed to validate nodes")?;

        Ok(Self {
            params,
            links: processed,
            nodes,
        })
    }
}

#[derive(Debug)]
pub struct Params {
    timestep_length: f32,
    timesteps: u64,
    seed: u16,
    root: std::path::PathBuf,
}
impl Params {
    fn validate(val: raw::Params) -> Result<Self> {
        let root = expand_home(val.root.as_str());

        match root.try_exists() {
            Ok(true) => {}
            Ok(false) => {
                bail!(
                    "Unable to find root for simulation at path \"{}\"",
                    root.display()
                );
            }
            err => {
                err.context(format!(
                    "Could not verify whether root for simulation exists or not at path \"{:?}\"",
                    root
                ))?;
            }
        }
        if !root.is_dir() {
            bail!("Protocol root at \"{}\" is not a directory", root.display());
        }
        Ok(Self {
            timesteps: val.timesteps,
            timestep_length: val.timestep_length,
            seed: val.seed,
            root,
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Delay {
    modifier: Modifier,
    avg: f32,
    std: f32,
}
impl Delay {
    fn validate(val: raw::Delay) -> Result<Self> {
        let modifier = Modifier::validate(val.modifier).context("Unable to validate delay.")?;
        Ok(Self {
            modifier,
            avg: val.avg,
            std: val.std,
        })
    }
}
impl Default for Delay {
    fn default() -> Self {
        Self {
            modifier: Modifier::Flat,
            avg: 0.0,
            std: 0.0,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Modifier {
    Flat,
    Linear,
    Logarithmic,
    Exponental,
}
impl Modifier {
    fn validate(mut val: raw::Modifier) -> Result<Self> {
        val.make_ascii_lowercase();
        let variant = match val.as_str() {
            "flat" => Self::Flat,
            "linear" => Self::Linear,
            "log" => Self::Logarithmic,
            "exp" => Self::Exponental,
            s => {
                bail!("Expected to find (\"flat\" | \"linear\" | \"log\" | \"exp\") but found {s}");
            }
        };
        Ok(variant)
    }
}

#[derive(Debug)]
pub struct Link {
    next: Option<LinkHandle>,
    bit_error: f32,
    intermediaries: u32,
    packet_loss: f32,
    packet_loss_mod: Modifier,
    trans_rate: f64,
    queue_delay: Delay,
    processing_delay: Delay,
    connection_delay: Delay,
    propagation_delay: Delay,
}
impl Link {
    const DEFAULT: &'static str = "ideal";
    const SELF: &'static str = "self";
    const NONE: &'static str = "none";
    const DIRECT: &'static str = "direct";
    const INDIRECT: &'static str = "indirect";

    /// Ensure provided values for links are valid and
    /// resolve inheritance.
    fn validate(
        val: raw::Link,
        link_handles: &HashSet<LinkHandle>,
        processed: &HashMap<LinkHandle, Self>,
    ) -> Result<Self> {
        let ancestor = processed
            .get(&val.inherit.expect("This should have been filled in"))
            .expect("Ancestory should have been resolved by now");
        let next = if let Some(next) = val.next {
            if next == Link::NONE {
                None
            } else {
                ensure!(
                    link_handles.contains(&next),
                    "Link points to nonexistent next link \"{next}\""
                );
                Some(next)
            }
        } else {
            None
        };
        let bit_error = val.bit_error.unwrap_or(ancestor.bit_error);
        let intermediaries = val.intermediaries.unwrap_or(ancestor.intermediaries);
        let packet_loss = val.packet_loss.unwrap_or(ancestor.packet_loss);
        let packet_loss_mod = val
            .packet_loss_mod
            .map(Modifier::validate)
            .unwrap_or(Ok(ancestor.packet_loss_mod))
            .context("Unable to validate link modifier")?;
        let trans_rate = val.trans_rate.unwrap_or(ancestor.trans_rate);
        let queue_delay = val
            .queue_delay
            .map(Delay::validate)
            .unwrap_or(Ok(ancestor.queue_delay))
            .context("Unable to validate link queue delay")?;
        let processing_delay = val
            .processing_delay
            .map(Delay::validate)
            .unwrap_or(Ok(ancestor.processing_delay))
            .context("Unable to validate link processing delay")?;
        let connection_delay = val
            .connection_delay
            .map(Delay::validate)
            .unwrap_or(Ok(ancestor.connection_delay))
            .context("Unable to validate link connection delay")?;
        let propagation_delay = val
            .propagation_delay
            .map(Delay::validate)
            .unwrap_or(Ok(ancestor.propagation_delay))
            .context("Unable to validate link propagation delay")?;
        Ok(Self {
            next,
            bit_error,
            intermediaries,
            packet_loss,
            packet_loss_mod,
            trans_rate,
            queue_delay,
            processing_delay,
            connection_delay,
            propagation_delay,
        })
    }
}
impl Default for Link {
    fn default() -> Self {
        Self {
            next: None,
            bit_error: 0.0,
            intermediaries: 0,
            packet_loss: 0.0,
            packet_loss_mod: Modifier::Flat,
            trans_rate: f64::INFINITY,
            queue_delay: Delay::default(),
            processing_delay: Delay::default(),
            connection_delay: Delay::default(),
            propagation_delay: Delay::default(),
        }
    }
}

#[derive(Debug)]
pub struct Position {
    x: i64,
    y: i64,
}

#[derive(Debug)]
pub struct Node {
    positions: Vec<Position>,
    protocols: HashMap<ProtocolHandle, NodeProtocol>,
}
impl Node {
    const SELF: &'static str = "self";
    fn validate(
        mut val: raw::Node,
        node_handles: &HashSet<NodeHandle>,
        link_handles: &HashSet<LinkHandle>,
    ) -> Result<Self> {
        // No duplicate internal names
        let mut links = HashSet::new();
        for handle in val.internal_names.iter_mut() {
            handle.0.make_ascii_lowercase();
            if !links.insert(handle.0.clone()) {
                bail!("Node contains duplicate links with handle \"{}\"", handle.0);
            }
        }
        // These can be duplicated with internal links
        for mut handle in link_handles.clone().into_iter() {
            handle.make_ascii_lowercase();
            let _ = links.insert(handle);
        }

        let positions = val
            .positions
            .into_iter()
            .map(|p| Position { x: p.x, y: p.y })
            .collect();

        // No duplicate protocol names
        let mut protocol_names = HashSet::new();
        for protocol in val.protocols.iter_mut() {
            protocol.name.make_ascii_lowercase();
            if !protocol_names.insert(protocol.name.clone()) {
                bail!("Found duplicate protocol: \"{}\"", protocol.name);
            }
        }

        // Validate each protocol
        let protocols = val
            .protocols
            .into_iter()
            .map(|protocol| {
                let name = protocol.name.clone();
                NodeProtocol::validate(protocol, node_handles, &links)
                    .map(|validated| (name, validated))
            })
            .collect::<Result<HashMap<ProtocolHandle, NodeProtocol>>>()
            .context("Unable to validate node protocols")?;
        Ok(Self {
            positions,
            protocols,
        })
    }
}

#[derive(Debug)]
pub struct ConnectionRange {
    maximum: u64,
    offset: u64,
}

#[derive(Debug)]
pub struct Cmd {
    cmd: String,
    args: Vec<String>,
}

#[derive(Debug)]
pub struct NodeProtocol {
    root: std::path::PathBuf,
    runner: Cmd,
    accepts: HashSet<LinkHandle>,
    direct: HashMap<NodeHandle, HashSet<LinkHandle>>,
    indirect: HashMap<LinkHandle, ConnectionRange>,
}
impl NodeProtocol {
    fn validate(
        val: raw::NodeProtocol,
        node_handles: &HashSet<NodeHandle>,
        link_handles: &HashSet<LinkHandle>,
    ) -> Result<Self> {
        let root = expand_home(val.root.as_str());
        match root.try_exists() {
            Ok(true) => {}
            Ok(false) => {
                bail!(
                    "Unable to find root for node protocol \"{}\" at path \"{}\"",
                    val.name,
                    root.display()
                );
            }
            err => {
                err.context(format!("Could not verify whether root for node protocol \"{}\" exists or not at path \"{:?}\"", val.name, root))?;
            }
        }
        if !root.is_dir() {
            bail!("Protocol root at \"{}\" is not a directory", root.display());
        }
        let runner = Cmd {
            cmd: val.runner,
            args: val.runner_args,
        };
        let mut accepts = HashSet::new();
        for link in val.accepts.into_iter() {
            if !link_handles.contains(&link.0) {
                bail!(
                    "Protocol \"{}\" accepts nonexistent link \"{}\".",
                    val.name,
                    link.0
                )
            }
            let _ = accepts.insert(link.0);
        }
        // Ensure:
        // 1. There are no nonexistent nodes or links in the list.
        // 2. There are no duplicate entries for a given (node, link) pair.
        let mut direct = HashMap::new();
        for conn in val.direct.into_iter() {
            if !node_handles.contains(&conn.node.0) && conn.node.0 != Node::SELF {
                bail!(
                    "Protocol \"{}\" has link \"{}\" to nonexistent node \"{}\".",
                    val.name,
                    conn.link.0,
                    conn.node.0,
                )
            }
            let entry = direct.entry(conn.node.0.clone()).or_insert(HashSet::new());
            if !link_handles.contains(&conn.link.0.clone()) {
                bail!(
                    "Protocol \"{}\" has nonexistent link \"{}\" to node \"{}\".",
                    val.name,
                    conn.link.0,
                    conn.node.0,
                )
            } else if !entry.insert(conn.link.0.clone()) {
                bail!(
                    "Protocol \"{}\" has duplicate link \"{}\" to node \"{}\".",
                    val.name,
                    conn.link.0,
                    conn.node.0,
                )
            }
        }
        let mut indirect = HashMap::new();
        for conn in val.indirect.into_iter() {
            if !link_handles.contains(&conn.link.0) {
                bail!(
                    "Protocol \"{}\" has nonexistent indirect link \"{}\".",
                    val.name,
                    conn.link.0,
                )
            }
            if indirect
                .insert(
                    conn.link.0.clone(),
                    ConnectionRange {
                        maximum: conn.max_range,
                        offset: conn.modifier_offset,
                    },
                )
                .is_some()
            {
                bail!(
                    "Protocol \"{}\" has duplicate indirect link \"{}\".",
                    val.name,
                    conn.link.0,
                )
            }
        }
        Ok(Self {
            root,
            runner,
            accepts,
            direct,
            indirect,
        })
    }
}

mod raw {
    use serde::Deserialize;
    use std::collections::HashMap;
    #[derive(Debug, Default, Deserialize)]
    #[serde(default, deny_unknown_fields)]
    pub struct Simulation {
        pub(super) params: Params,
        pub(super) links: HashMap<String, Link>,
        pub(super) nodes: HashMap<String, Node>,
    }

    #[derive(Debug, Deserialize)]
    #[serde(default, deny_unknown_fields)]
    pub struct Params {
        pub(super) timestep_length: f32,
        pub(super) timesteps: u64,
        pub(super) seed: u16,
        pub(super) root: String,
    }

    impl Default for Params {
        fn default() -> Self {
            Self {
                timestep_length: 0.01,
                timesteps: 1_000_000,
                seed: 42,
                root: String::from("~/testnet/simulations"),
            }
        }
    }

    pub(super) type Modifier = String;

    #[derive(Debug, Deserialize)]
    #[serde(default, deny_unknown_fields)]
    pub struct Delay {
        pub(super) modifier: Modifier,
        pub(super) avg: f32,
        pub(super) std: f32,
    }
    impl Default for Delay {
        fn default() -> Self {
            Self {
                modifier: String::from("flat"),
                avg: 0.0,
                std: 0.0,
            }
        }
    }

    #[derive(Debug, Default, Deserialize)]
    pub struct LinkName(pub String);

    #[derive(Debug, Default, Deserialize)]
    #[serde(default, deny_unknown_fields)]
    pub struct Link {
        pub(super) inherit: Option<String>,
        pub(super) next: Option<String>,
        pub(super) bit_error: Option<f32>,
        pub(super) intermediaries: Option<u32>,
        pub(super) packet_loss: Option<f32>,
        pub(super) packet_loss_mod: Option<Modifier>,
        pub(super) trans_rate: Option<f64>,
        pub(super) queue_delay: Option<Delay>,
        pub(super) processing_delay: Option<Delay>,
        pub(super) connection_delay: Option<Delay>,
        pub(super) propagation_delay: Option<Delay>,
    }

    #[derive(Debug, Default, Deserialize)]
    pub struct NodeName(pub String);

    #[derive(Debug, Default, Deserialize)]
    pub struct ProtocolName(pub String);

    #[derive(Debug, Deserialize)]
    pub struct Position {
        pub(super) x: i64,
        pub(super) y: i64,
    }

    #[derive(Debug, Default, Deserialize)]
    pub struct IndirectConnection {
        pub(super) max_range: u64,
        pub(super) modifier_offset: u64,
        pub(super) link: LinkName,
    }

    #[derive(Debug, Default, Deserialize)]
    pub struct DirectConnection {
        pub(super) node: NodeName,
        pub(super) link: LinkName,
    }

    #[derive(Debug, Default, Deserialize)]
    #[serde(default, deny_unknown_fields)]
    pub struct Node {
        pub(super) positions: Vec<Position>,
        pub(super) internal_names: Vec<ProtocolName>,
        pub(super) protocols: Vec<NodeProtocol>,
    }

    #[derive(Debug, Default, Deserialize)]
    #[serde(default, deny_unknown_fields)]
    pub struct NodeProtocol {
        pub(super) name: String,
        pub(super) root: String,
        pub(super) runner: String,
        pub(super) runner_args: Vec<String>,
        pub(super) accepts: Vec<LinkName>,
        pub(super) direct: Vec<DirectConnection>,
        pub(super) indirect: Vec<IndirectConnection>,
    }
}
