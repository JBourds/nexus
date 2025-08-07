use super::parse;
use anyhow::{Context, Result, bail, ensure};
use std::path::{Path, PathBuf};
use std::{
    collections::{HashMap, HashSet},
    num::NonZeroU64,
};

fn expand_home(path: &PathBuf) -> PathBuf {
    if let Some(stripped) = path.as_os_str().to_str().unwrap().strip_prefix("~/") {
        if let Some(home_dir) = home::home_dir() {
            return home_dir.join(stripped);
        }
    }
    PathBuf::from(path)
}

fn verify_nonnegative(val: f64) -> Result<f64> {
    if val.is_sign_negative() {
        bail!("Value must be positive")
    } else {
        Ok(val)
    }
}

fn resolve_directory(config_root: &PathBuf, path: &PathBuf) -> Result<PathBuf> {
    let root = expand_home(path);
    let root = if root.is_relative() {
        std::fs::canonicalize(Path::new(config_root).join(root))?
    } else {
        root
    };
    match root.try_exists() {
        Ok(true) => {}
        Ok(false) => {
            bail!(
                "Unable to find directory at path \"{}\"",
                path.to_string_lossy()
            );
        }
        err => {
            err.context(format!(
                "Could not verify whether root exists at path \"{:?}\"",
                root.to_string_lossy()
            ))?;
        }
    }
    if !root.is_dir() {
        bail!("Protocol root at \"{}\" is not a directory", root.display());
    } else {
        Ok(root)
    }
}

// Just use the string names as handles here since they will be small
// strings and the overhead of cloning a Rc is likely slower
pub type NodeHandle = String;
pub type LinkHandle = String;
pub type ProtocolHandle = String;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DataUnit {
    Bit,
    Kilobit,
    Megabit,
    Gigabit,
    Byte,
    Kilobyte,
    Megabyte,
    Gigabyte,
}

impl DataUnit {
    fn validate(mut val: parse::Unit) -> Result<Self> {
        let case_insensitive_len = val.0.len() - 1;
        val.0[..case_insensitive_len].make_ascii_lowercase();
        let variant = match val.0.as_str() {
            "bit" | "b" => Self::Bit,
            "kilobit" | "kb" => Self::Kilobit,
            "megabit" | "mb" => Self::Megabit,
            "gigabit" | "gb" => Self::Gigabit,
            "byte" | "B" => Self::Byte,
            "kilobyte" | "kB" => Self::Kilobyte,
            "megabyte" | "mB" => Self::Megabyte,
            "gigabyte" | "gB" => Self::Gigabyte,
            s => {
                bail!("Expected a valid data unit but found \"{s}\"");
            }
        };
        Ok(variant)
    }
}

