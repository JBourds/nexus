use super::parse;
use crate::ast::*;
use crate::helpers::*;
use crate::parse::Deployment;
use anyhow::ensure;
use anyhow::{Context, Result, bail};
use std::path::PathBuf;
use std::{
    collections::{HashMap, HashSet},
    num::NonZeroU64,
};

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
                    max_size,
                    read_own_writes,
                }
            }
            parse::ChannelType::Exclusive {
                ttl,
                unit,
                nbuffered,
                max_size,
            } => {
                let unit = unit
                    .map(TimeUnit::validate)
                    .unwrap_or(Ok(TimeUnit::default()))
                    .context("Failed to validate time unit when parsing channel type.")?;
                let max_size = max_size.unwrap_or(Self::MSG_MAX_DEFAULT);
                Self::Exclusive {
                    ttl,
                    unit,
                    nbuffered,
                    max_size,
                }
            }
        };
        Ok(val)
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

    fn trace_link_dependencies(
        links: &mut HashMap<LinkHandle, parse::Link>,
    ) -> Result<Vec<LinkHandle>> {
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
        Ok(ordering)
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

        // Now that the topological ordering is complete, process links in the
        // order we created
        let ordering = Self::trace_link_dependencies(&mut links)?;
        let mut processed = HashMap::new();
        let _ = processed.insert(Link::DEFAULT.to_string(), Link::default());
        // Skip 1 for the default link we insert since that will always be first
        for key in ordering.iter().skip(1) {
            let link = links
                .remove(key)
                .expect("Topological ordering is derived from links map so this should be okay.");
            let res = Link::validate(link, &processed, params.timestep)
                .context(format!("Unable to process link \"{}\"", key))?;
            let _ = processed.insert(key.to_string(), res);
        }

        let channels: HashMap<_, _> = val
            .channels
            .into_iter()
            .map(|(name, channel)| Channel::validate(channel, &processed).map(|val| (name, val)))
            .collect::<Result<_>>()
            .context("Failed to validate channels.")?;

        let channel_handles = channels.keys().cloned().collect::<HashSet<_>>();
        let nodes = val
            .nodes
            .into_iter()
            .map(|(key, node)| {
                Node::validate(config_root, node, &channel_handles).map(|nodes| (key, nodes))
            })
            .collect::<Result<HashMap<NodeHandle, Vec<Node>>>>()
            .context("Failed to validate nodes")?;
        let flat_nodes: Vec<_> = nodes.values().flatten().collect();

        if flat_nodes.iter().all(|node| node.protocols.is_empty()) {
            bail!("Must have at least one node protocol defined to run a simulation!");
        } else if flat_nodes.is_empty() {
            bail!(
                "Must have at least one node position defined to run a simulation. \
            If your simulation does not require a fixed position, satisfy this requirement \
            by placing a blank coordinate in the `positions` field.

            Ex. `deployments = [{{}}]"
            );
        }

        Ok(Self {
            params,
            nodes,
            channels,
        })
    }
}

impl TimestepConfig {
    pub(crate) const DEFAULT_TIMESTEP_LEN: u64 = 1;
    pub(crate) const DEFAULT_TIMESTEP_COUNT: NonZeroU64 = NonZeroU64::new(1_000_000).unwrap();

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
    pub(crate) fn validate(delays: Delays, ts_config: TimestepConfig) -> Result<Self> {
        if delays.propagation.rate.clone().bind("x").is_err() {
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
        rate: Rate,
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

impl Node {
    pub const SELF: &'static str = "self";
    fn validate(
        config_root: &PathBuf,
        val: parse::Node,
        channel_handles: &HashSet<ChannelHandle>,
    ) -> Result<Vec<Self>> {
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

        // Check that all internal names were used
        let internal_names_used = protocols
            .values()
            .flat_map(|p| p.inbound.iter().chain(p.outbound.iter()))
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
        for Deployment {
            position,
            extra_args,
        } in deployments
        {
            let protocols = protocols
                .clone()
                .into_iter()
                .map(|(name, protocol)| {
                    let protocol = if let Some(extra_args) = extra_args.clone() {
                        let Cmd { cmd, mut args } = protocol.runner;
                        args.extend(extra_args);
                        NodeProtocol {
                            runner: Cmd { cmd, args },
                            ..protocol
                        }
                    } else {
                        protocol
                    };
                    (name, protocol)
                })
                .collect::<HashMap<_, _>>();
            let position = position
                .map(Position::validate)
                .unwrap_or(Ok(Position::default()))
                .context("Failed to validate node coordinates.")?;
            nodes.push(Node {
                position,
                internal_names: internal_names.clone().into_iter().collect(),
                protocols,
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
        let outbound = val
            .outbound
            .unwrap_or_default()
            .into_iter()
            .map(|ch| {
                if channel_handles.contains(&ch.0) {
                    Ok(ch.0)
                } else {
                    bail!(
                        "Could not find outbound channel \"{}\" in protocol \"{}\"",
                        ch.0,
                        val.name
                    )
                }
            })
            .collect::<Result<_>>()?;
        let inbound = val
            .inbound
            .unwrap_or_default()
            .into_iter()
            .map(|ch| {
                if channel_handles.contains(&ch.0) {
                    Ok(ch.0)
                } else {
                    bail!(
                        "Could not find inbound channel \"{}\" in protocol \"{}\"",
                        ch.0,
                        val.name
                    )
                }
            })
            .collect::<Result<_>>()?;

        Ok(Self {
            root,
            runner,
            outbound,
            inbound,
        })
    }
}
