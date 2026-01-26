use super::namespace::Namespace;
use super::parse;
use crate::CONTROL_FILES;
use crate::RESERVED_LINKS;
use crate::ast::*;
use crate::helpers::*;
use crate::parse::Deployment;
use crate::parse::PowerSink;
use crate::parse::PowerSource;
use crate::parse::Unit;
use crate::units::*;
use anyhow::ensure;
use anyhow::{Context, Result, bail};
use chrono::Utc;
use std::path::PathBuf;
use std::time::SystemTime;
use std::{
    collections::{HashMap, HashSet},
    num::NonZeroU64,
};

impl ClockUnit {
    fn validate(mut val: parse::Unit) -> Result<Self> {
        let case_insensitive_len = val.0.len() - 1;
        val.0[..case_insensitive_len].make_ascii_lowercase();
        let variant = match val.0.as_str() {
            "hertz" | "hz" => Self::Hertz,
            "kilohertz" | "khz" => Self::Kilohertz,
            "megahertz" | "mhz" => Self::Megahertz,
            "gigahertz" | "ghz" => Self::Gigahertz,
            s => {
                bail!("Expected a clock unit but found \"{s}\"");
            }
        };
        Ok(variant)
    }
}

impl DataUnit {
    fn validate_byte_aligned(mut val: parse::Unit) -> Result<Self> {
        let case_insensitive_len = val.0.len() - 1;
        val.0[..case_insensitive_len].make_ascii_lowercase();
        let variant = match val.0.as_str() {
            "bytes" | "byte" | "b" => Self::Byte,
            "kilobytes" | "kilobyte" | "kb" => Self::Kilobyte,
            "megabytes" | "megabyte" | "mb" => Self::Megabyte,
            "gigabytes" | "gigabyte" | "gb" => Self::Gigabyte,
            s => {
                bail!("Expected a valid data unit aligned to bytes but found \"{s}\"");
            }
        };
        Ok(variant)
    }
    fn validate(mut val: parse::Unit) -> Result<Self> {
        let case_insensitive_len = val.0.len() - 1;
        val.0[..case_insensitive_len].make_ascii_lowercase();
        let variant = match val.0.as_str() {
            "bits" | "bit" | "b" => Self::Bit,
            "kilobits" | "kilobit" | "kb" => Self::Kilobit,
            "megabits" | "megabit" | "mb" => Self::Megabit,
            "gigabits" | "gigabit" | "gb" => Self::Gigabit,
            "bytes" | "byte" | "B" => Self::Byte,
            "kilobytes" | "kilobyte" | "kB" => Self::Kilobyte,
            "megabytes" | "megabyte" | "mB" => Self::Megabyte,
            "gigabytes" | "gigabyte" | "gB" => Self::Gigabyte,
            s => {
                bail!("Expected a valid data unit but found \"{s}\"");
            }
        };
        Ok(variant)
    }
}

impl PowerUnit {
    fn validate(mut val: parse::Unit) -> Result<Self> {
        val.0[1..].make_ascii_lowercase();
        let variant = match val.0.as_str() {
            "Nanowatt" | "nanowatt" | "nw" | "Nw" => Self::NanoWatt,
            "Microwatt" | "microwatt" | "uw" | "Uw" => Self::MicroWatt,
            "Milliwatt" | "milliwatt" | "mw" => Self::MilliWatt,
            "Watt" | "watt" | "w" => Self::Watt,
            "Kilowatt" | "kilowatt" | "Kw" | "kw" => Self::KiloWatt,
            "Megawatt" | "megawatt" | "Mw" => Self::MegaWatt,
            "Gigawatt" | "gigawatt" | "Gw" | "gw" => Self::GigaWatt,
            s => {
                bail!("Expected to find a valid power unit but found \"{s}\"");
            }
        };
        Ok(variant)
    }
}