impl Default for DataUnit {
    fn default() -> Self {
        Self::Bit
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TimeUnit {
    Hours,
    Minutes,
    Seconds,
    Milliseconds,
    Microseconds,
    Nanoseconds,
}

impl TimeUnit {
    fn validate(mut val: parse::Unit) -> Result<Self> {
        val.0.make_ascii_lowercase();
        let variant = match val.0.as_str() {
            "seconds" | "s" => Self::Seconds,
            "minutes" | "m" => Self::Minutes,
            "hours" | "h" => Self::Hours,
            "milliseconds" | "ms" => Self::Milliseconds,
            "microseconds" | "us" => Self::Microseconds,
            "nanoseconds" | "ns" => Self::Nanoseconds,
            s => {
                bail!("Expected to find a valid time unit but found \"{s}\"");
            }
        };
        Ok(variant)
    }
}

impl Default for TimeUnit {
    fn default() -> Self {
        Self::Seconds
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DistanceUnit {
    Meters,
    Kilometers,
    Feet,
    Yards,
    Miles,
}

impl DistanceUnit {
    fn validate(mut val: parse::Unit) -> Result<Self> {
        val.0.make_ascii_lowercase();
        let variant = match val.0.as_str() {
            "meters" | "m" => Self::Meters,
            "kilometers" | "km" => Self::Kilometers,
            "feet" => Self::Feet,
            "yards" => Self::Yards,
            "miles" | "mi" => Self::Miles,
            s => {
                bail!("Expected to find a valid distance unit but found \"{s}\"");
            }
        };
        Ok(variant)
    }
}

impl Default for DistanceUnit {
    fn default() -> Self {
        Self::Kilometers
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Modifier {
    Flat,
    Linear,
    Logarithmic,
    Exponental,
}

impl Modifier {
    fn validate(mut val: parse::Modifier) -> Result<Self> {
        val.0.make_ascii_lowercase();
        let variant = match val.0.as_str() {
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

impl Default for Modifier {
    fn default() -> Self {
        Self::Flat
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rate {
    pub rate: f64,
    pub data_unit: DataUnit,
    pub time_unit: TimeUnit,
}

impl Rate {
    fn validate(val: parse::Rate) -> Result<Self> {
        let data_unit = val
            .data_unit
            .map(DataUnit::validate)
            .unwrap_or(Ok(DataUnit::default()))
            .context("Unable to validate rate's data unit")?;
        let time_unit = val
            .time_unit
            .map(TimeUnit::validate)
            .unwrap_or(Ok(TimeUnit::default()))
            .context("Unable to validate rate's time unit")?;
        let rate = val
            .rate
            .map(verify_nonnegative)
            .unwrap_or(Ok(f64::default()))
            .context("Unable to validate rate's value.")?;
        Ok(Self {
            rate,
            data_unit,
            time_unit,
        })
    }
}

impl Default for Rate {
    fn default() -> Self {
        Self {
            rate: f64::INFINITY,
            data_unit: DataUnit::default(),
            time_unit: TimeUnit::default(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Simulation {
    pub params: Params,
    pub links: HashMap<LinkHandle, Link>,
    pub nodes: HashMap<NodeHandle, Node>,
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

    pub(crate) fn validate(config_root: &PathBuf, val: parse::Simulation) -> Result<Self> {
        let params = Params::validate(config_root, val.params)
            .context("Unable to validate simulation parameters")?;

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
        if link_dependencies.len() != ordering.len() && !link_dependencies.is_empty() {
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
                Node::validate(config_root, node, &node_handles, &link_handles)
                    .map(|node| (key, node))
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimestepConfig {
    pub length: f64,
    pub unit: TimeUnit,
    pub count: NonZeroU64,
}

impl Default for TimestepConfig {
    fn default() -> Self {
        Self {
            length: Self::DEFAULT_TIMESTEP_LEN,
            unit: TimeUnit::default(),
            count: Self::DEFAULT_TIMESTEP_COUNT,
        }
    }
}

impl TimestepConfig {
    const DEFAULT_TIMESTEP_LEN: f64 = 0.1;
    const DEFAULT_TIMESTEP_COUNT: NonZeroU64 = NonZeroU64::new(1_000_000).unwrap();

    fn validate(val: parse::TimestepConfig) -> Result<Self> {
        let unit = val
            .unit
            .map(TimeUnit::validate)
            .unwrap_or(Ok(TimeUnit::default()))
            .context("Unable to validate time unit in timestep config")?;
        let count = val
            .count
            .map(NonZeroU64::new)
            .unwrap_or_default()
            .context("Unable to validate time unit in timestep config")?;
        let length = val
            .length
            .map(verify_nonnegative)
            .unwrap_or(Ok(Self::DEFAULT_TIMESTEP_LEN))
            .context("Unable to validate length in timestep config")?;
        Ok(Self {
            length,
            count,
            unit,
        })
    }
}

#[derive(Clone, Debug)]
pub struct Params {
    pub timestep: TimestepConfig,
    pub intermediary_link_threshold: u32,
    pub seed: u16,
    pub root: PathBuf,
}
impl Params {
    const INTERMEDIARY_LINK_THRESHOLD_DEFAULT: u32 = 100;
    fn validate(config_root: &PathBuf, val: parse::Params) -> Result<Self> {
        let root = resolve_directory(config_root, &PathBuf::from(val.root))?;
        let timestep = val
            .timestep
            .map(TimestepConfig::validate)
            .unwrap_or(Ok(TimestepConfig::default()))
            .context("Unable to validate timestep configuration in simulation config.")?;
        Ok(Self {
            timestep,
            intermediary_link_threshold: val
                .intermediary_link_threshold
                .unwrap_or(Self::INTERMEDIARY_LINK_THRESHOLD_DEFAULT),
            seed: val.seed.unwrap_or_default(),
            root,
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DistanceVar {
    pub avg: f64,
    pub std: f64,
    pub modifier: Modifier,
    pub unit: DistanceUnit,
}
impl DistanceVar {
    fn validate(val: parse::DistanceVar) -> Result<Self> {
        let def = Self::default();
        let avg = val.avg.unwrap_or(def.avg);
        let std = val.std.unwrap_or(def.std);
        let unit = if let Some(unit) = val.unit {
            DistanceUnit::validate(unit).context("Unable to validate distance unit.")?
        } else {
            def.unit
        };
        let modifier = if let Some(modifier) = val.modifier {
            Modifier::validate(modifier).context("Unable to validate distance modifier.")?
        } else {
            def.modifier
        };
        Ok(Self {
            avg,
            std,
            modifier,
            unit,
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Link {
    pub next: Option<LinkHandle>,
    pub intermediaries: u32,
    pub signal: Signal,
    pub transmission: Rate,
    pub bit_error: DistanceVar,
    pub packet_loss: DistanceVar,
    pub queue_delay: DistanceVar,
    pub processing_delay: DistanceVar,
    pub connection_delay: DistanceVar,
    pub propagation_delay: DistanceVar,
}

impl Link {
    const DEFAULT: &'static str = "ideal";
    const NONE: &'static str = "none";
    const DIRECT: &'static str = "direct";
    const INDIRECT: &'static str = "indirect";

    /// Ensure provided values for links are valid and
    /// resolve inheritance.
    fn validate(
        val: parse::Link,
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
        let intermediaries = val.intermediaries.unwrap_or(ancestor.intermediaries);
        ensure!(
            !(next.is_none() && intermediaries > 0),
            "Cannot have a next link of \"none\" with nonzero intermediary links (found {intermediaries})"
        );

        let signal = val
            .signal
            .map(Signal::validate)
            .unwrap_or(Ok(ancestor.signal))
            .context("Unable to validate link signal")?;

        let transmission = val
            .transmission
            .map(Rate::validate)
            .unwrap_or(Ok(ancestor.transmission))
            .context("Unable to validate link transmission rate")?;
        let bit_error = val
            .bit_error
            .map(DistanceVar::validate)
            .unwrap_or(Ok(ancestor.bit_error))
            .context("Unable to validate link bit error variable.")?;
        let packet_loss = val
            .packet_loss
            .map(DistanceVar::validate)
            .unwrap_or(Ok(ancestor.packet_loss))
            .context("Unable to validate link packet loss variable.")?;
        let queue_delay = val
            .queue_delay
            .map(DistanceVar::validate)
            .unwrap_or(Ok(ancestor.queue_delay))
            .context("Unable to validate link queue delay variable.")?;
        let processing_delay = val
            .processing_delay
            .map(DistanceVar::validate)
            .unwrap_or(Ok(ancestor.processing_delay))
            .context("Unable to validate link processing delay variable.")?;
        let connection_delay = val
            .connection_delay
            .map(DistanceVar::validate)
            .unwrap_or(Ok(ancestor.connection_delay))
            .context("Unable to validate link connection delay variable.")?;
        let propagation_delay = val
            .propagation_delay
            .map(DistanceVar::validate)
            .unwrap_or(Ok(ancestor.propagation_delay))
            .context("Unable to validate link propagation delay variable.")?;
        Ok(Self {
            next,
            intermediaries,
            signal,
            transmission,
            bit_error,
            packet_loss,
            queue_delay,
            processing_delay,
            connection_delay,
            propagation_delay,
        })
    }
}

#[derive(Clone, Default, Debug)]
pub struct Position {
    pub coordinates: Vec<Coordinate>,
    pub units: DistanceUnit,
}

impl Position {
    fn validate(val: parse::Position) -> Result<Self> {
        let coordinates = val
            .coordinates
            .unwrap_or_default()
            .into_iter()
            .map(|c| Coordinate {
                point: c.point.map(Point::validate).unwrap_or_default(),
                orientation: c.orientation.map(Orientation::validate).unwrap_or_default(),
            })
            .collect();

        let units = val
            .units
            .map(DistanceUnit::validate)
            .unwrap_or(Ok(DistanceUnit::default()))
            .context("Unable to validate distance units for node position")?;
        Ok(Self { coordinates, units })
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Point {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Point {
    fn validate(val: parse::Point) -> Self {
        Self {
            x: val.x.unwrap_or_default(),
            y: val.y.unwrap_or_default(),
            z: val.z.unwrap_or_default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Orientation {
    pub az: f64,
    pub el: f64,
    pub roll: f64,
}

impl Orientation {
    fn validate(val: parse::Orientation) -> Self {
        Self {
            az: val.az.unwrap_or_default(),
            el: val.el.unwrap_or_default(),
            roll: val.roll.unwrap_or_default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Coordinate {
    pub point: Point,
    pub orientation: Orientation,
}

#[derive(Clone, Debug)]
pub struct Node {
    pub position: Position,
    pub protocols: HashMap<ProtocolHandle, NodeProtocol>,
}

impl Node {
    const SELF: &'static str = "self";
    fn validate(
        config_root: &PathBuf,
        val: parse::Node,
        node_handles: &HashSet<NodeHandle>,
        link_handles: &HashSet<LinkHandle>,
    ) -> Result<Self> {
        // No duplicate internal names
        let mut links = HashSet::new();
        let mut internal_names = val.internal_names.unwrap_or_default();
        for handle in internal_names.iter_mut() {
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

        let position = Position::validate(val.position.unwrap_or_default())
            .context("Unable to validate node positioning of nodes with class")?;

        // No duplicate protocol names
        let mut protocol_names = HashSet::new();
        let mut protocols = val.protocols.unwrap_or_default();
        for protocol in protocols.iter_mut() {
            protocol.name.make_ascii_lowercase();
            if !protocol_names.insert(protocol.name.clone()) {
                bail!("Found duplicate protocol: \"{}\"", protocol.name);
            }
        }

        // Validate each protocol
        let protocols = protocols
            .into_iter()
            .map(|protocol| {
                let name = protocol.name.clone();
                NodeProtocol::validate(config_root, protocol, node_handles, &links)
                    .map(|validated| (name, validated))
            })
            .collect::<Result<HashMap<ProtocolHandle, NodeProtocol>>>()
            .context("Unable to validate node protocols")?;
        Ok(Self {
            position,
            protocols,
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Signal {
    pub range: ConnectionRange,
    pub shape: SignalShape,
    pub unit: DistanceUnit,
}

impl Signal {
    fn validate(val: parse::Signal) -> Result<Self> {
        let range = ConnectionRange {
            maximum: val.max_range,
            offset: val.offset,
        };
        let shape = val
            .shape
            .map(SignalShape::validate)
            .unwrap_or(Ok(SignalShape::default()))
            .context("Unable to validate signal shape.")?;
        let unit = val
            .unit
            .map(DistanceUnit::validate)
            .unwrap_or(Ok(DistanceUnit::default()))
            .context("Unable to validate distance unit.")?;
        Ok(Self { range, shape, unit })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ConnectionRange {
    pub maximum: Option<u64>,
    pub offset: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SignalShape {
    Omnidirectional,
    Cone,
    Direct,
}

impl SignalShape {
    fn validate(mut val: parse::SignalShape) -> Result<Self> {
        val.0.make_ascii_lowercase();
        let variant = match val.0.as_str() {
            "omni" => Self::Omnidirectional,
            "cone" => Self::Cone,
            "direct" => Self::Direct,
            s => {
                bail!("Expected to find (\"omni\" | \"cone\" | \"direct\") but found {s}");
            }
        };
        Ok(variant)
    }
}

impl Default for SignalShape {
    fn default() -> Self {
        Self::Omnidirectional
    }
}

#[derive(Clone, Debug)]
pub struct Cmd {
    pub cmd: String,
    pub args: Vec<String>,
}

impl std::fmt::Display for Cmd {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {}", self.cmd, self.args.join(" "))
    }
}

#[derive(Clone, Debug)]
pub struct NodeProtocol {
    pub root: PathBuf,
    pub runner: Cmd,
    pub accepts: HashSet<LinkHandle>,
    pub direct: HashMap<NodeHandle, HashSet<LinkHandle>>,
    pub indirect: HashSet<LinkHandle>,
}

impl NodeProtocol {
    pub fn links(&self) -> HashSet<LinkHandle> {
        let mut links = self.outbound_links();
        links.extend(self.inbound_links());
        links
    }

    pub fn outbound_links(&self) -> HashSet<LinkHandle> {
        let mut links = HashSet::new();
        for link_set in self.direct.values() {
            links.extend(link_set.iter().cloned());
        }
        links.extend(self.indirect.iter().cloned());
        links
    }

    pub fn inbound_links(&self) -> HashSet<LinkHandle> {
        self.accepts.clone()
    }

    fn validate(
        config_root: &PathBuf,
        val: parse::NodeProtocol,
        node_handles: &HashSet<NodeHandle>,
        link_handles: &HashSet<LinkHandle>,
    ) -> Result<Self> {
        let root = resolve_directory(config_root, &PathBuf::from(val.root))?;
        let runner = Cmd {
            cmd: val.runner,
            args: val.runner_args.unwrap_or_default(),
        };
        let mut accepts = HashSet::new();
        let node_accepts = val.accepts.unwrap_or_default();
        for link in node_accepts.into_iter() {
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
        let node_direct_connections = val.direct.unwrap_or_default();
        for conn in node_direct_connections.into_iter() {
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
        let mut indirect = HashSet::new();
        let node_indirect_connections = val.indirect.unwrap_or_default();
        for link in node_indirect_connections.into_iter() {
            if !link_handles.contains(&link.0) {
                bail!(
                    "Protocol \"{}\" has nonexistent indirect link \"{}\".",
                    val.name,
                    link.0,
                )
            }
            if !indirect.insert(link.0.clone()) {
                bail!(
                    "Protocol \"{}\" has duplicate indirect link \"{}\".",
                    val.name,
                    link.0,
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
