use super::parse;
use crate::helpers::*;
use anyhow::{Context, Result, bail};
use std::path::PathBuf;
use std::rc::Rc;
use std::{
    collections::{HashMap, HashSet},
    num::NonZeroU64,
};

pub type NodeHandle = String;
pub type LinkHandle = String;
pub type ProtocolHandle = String;

#[derive(Clone, Debug)]
pub struct Simulation {
    pub params: Params,
    pub links: HashMap<LinkHandle, Link>,
    pub nodes: HashMap<NodeHandle, Node>,
}

#[derive(Clone, Default, Debug)]
pub struct Position {
    pub coordinates: Vec<Coordinate>,
    pub unit: DistanceUnit,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Coordinate {
    pub point: Point,
    pub orientation: Orientation,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Point {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Orientation {
    pub az: f64,
    pub el: f64,
    pub roll: f64,
}

#[derive(Clone, Debug)]
pub struct Node {
    pub position: Position,
    pub internal_names: Vec<ProtocolHandle>,
    pub protocols: HashMap<ProtocolHandle, NodeProtocol>,
}

#[derive(Clone, Debug)]
pub struct NodeProtocol {
    pub root: PathBuf,
    pub runner: Cmd,
    pub accepts: HashSet<LinkHandle>,
    pub direct: HashMap<NodeHandle, HashSet<LinkHandle>>,
    pub indirect: HashSet<LinkHandle>,
}

#[derive(Clone, Debug)]
pub struct Cmd {
    pub cmd: String,
    pub args: Vec<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Signal {
    pub range: ConnectionRange,
    pub shape: SignalShape,
    pub unit: DistanceUnit,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SignalShape {
    Omnidirectional,
    Cone,
    Direct,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ConnectionRange {
    pub maximum: Option<f64>,
    pub offset: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimestepConfig {
    pub length: u64,
    pub unit: TimeUnit,
    pub count: NonZeroU64,
}

#[derive(Clone, Debug)]
pub struct Params {
    pub timestep: TimestepConfig,
    pub seed: u64,
    pub root: PathBuf,
}

#[derive(Clone, Debug, Default)]
pub struct Link {
    pub signal: Signal,
    pub transmission: Rate,
    pub bit_error: DistanceProbVar,
    pub packet_loss: DistanceProbVar,
    delays: DelayCalculator,
}

#[derive(Clone)]
pub struct DelayCalculator {
    pub transmission: Rate,
    pub processing: Rate,
    pub propagation: Rc<dyn Fn(f64, DistanceUnit) -> f64>,
    pub ts_config: TimestepConfig,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Delays {
    pub transmission: Rate,
    pub processing: Rate,
    pub propagation: DistanceTimeVar,
}

/// Expression of `x` (distance) which is equal to the duration in `unit`s
/// for an event to occur (ex. Bits to propagate).
#[derive(Clone, Debug, PartialEq)]
pub struct DistanceTimeVar {
    pub rate: meval::Expr,
    pub time: TimeUnit,
    pub distance: DistanceUnit,
}

/// Expression of `x` (distance) which equals the probability of an event.
#[derive(Clone, Debug, PartialEq)]
pub struct DistanceProbVar {
    pub rate: meval::Expr,
    pub distance: DistanceUnit,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rate {
    pub rate: u64,
    pub data: DataUnit,
    pub time: TimeUnit,
}

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

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TimeUnit {
    Seconds,
    Milliseconds,
    Microseconds,
    Nanoseconds,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DistanceUnit {
    Millimeters,
    Centimeters,
    Meters,
    Kilometers,
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

    /// Return the left shift ratio of left / right with a boolean
    /// flag to indicate whether it was the left (true) or right
    /// (false) which is the numerator in the expression.
    pub fn ratio(left: Self, right: Self) -> (bool, usize) {
        let left = left.lshifts();
        let right = right.lshifts();
        let left_greater = left > right;
        let ratio = std::cmp::max(left, right) - std::cmp::min(left, right);
        (left_greater, ratio)
    }

    pub fn lshifts(&self) -> usize {
        match self {
            DataUnit::Bit => 0,
            DataUnit::Kilobit => 10,
            DataUnit::Megabit => 20,
            DataUnit::Gigabit => 30,
            DataUnit::Byte => 3,
            DataUnit::Kilobyte => 13,
            DataUnit::Megabyte => 23,
            DataUnit::Gigabyte => 33,
        }
    }
}

impl Default for DataUnit {
    fn default() -> Self {
        Self::Bit
    }
}

impl TimeUnit {
    fn validate(mut val: parse::Unit) -> Result<Self> {
        val.0.make_ascii_lowercase();
        let variant = match val.0.as_str() {
            "seconds" | "s" => Self::Seconds,
            "milliseconds" | "ms" => Self::Milliseconds,
            "microseconds" | "us" => Self::Microseconds,
            "nanoseconds" | "ns" => Self::Nanoseconds,
            s => {
                bail!("Expected to find a valid time unit but found \"{s}\"");
            }
        };
        Ok(variant)
    }

    /// Return the log_10 ratio of left / right with a boolean
    /// flag to indicate whether it was the left (true) or right
    /// (false) which is the numerator in the expression.
    pub fn ratio(left: Self, right: Self) -> (bool, usize) {
        let left = left.power();
        let right = right.power();
        let left_greater = left > right;
        let ratio = std::cmp::max(left, right) - std::cmp::min(left, right);
        (left_greater, ratio)
    }

    pub fn power(&self) -> usize {
        match self {
            TimeUnit::Seconds => 0,
            TimeUnit::Milliseconds => 3,
            TimeUnit::Microseconds => 6,
            TimeUnit::Nanoseconds => 9,
        }
    }
}

impl Default for TimeUnit {
    fn default() -> Self {
        Self::Milliseconds
    }
}

impl DistanceUnit {
    fn validate(mut val: parse::Unit) -> Result<Self> {
        val.0.make_ascii_lowercase();
        let variant = match val.0.as_str() {
            "millimeters" | "mm" => Self::Millimeters,
            "centimeters" | "cm" => Self::Centimeters,
            "meters" | "m" => Self::Meters,
            "kilometers" | "km" => Self::Kilometers,
            s => {
                bail!("Expected to find a valid distance unit but found \"{s}\"");
            }
        };
        Ok(variant)
    }

    /// Return the log_10 ratio of left / right with a boolean
    /// flag to indicate whether it was the left (true) or right
    /// (false) which is the numerator in the expression.
    pub fn ratio(left: Self, right: Self) -> (bool, usize) {
        let left = left.power();
        let right = right.power();
        let left_greater = left > right;
        let ratio = std::cmp::max(left, right) - std::cmp::min(left, right);
        (left_greater, ratio)
    }

    pub fn power(&self) -> usize {
        match self {
            DistanceUnit::Millimeters => 0,
            DistanceUnit::Centimeters => 3,
            DistanceUnit::Meters => 6,
            DistanceUnit::Kilometers => 9,
        }
    }
}

impl Default for DistanceUnit {
    fn default() -> Self {
        Self::Kilometers
    }
}

impl Rate {
    fn validate(val: parse::Rate) -> Result<Self> {
        let data = val
            .data
            .map(DataUnit::validate)
            .unwrap_or(Ok(DataUnit::default()))
            .context("Unable to validate rate's data unit")?;
        let time = val
            .time
            .map(TimeUnit::validate)
            .unwrap_or(Ok(TimeUnit::default()))
            .context("Unable to validate rate's time unit")?;
        let rate = val.rate.unwrap_or(u64::MAX);
        Ok(Self { rate, data, time })
    }
}

impl Default for Rate {
    fn default() -> Self {
        Self {
            rate: u64::MAX,
            data: DataUnit::default(),
            time: TimeUnit::default(),
        }
    }
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
            let res = Link::validate(link, &processed, params.timestep)
                .context(format!("Unable to process link \"{}\"", key))?;
            let _ = processed.insert(key.to_string(), res);
        }

        let node_handles = val
            .nodes
            .keys()
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

        if nodes.iter().all(|(_, node)| node.protocols.is_empty()) {
            bail!("Must have at least one node protocol defined to run a simulation!");
        } else if nodes
            .iter()
            .all(|(_, node)| node.position.coordinates.is_empty())
        {
            bail!(
                "Must have at least one node position defined to run a simulation. \
            If your simulation does not require a fixed position, satisfy this requirement \
            by placing a blank coordinate in the `positions` field.

            Ex. `position.coordinates = [{{}}]"
            );
        }

        Ok(Self {
            params,
            links: processed,
            nodes,
        })
    }
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
    const DEFAULT_TIMESTEP_LEN: u64 = 1;
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
        let length = val.length.unwrap_or(Self::DEFAULT_TIMESTEP_LEN);
        Ok(Self {
            length,
            count,
            unit,
        })
    }
}

impl Params {
    fn validate(config_root: &PathBuf, val: parse::Params) -> Result<Self> {
        let root = resolve_directory(config_root, &PathBuf::from(val.root))?;
        let timestep = val
            .timestep
            .map(TimestepConfig::validate)
            .unwrap_or(Ok(TimestepConfig::default()))
            .context("Unable to validate timestep configuration in simulation config.")?;
        Ok(Self {
            timestep,
            seed: val.seed.unwrap_or_default(),
            root,
        })
    }
}

impl Delays {
    fn validate(val: parse::Delays) -> Result<Self> {
        let transmission = val
            .transmission
            .map(Rate::validate)
            .unwrap_or(Ok(Rate::default()))
            .context("Unable to validate transmission delay rate.")?;
        let processing = val
            .processing
            .map(Rate::validate)
            .unwrap_or(Ok(Rate::default()))
            .context("Unable to validate processing delay rate.")?;
        let propagation = val
            .propagation
            .map(DistanceTimeVar::validate)
            .unwrap_or(Ok(DistanceTimeVar::default()))
            .context("Unable to validate propagation delay rate.")?;
        Ok(Self {
            transmission,
            processing,
            propagation,
        })
    }
}

impl DelayCalculator {
    fn validate(delays: Delays, ts_config: TimestepConfig) -> Result<Self> {
        let DistanceTimeVar {
            rate,
            time,
            distance: distance_unit,
        } = delays.propagation;
        let Ok(func) = rate.bind("x") else {
            bail!("Link rates must be a one variable function of distance \"x\"");
        };
        Ok(Self {
            transmission: delays.transmission,
            processing: delays.processing,
            propagation: Rc::new(move |distance: f64, unit: DistanceUnit| {
                // Number of `distance_unit` / `time_unit` for value of `distance`
                let dist_time_units = func(distance);
                let (distance_prop_greater, distance_ratio) =
                    DistanceUnit::ratio(distance_unit, unit);
                // Scale distance units
                let scalar = 10u64
                    .checked_pow(distance_ratio.try_into().unwrap())
                    .expect("Exponentiation overflow.") as f64;
                let (distance_num, distance_den) = if distance_prop_greater {
                    (dist_time_units, scalar)
                } else {
                    (dist_time_units * scalar, 1.0)
                };
                // Scale time units
                let (time_prop_greater, time_ratio) = TimeUnit::ratio(time, ts_config.unit);
                let scalar = 10_u64
                    .checked_pow(time_ratio.try_into().unwrap())
                    .expect("Exponentiation overflow.") as f64;
                if time_prop_greater {
                    distance_num * scalar / distance_den
                } else {
                    distance_num / distance_den * scalar
                }
            }),
            ts_config,
        })
    }

    /// Determine how many timesteps are required to delay for based on the
    /// distance of the transmission and amount of data to transmit.
    pub fn timestep_delay(
        &self,
        distance: f64,
        amount: u64,
        data_unit: DataUnit,
        distance_unit: DistanceUnit,
    ) -> u64 {
        let (proc_num, proc_den) =
            Self::timesteps_required(amount, data_unit, self.processing, self.ts_config);
        let (trans_num, trans_den) =
            Self::timesteps_required(amount, data_unit, self.transmission, self.ts_config);
        let prop_timesteps = (self.propagation)(distance, distance_unit);
        let mut num = proc_num * trans_den + trans_num * proc_den;
        let den = proc_den * trans_den;
        num += (prop_timesteps * den as f64) as u64;
        num.div_ceil(den)
    }

    /// Determine `u64` timesteps required to transmit `amount` `unit`s
    /// of data given the `rate` data flows and the `config` for timesteps.
    fn timesteps_required(
        amount: u64,
        unit: DataUnit,
        rate: Rate,
        config: TimestepConfig,
    ) -> (u64, u64) {
        // Determine which data unit is larger (higher magnitude), and how many
        // left shifts are needed to align them.
        let (data_tx_greater, data_ratio) = DataUnit::ratio(rate.data, unit);
        let (data_num, data_den) = if data_tx_greater {
            (
                amount,
                rate.rate
                    .checked_shl(data_ratio.try_into().unwrap())
                    .expect("Left shift overflow."),
            )
        } else {
            (
                amount
                    .checked_shl(data_ratio.try_into().unwrap())
                    .expect("Left shift overflow."),
                rate.rate,
            )
        };
        // Determine which time unit is larger (higher magnitude), and how many
        // powers of 10 the difference is by.
        let (time_tx_greater, time_ratio) = TimeUnit::ratio(rate.time, config.unit);
        let scalar = 10_u64
            .checked_pow(time_ratio.try_into().unwrap())
            .expect("Exponentiation overflow.");
        if time_tx_greater {
            (data_num * scalar, data_den)
        } else {
            (data_num, data_den * scalar)
        }
    }
}

impl Default for DelayCalculator {
    fn default() -> Self {
        Self {
            transmission: Default::default(),
            processing: Default::default(),
            propagation: Rc::new(|_, _| 0.0),
            ts_config: Default::default(),
        }
    }
}

impl std::fmt::Debug for DelayCalculator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "DelayCalculator {{ .. }}")
    }
}

impl Default for DistanceTimeVar {
    fn default() -> Self {
        Self {
            rate: "0".parse().unwrap(),
            time: Default::default(),
            distance: Default::default(),
        }
    }
}

impl DistanceTimeVar {
    fn validate(val: parse::DistanceTimeVar) -> Result<Self> {
        let def = Self::default();
        let rate = val.rate.unwrap_or(def.rate);
        let time = if let Some(time) = val.time {
            TimeUnit::validate(time).context("Unable to validate distance time unit.")?
        } else {
            def.time
        };
        let distance = if let Some(distance) = val.distance {
            DistanceUnit::validate(distance).context("Unable to validate distance unit.")?
        } else {
            def.distance
        };
        Ok(Self {
            rate,
            time,
            distance,
        })
    }
}

impl Default for DistanceProbVar {
    fn default() -> Self {
        Self {
            rate: "0".parse().unwrap(),
            distance: DistanceUnit::default(),
        }
    }
}

impl DistanceProbVar {
    fn validate(val: parse::DistanceProbVar) -> Result<Self> {
        let def = Self::default();
        let rate = val.rate.unwrap_or(def.rate);
        let distance = if let Some(distance) = val.distance {
            DistanceUnit::validate(distance).context("Unable to validate distance unit.")?
        } else {
            def.distance
        };
        Ok(Self { rate, distance })
    }
}

impl Link {
    const DEFAULT: &'static str = "ideal";
    const DIRECT: &'static str = "direct";
    const INDIRECT: &'static str = "indirect";

    /// Ensure provided values for links are valid and
    /// resolve inheritance.
    fn validate(
        val: parse::Link,
        processed: &HashMap<LinkHandle, Self>,
        ts_config: TimestepConfig,
    ) -> Result<Self> {
        let ancestor = processed
            .get(&val.inherit.expect("This should have been filled in"))
            .expect("Ancestory should have been resolved by now");
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
            .map(DistanceProbVar::validate)
            .unwrap_or(Ok(ancestor.bit_error.clone()))
            .context("Unable to validate link bit error variable.")?;
        let packet_loss = val
            .packet_loss
            .map(DistanceProbVar::validate)
            .unwrap_or(Ok(ancestor.packet_loss.clone()))
            .context("Unable to validate link packet loss variable.")?;
        let delays = if let Some(delays) = val.delays {
            let delays = Delays::validate(delays).context("Failed to validate link delays.")?;
            DelayCalculator::validate(delays, ts_config)
                .context("Unable to create delay calculator.")?
        } else {
            ancestor.delays.clone()
        };
        Ok(Self {
            signal,
            transmission,
            bit_error,
            packet_loss,
            delays,
        })
    }
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

        let unit = val
            .unit
            .map(DistanceUnit::validate)
            .unwrap_or(Ok(DistanceUnit::default()))
            .context("Unable to validate distance units for node position")?;
        Ok(Self { coordinates, unit })
    }
}

impl Coordinate {
    /// Return 3D euclidean distance between two points
    /// after converting to a common unit system.
    pub fn distance(from: &Self, to: &Self) -> f64 {
        let x = from.point.x - to.point.x;
        let y = from.point.y - to.point.y;
        let z = from.point.z - to.point.z;
        (x * x + y * y + z * z).sqrt()
    }
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

impl Orientation {
    fn validate(val: parse::Orientation) -> Self {
        Self {
            az: val.az.unwrap_or_default(),
            el: val.el.unwrap_or_default(),
            roll: val.roll.unwrap_or_default(),
        }
    }
}

impl Node {
    pub const SELF: &'static str = "self";
    fn validate(
        config_root: &PathBuf,
        val: parse::Node,
        node_handles: &HashSet<NodeHandle>,
        link_handles: &HashSet<LinkHandle>,
    ) -> Result<Self> {
        // No duplicate internal names
        let mut internal_names = HashSet::new();
        if let Some(names) = val.internal_names {
            for name in names {
                if !internal_names.insert(name.0.to_lowercase()) {
                    bail!("Node contains duplicate links with name \"{}\"", name.0);
                }
            }
        }

        // These can be duplicated with internal links
        let valid_links = link_handles
            .iter()
            .map(|s| s.to_lowercase())
            .chain(internal_names.clone())
            .collect();

        let position = Position::validate(val.position.unwrap_or_default())
            .context("Unable to validate node positioning of nodes with class")?;

        // No duplicate protocol names
        let mut protocol_names = HashSet::new();
        let mut protocols = val.protocols.unwrap_or_default();
        for protocol in protocols.iter_mut() {
            protocol.name.make_ascii_lowercase();
            if protocol.name.is_empty() {
                bail!("Protocols must have unique, non-empty names.");
            } else if protocol.runner.is_empty() {
                bail!("Must provide non-empty command to run protocol program.");
            } else if !protocol_names.insert(protocol.name.clone()) {
                bail!("Found duplicate protocol: \"{}\"", protocol.name);
            }
        }

        // Validate each protocol
        let protocols = protocols
            .into_iter()
            .map(|protocol| {
                let name = protocol.name.clone();
                NodeProtocol::validate(config_root, protocol, node_handles, &valid_links)
                    .map(|validated| (name, validated))
            })
            .collect::<Result<HashMap<ProtocolHandle, NodeProtocol>>>()
            .context("Unable to validate node protocols")?;
        Ok(Self {
            position,
            internal_names: internal_names.into_iter().collect(),
            protocols,
        })
    }
}

impl Signal {
    fn validate(val: parse::Signal) -> Result<Self> {
        let maximum = val
            .max_range
            .map(|maximum| {
                verify_nonnegative(maximum).context("Maximum distance must be positive.")
            })
            .transpose()?;
        let offset = val
            .offset
            .map(|maximum| verify_nonnegative(maximum).context("Distance offset must be positive."))
            .transpose()?;
        let range = ConnectionRange { maximum, offset };
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
impl std::fmt::Display for Cmd {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {}", self.cmd, self.args.join(" "))
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delay_calculator() {
        let ts_config = TimestepConfig {
            length: 1,
            unit: TimeUnit::Seconds,
            count: NonZeroU64::new(1000).unwrap(),
        };
        let transmission = Rate {
            rate: 200,
            data: DataUnit::Bit,
            time: TimeUnit::Seconds,
        };
        let processing = Rate {
            rate: 200,
            data: DataUnit::Bit,
            time: TimeUnit::Seconds,
        };
        let propagation = DistanceTimeVar {
            rate: "5 * x".parse().unwrap(),
            time: TimeUnit::Seconds,
            distance: DistanceUnit::Kilometers,
        };
        let delays = Delays {
            transmission,
            processing,
            propagation,
        };
        let calculator = DelayCalculator::validate(delays, ts_config).unwrap();
        let tests = [
            ((0.0001, 0, DataUnit::Bit, DistanceUnit::Kilometers), 1),
            ((0.0, 1, DataUnit::Bit, DistanceUnit::Kilometers), 1),
            ((0.0, 100, DataUnit::Bit, DistanceUnit::Kilometers), 1),
            ((1.0, 0, DataUnit::Bit, DistanceUnit::Kilometers), 5),
            ((1.0, 200, DataUnit::Bit, DistanceUnit::Kilometers), 7),
            ((1.4, 200, DataUnit::Bit, DistanceUnit::Kilometers), 9),
            ((1.9, 200, DataUnit::Bit, DistanceUnit::Kilometers), 12),
            ((2.0, 200, DataUnit::Bit, DistanceUnit::Kilometers), 12),
        ];
        for ((distance, amount, data_unit, distance_unit), expected) in tests.into_iter() {
            assert_eq!(
                calculator.timestep_delay(distance, amount, data_unit, distance_unit),
                expected
            );
        }
    }
}