impl TimeUnit {
    fn validate(mut val: parse::Unit) -> Result<Self> {
        val.0.make_ascii_lowercase();
        let variant = match val.0.as_str() {
            "hours" | "h" => Self::Hours,
            "minutes" | "m" => Self::Minutes,
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
}

impl DataRate {
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
        let rate = val.rate.unwrap_or(i64::MAX as u64);
        Ok(Self { rate, data, time })
    }
}

impl Channel {
    fn validate(val: parse::Channel, links: &HashMap<LinkHandle, Link>) -> Result<Self> {
        let link = val.link.map(|link| link.0).unwrap_or("ideal".to_string());
        let Some(link) = links.get(&link).cloned() else {
            bail!("Could not find link \"{link}\" in simulated links.");
        };
        let r#type =
            ChannelType::validate(val.r#type).context("Failed to validate channel type.")?;
        Ok(Self { link, r#type })
    }
}

impl ChannelType {
    fn validate(val: parse::ChannelType) -> Result<Self> {
        let val = match val {
            parse::ChannelType::Shared {
                ttl,
                unit,
                max_size,
                read_own_writes,
            } => {
                let unit = unit
                    .map(TimeUnit::validate)
                    .unwrap_or(Ok(TimeUnit::default()))
                    .context("Failed to validate time unit when parsing channel type.")?;
                let max_size = max_size.unwrap_or(Self::MSG_MAX_DEFAULT);
                let read_own_writes = read_own_writes.unwrap_or_default();
                Self::Shared {
                    ttl,
                    unit,
                    read_own_writes,
                    max_size,
                }
            }
            parse::ChannelType::Exclusive {
                ttl,
                unit,
                nbuffered,
                max_size,
                read_own_writes,
            } => {
                let unit = unit
                    .map(TimeUnit::validate)
                    .unwrap_or(Ok(TimeUnit::default()))
                    .context("Failed to validate time unit when parsing channel type.")?;
                let max_size = max_size.unwrap_or(Self::MSG_MAX_DEFAULT);
                let read_own_writes = read_own_writes.unwrap_or_default();
                Self::Exclusive {
                    ttl,
                    unit,
                    nbuffered,
                    max_size,
                    read_own_writes,
                }
            }
        };
        Ok(val)
    }
}

fn link_namespace(mut links: HashMap<String, parse::Link>) -> Result<Namespace<parse::Link>> {
    let mut ns = Namespace::<parse::Link>::new(String::from("Links"));
    for (_, l) in links.iter_mut() {
        if let Some(ref mut inherit) = l.inherit {
            inherit.make_ascii_lowercase();
        }
    }
    ns.ban_names(&HashSet::from(RESERVED_LINKS))?
        .add_entries(links)?;
    Ok(ns)
}

fn source_namespace(sources: Vec<PowerSource>) -> Result<Namespace<PowerRate>> {
    let mut ns = Namespace::<PowerRate>::new(String::from("PowerSource"));
    for (name, v) in sources.into_iter().map(
        |PowerSource {
             name,
             quantity,
             unit,
             time,
         }| (name, PowerRate::validate(quantity, unit, time, true)),
    ) {
        ns.add(name, v?)?;
    }
    Ok(ns)
}

fn sink_namespace(sinks: Vec<PowerSink>) -> Result<Namespace<PowerRate>> {
    let mut ns = Namespace::<PowerRate>::new(String::from("PowerSink"));
    for (name, v) in sinks.into_iter().map(
        |PowerSink {
             name,
             quantity,
             unit,
             time,
         }| (name, PowerRate::validate(quantity, unit, time, true)),
    ) {
        ns.add(name, v?)?;
    }
    Ok(ns)
}

fn channel_namespace(
    channels: HashMap<String, parse::Channel>,
    processed: &HashMap<LinkHandle, Link>,
) -> Result<Namespace<Channel>> {
    let mut ns = Namespace::<Channel>::new(String::from("Channel"));
    ns.ban_names(&HashSet::from(CONTROL_FILES))?;
    for (name, channel) in channels {
        ns.add(name, Channel::validate(channel, processed)?)?;
    }
    Ok(ns)
}

impl Simulation {
    /// Used for time dilation to increase the amount of CPU time given to each
    /// node.
    pub(crate) fn scale_cpu(&mut self, ratio: f64) {
        for (_, node) in self.nodes.iter_mut() {
            node.resources.scale_cpu(ratio);
        }
    }

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

    fn trace_link_dependencies(
        links: &mut HashMap<LinkHandle, parse::Link>,
    ) -> Result<Vec<LinkHandle>> {
        // Create a dependency graph mapping links to children
        let mut link_dependencies = HashMap::new();
        for (name, link) in links.iter_mut() {
            // Resolve/check inheritance relation here
            let inherit = match link.inherit.as_ref() {
                Some(other) => other.to_string(),
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
        Ok(ordering)
    }

    pub(crate) fn validate(config_root: &PathBuf, val: parse::Simulation) -> Result<Self> {
        let params = Params::validate(config_root, val.params)
            .context("Unable to validate simulation parameters")?;

        let mut links: HashMap<_, _> = link_namespace(val.links)?.into();
        // Now that the topological ordering is complete, process links in the
        // order we created
        let ordering = Self::trace_link_dependencies(&mut links)?;

        let mut processed = HashMap::new();
        let _ = processed
            .insert(Link::DEFAULT.to_string(), Link::default())
            .is_some();
        // Skip 1 for the default link we insert since that will always be first
        for key in ordering.iter().skip(1) {
            let link = links
                .remove(key)
                .expect("Topological ordering is derived from links map so this should be okay.");
            let res = Link::validate(link, &processed, params.timestep)
                .context(format!("Unable to process link \"{}\"", key))?;
            let _ = processed.insert(key.to_string(), res);
        }

        let sources: HashMap<_, _> = source_namespace(val.sources.unwrap_or_default())?.into();
        let source_names: HashSet<_> = sources.keys().cloned().collect();

        let sinks: HashMap<_, _> = sink_namespace(val.sinks.unwrap_or_default())?.into();
        let sink_names: HashSet<_> = sinks.keys().cloned().collect();

        let channels: HashMap<_, _> = channel_namespace(val.channels, &processed)?.into();
        let channel_handles = channels.keys().cloned().collect::<HashSet<_>>();
        let validated_nodes = val
            .nodes
            .into_iter()
            // Append a unique suffix corresponding to deployment ID to each
            // node's name to deduplicate the handles
            .map(|(key, node)| {
                Node::validate(
                    config_root,
                    node,
                    &channel_handles,
                    &sink_names,
                    &source_names,
                )
                .map(|nodes| {
                    nodes
                        .into_iter()
                        .enumerate()
                        .map(|(index, node)| (format!("{key}.{index}"), node))
                        .collect::<Vec<_>>()
                })
            })
            // Collect the intermediary step
            .collect::<Result<Vec<Vec<(NodeHandle, Node)>>>>()
            .context("Failed to validate nodes")?;
        // Flatten 2D array of nodes into unique handles
        let nodes = validated_nodes
            .into_iter()
            .flatten()
            .collect::<HashMap<NodeHandle, Node>>();

        if nodes.values().all(|node| node.protocols.is_empty()) {
            bail!("Must have at least one node protocol defined to run a simulation!");
        } else if nodes.is_empty() {
            bail!(
                "Must have at least one node position defined to run a simulation. \
            If your simulation does not require a fixed position, satisfy this requirement \
            by placing a blank coordinate in the `positions` field.

            Ex. `deployments = [{{}}]"
            );
        }

        let mut res = Self {
            params,
            nodes,
            channels,
            sinks,
            sources,
        };
        res.scale_cpu(res.params.time_dilation);
        Ok(res)
    }
}

impl Cpu {
    fn new(cores: Option<NonZeroU64>, hertz: Option<NonZeroU64>, unit: ClockUnit) -> Self {
        Self { cores, unit, hertz }
    }
}

impl Mem {
    fn new(amount: Option<NonZeroU64>, unit: DataUnit) -> Self {
        Self { amount, unit }
    }
}

impl Resources {
    fn validate(val: parse::Resources) -> Result<Self> {
        let clock_units = val
            .clock_units
            .map(ClockUnit::validate)
            .unwrap_or(Ok(ClockUnit::default()))
            .context("Failed to validate clock rate. Please provide in hz, khz, mhz, or ghz.")?;
        let ram_units = val
            .ram_units
            .map(DataUnit::validate_byte_aligned)
            .unwrap_or(Ok(DataUnit::default()))
            .context("Failed to validate ram units.")?;
        let cpu = Cpu::new(val.cores, val.clock_rate, clock_units);
        let mem = Mem::new(val.ram, ram_units);
        Ok(Self { cpu, mem })
    }
}

impl TimestepConfig {
    pub(crate) const DEFAULT_TIMESTEP_LEN: NonZeroU64 = NonZeroU64::new(1).unwrap();
    pub(crate) const DEFAULT_TIMESTEP_COUNT: NonZeroU64 = NonZeroU64::new(1_000_000).unwrap();

    fn validate(val: parse::TimestepConfig) -> Result<Self> {
        let unit = val
            .unit
            .map(TimeUnit::validate)
            .unwrap_or(Ok(TimeUnit::default()))
            .context("Unable to validate time unit in timestep config")?;
        if matches!(unit, TimeUnit::Minutes | TimeUnit::Hours) {
            bail!("Simulation timestamp must be in seconds or smaller.");
        }
        let count = val
            .count
            .map(NonZeroU64::new)
            .unwrap_or_default()
            .context("Unable to validate time unit in timestep config")?;
        let length = val
            .length
            .map(NonZeroU64::new)
            .unwrap_or(Some(Self::DEFAULT_TIMESTEP_LEN))
            .context("Unable to validate time unit in timestep config")?;
        let start = val
            .start
            .map(toml_datetime_to_system_time)
            .unwrap_or(SystemTime::now());
        Ok(Self {
            length,
            count,
            unit,
            start,
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
        let time_dilation = val.time_dilation.unwrap_or(1.0);
        Ok(Self {
            timestep,
            seed: val.seed.unwrap_or_default(),
            root,
            time_dilation,
        })
    }
}

impl Delays {
    fn validate(val: parse::Delays) -> Result<Self> {
        let transmission = val
            .transmission
            .map(DataRate::validate)
            .unwrap_or(Ok(DataRate::default()))
            .context("Unable to validate transmission delay rate.")?;
        let processing = val
            .processing
            .map(DataRate::validate)
            .unwrap_or(Ok(DataRate::default()))
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
    pub(crate) fn validate(delays: Delays, ts_config: TimestepConfig) -> Result<Self> {
        if delays
            .propagation
            .rate
            .parse::<meval::Expr>()?
            .bind("x")
            .is_err()
        {
            bail!("Link rates must be a one variable function of distance \"x\"");
        };
        Ok(Self {
            transmission: delays.transmission,
            processing: delays.processing,
            propagation: delays.propagation,
            ts_config,
        })
    }

    /// Determine `u64` timesteps required to transmit `amount` `unit`s
    /// of data given the `rate` data flows and the `config` for timesteps.
    pub(crate) fn timesteps_required(
        amount: u64,
        unit: DataUnit,
        rate: DataRate,
        config: TimestepConfig,
    ) -> (u64, u64) {
        // Determine which data unit is larger (higher magnitude), and how many
        // left shifts are needed to align them.
        let (should_scale_down, data_ratio) = DataUnit::ratio(rate.data, unit);
        let (data_num, data_den) = if should_scale_down {
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
        let (should_scale_down, time_ratio) = TimeUnit::ratio(rate.time, config.unit);
        let scalar = 10_u64
            .checked_pow(time_ratio.try_into().unwrap())
            .expect("Exponentiation overflow.");
        if should_scale_down {
            (data_num, data_den * scalar)
        } else {
            (data_num * scalar, data_den)
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

impl DistanceProbVar {
    fn validate(val: parse::DistanceProbVar) -> Result<Self> {
        let def = Self::default();
        let rate = val.rate.unwrap_or(def.rate);
        if rate.parse::<meval::Expr>()?.bind2("x", "y").is_err() {
            bail!(
                "Distance probability variable must be a function of \"x\" (distance) and \"y\" (data)"
            );
        }
        let distance = if let Some(distance) = val.distance {
            DistanceUnit::validate(distance).context("Unable to validate distance unit.")?
        } else {
            def.distance
        };
        let size = if let Some(size) = val.size {
            DataUnit::validate(size).context("Unable to validate size unit.")?
        } else {
            def.size
        };
        Ok(Self {
            rate,
            distance,
            size,
        })
    }
}

impl Link {
    const DEFAULT: &'static str = "ideal";

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
            bit_error,
            packet_loss,
            delays,
        })
    }
}

impl Charge {
    fn validate(val: parse::Charge) -> Result<Self> {
        Ok(Self {
            quantity: val.quantity,
            unit: PowerUnit::validate(val.unit)
                .context("Failed to validate poer unit in charge.")?,
        })
    }
}

impl Position {
    fn validate(val: parse::Coordinate) -> Result<Self> {
        let point = val.point.map(Point::validate).unwrap_or_default();
        let orientation = val
            .orientation
            .map(Orientation::validate)
            .unwrap_or_default();
        let unit = val
            .unit
            .map(DistanceUnit::validate)
            .unwrap_or(Ok(DistanceUnit::default()))
            .context("Unable to validate distance units for node position")?;
        Ok(Self {
            point,
            orientation,
            unit,
        })
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

impl PowerRate {
    fn validate(quantity: u64, unit: Unit, time: Unit, is_source: bool) -> Result<Self> {
        let multiplier: i64 = if is_source { 1 } else { -1 };
        let quantity: i64 = quantity
            .try_into()
            .context("Power quantity too large to fit in i64")?;
        Ok(PowerRate {
            rate: multiplier * quantity,
            unit: PowerUnit::validate(unit).context("Failed to validate power unit")?,
            time: TimeUnit::validate(time).context("Failed to validate time unit")?,
        })
    }
}

impl Node {
    pub const SELF: &'static str = "self";
    fn validate(
        config_root: &PathBuf,
        val: parse::Node,
        channel_handles: &HashSet<ChannelHandle>,
        sink_handles: &HashSet<SinkHandle>,
        source_handles: &HashSet<SourceHandle>,
    ) -> Result<Vec<Self>> {
        let resources = val
            .resources
            .map(Resources::validate)
            .unwrap_or(Ok(Resources::default()))
            .context("Failed to validate node resource allocation.")?;
        // No duplicate internal names
        let mut internal_names = HashSet::new();
        if let Some(names) = val.internal_names {
            for name in names {
                if !internal_names.insert(name.0.to_lowercase()) {
                    bail!("Node contains duplicate channels with name \"{}\"", name.0);
                }
            }
        }

        // These can be duplicated with internal channels
        let valid_channels = channel_handles
            .iter()
            .map(|s| s.to_lowercase())
            .chain(internal_names.clone())
            .collect();

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
                NodeProtocol::validate(config_root, protocol, &valid_channels)
                    .map(|validated| (name, validated))
            })
            .collect::<Result<HashMap<ProtocolHandle, NodeProtocol>>>()
            .context("Unable to validate node protocols")?;

        // Validate sinks/source names are valid
        let sink_names: HashSet<_> = val.sinks.unwrap_or_default().into_iter().collect();
        let sink_diff: HashSet<_> = sink_names.difference(sink_handles).collect();
        ensure!(
            sink_diff.is_empty(),
            "Found undefined sink names in node: {sink_diff:#?}"
        );
        let source_names: HashSet<_> = val.sources.unwrap_or_default().into_iter().collect();
        let source_diff: HashSet<_> = source_names.difference(source_handles).collect();
        ensure!(
            source_diff.is_empty(),
            "Found undefined source names in node: {source_diff:#?}"
        );

        // Check that all internal names were used
        let internal_names_used = protocols
            .values()
            .flat_map(|p| p.subscribers.iter().chain(p.publishers.iter()))
            .cloned()
            .collect::<HashSet<_>>();
        let difference = internal_names
            .difference(&internal_names_used)
            .collect::<Vec<_>>();
        ensure!(
            difference.is_empty(),
            format!("Found unused internal channels: {difference:#?}")
        );

        let mut nodes = vec![];
        let Some(deployments) = val.deployments else {
            bail!("Node cannot be defined without a single deployment location.");
        };

        // Share immutable data across deployments when possible
        for Deployment {
            position,
            run_args: deployment_run_args,
            build_args: deployment_build_args,
            charge,
        } in deployments
        {
            let deployment_run_args = deployment_run_args.unwrap_or_default();
            let deployment_build_args = deployment_build_args.unwrap_or_default();
            let protocols = protocols
                .clone()
                .into_iter()
                .map(|(name, protocol)| {
                    let Cmd {
                        cmd: run_cmd,
                        args: mut run_args,
                    } = protocol.runner;
                    run_args.extend(deployment_run_args.clone());
                    let Cmd {
                        cmd: build_cmd,
                        args: mut build_args,
                    } = protocol.build;
                    build_args.extend(deployment_build_args.clone());
                    let protocol = NodeProtocol {
                        runner: Cmd {
                            cmd: run_cmd,
                            args: run_args,
                        },
                        build: Cmd {
                            cmd: build_cmd,
                            args: build_args,
                        },
                        ..protocol
                    };
                    (name, protocol)
                })
                .collect::<HashMap<_, _>>();
            let position = position
                .map(Position::validate)
                .unwrap_or(Ok(Position::default()))
                .context("Failed to validate node coordinates.")?;
            // Allow user to omit charge, but if they specify it
            // then it must pass validation.
            let charge = if let Some(charge) = charge {
                Some(Charge::validate(charge)?)
            } else {
                None
            };

            nodes.push(Node {
                charge,
                position,
                resources: resources.clone(),
                internal_names: internal_names.iter().cloned().collect(),
                protocols,
                sinks: sink_names.clone(),
                sources: source_names.clone(),
            });
        }
        Ok(nodes)
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

impl NodeProtocol {
    fn validate(
        config_root: &PathBuf,
        val: parse::NodeProtocol,
        channel_handles: &HashSet<ChannelHandle>,
    ) -> Result<Self> {
        let root = resolve_directory(config_root, &PathBuf::from(val.root))?;
        let runner = Cmd {
            cmd: val.runner,
            args: val.runner_args.unwrap_or_default(),
        };
        let build = Cmd {
            cmd: val.build,
            args: val.build_args.unwrap_or_default(),
        };
        let publishers = val
            .publishers
            .unwrap_or_default()
            .into_iter()
            .map(|ch| {
                if channel_handles.contains(&ch.0) {
                    Ok(ch.0)
                } else {
                    bail!(
                        "Could not find publishers channel \"{}\" in protocol \"{}\"",
                        ch.0,
                        val.name
                    )
                }
            })
            .collect::<Result<_>>()?;
        let subscribers = val
            .subscribers
            .unwrap_or_default()
            .into_iter()
            .map(|ch| {
                if channel_handles.contains(&ch.0) {
                    Ok(ch.0)
                } else {
                    bail!(
                        "Could not find subscribers channel \"{}\" in protocol \"{}\"",
                        ch.0,
                        val.name
                    )
                }
            })
            .collect::<Result<_>>()?;

        Ok(Self {
            root,
            runner,
            build,
            publishers,
            subscribers,
        })
    }
}
fn toml_datetime_to_system_time(dt: toml::value::Datetime) -> SystemTime {
    let s = dt.to_string();
    let chrono_dt: chrono::DateTime<Utc> = s.parse().expect("invalid date format in toml file");
    SystemTime::from(chrono_dt)
}
