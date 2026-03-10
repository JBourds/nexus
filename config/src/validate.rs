use super::namespace::Namespace;
use super::parse;
use crate::CONTROL_PREFIX;
use crate::RESERVED_LINKS;
use crate::ast::*;
use crate::helpers::*;
use crate::parse::Deployment;
use crate::units::{DecimalScaled, parse_duration_to_us};
use anyhow::ensure;
use anyhow::{Context, Result, bail};
use chrono::DateTime;
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

impl EnergyUnit {
    fn validate(mut val: parse::Unit) -> Result<Self> {
        val.0.make_ascii_lowercase();
        let variant = match val.0.as_str() {
            "nanojoule" | "nanojoules" | "nj" => Self::NanoJoule,
            "microjoule" | "microjoules" | "uj" => Self::MicroJoule,
            "millijoule" | "millijoules" | "mj" => Self::MilliJoule,
            "joule" | "joules" | "j" => Self::Joule,
            "kilojoule" | "kilojoules" | "kj" => Self::KiloJoule,
            "microwatthour" | "microwatthours" | "uwh" => Self::MicroWattHour,
            "milliwatthour" | "milliwatthours" | "mwh" => Self::MilliWattHour,
            "watthour" | "watthours" | "wh" => Self::WattHour,
            "kilowatthour" | "kilowatthours" | "kwh" => Self::KiloWattHour,
            s => {
                bail!("Expected a valid energy unit but found \"{s}\"");
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

fn channel_namespace(
    channels: HashMap<String, parse::Channel>,
    processed: &HashMap<LinkHandle, Link>,
) -> Result<Namespace<Channel>> {
    let mut ns = Namespace::<Channel>::new(String::from("Channel"));
    ns.ban_prefix(CONTROL_PREFIX)?;
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

    pub(crate) fn validate(config_root: &PathBuf, mut val: parse::Simulation) -> Result<Self> {
        // Apply profiles to nodes before validation (in order, so later
        // profiles layer on top of earlier ones).
        let profiles = val.profiles.take().unwrap_or_default();
        for (node_name, node) in val.nodes.iter_mut() {
            let profile_names = std::mem::take(&mut node.profile);
            for profile_name in &profile_names {
                // Case-insensitive profile lookup: module keys are stored lowercased.
                let key = profile_name.to_ascii_lowercase();
                let profile = profiles.get(&key).with_context(|| {
                    format!("Node \"{node_name}\" references unknown profile \"{profile_name}\"")
                })?;
                crate::module::apply_profile(node, profile);
            }
        }

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

        let channels: HashMap<_, _> = channel_namespace(val.channels, &processed)?.into();
        let channel_handles = channels.keys().cloned().collect::<HashSet<_>>();
        let validated_nodes = val
            .nodes
            .into_iter()
            // Append a unique suffix corresponding to deployment ID to each
            // node's name to deduplicate the handles
            .map(|(key, node)| {
                Node::validate(config_root, node, &params.timestep.start, &channel_handles).map(
                    |nodes| {
                        nodes
                            .into_iter()
                            .enumerate()
                            .map(|(index, node)| (format!("{key}.{index}"), node))
                            .collect::<Vec<_>>()
                    },
                )
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
        let parsed = delays
            .propagation
            .parsed_rate
            .as_ref()
            .expect("parsed_rate should be set by DistanceTimeVar::validate")
            .clone();
        if parsed.bind2("d", "distance").is_err() {
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
        let parsed_rate = Some(rate.parse::<meval::Expr>().context("Unable to parse rate expression")?);
        Ok(Self {
            rate,
            parsed_rate,
            time,
            distance,
        })
    }
}

impl RssiProbExpr {
    fn validate(val: parse::RssiProbExpr, noise_floor_dbm: f64) -> Result<Self> {
        let def = Self::default();
        let expr = val.0.unwrap_or(def.expr);
        let parsed_expr = expr.parse::<meval::Expr>()?;
        if parsed_expr.clone().bind2("snr", "rssi").is_err() {
            bail!("Distance probability variable must be a function of \"x\" (rssi)");
        }
        Ok(Self {
            expr,
            parsed_expr: Some(parsed_expr),
            noise_floor_dbm,
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
        let medium = val
            .medium
            .map(Medium::validate)
            .unwrap_or(Ok(ancestor.medium))
            .context("Unable to validate link medium")?;
        let noise_floor_dbm = medium.noise_floor_dbm();
        let bit_error = val
            .bit_error
            .map(|e| RssiProbExpr::validate(e, noise_floor_dbm))
            .unwrap_or(Ok(ancestor.bit_error.clone()))
            .context("Unable to validate link bit error variable.")?;
        let packet_loss = val
            .packet_loss
            .map(|e| RssiProbExpr::validate(e, noise_floor_dbm))
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
            medium,
            bit_error,
            packet_loss,
            delays,
        })
    }
}

impl Medium {
    pub fn validate(medium: parse::Medium) -> Result<Self> {
        match medium {
            parse::Medium::Wireless {
                shape,
                wavelength_meters,
                gain_dbi,
                rx_min_dbm,
                tx_min_dbm,
                tx_max_dbm,
            } => {
                if tx_min_dbm > tx_max_dbm {
                    bail!("cannot have tx_min_dbm > tx_max_dbm [{tx_min_dbm}, {tx_max_dbm}]");
                }
                let shape = shape
                    .map(SignalShape::validate)
                    .unwrap_or(Ok(SignalShape::default()))
                    .context("unable to validate signal shape in wireless link")?;
                Ok(Self::Wireless {
                    shape,
                    wavelength_meters,
                    gain_dbi,
                    rx_min_dbm,
                    tx_min_dbm,
                    tx_max_dbm,
                })
            }
            parse::Medium::Wired {
                rx_min_dbm,
                tx_min_dbm,
                tx_max_dbm,
                r,
                l,
                c,
                g,
                f,
            } => {
                if tx_min_dbm > tx_max_dbm {
                    bail!("cannot have tx_min_dbm > tx_max_dbm [{tx_min_dbm}, {tx_max_dbm}]");
                }
                Ok(Self::Wired {
                    rx_min_dbm,
                    tx_min_dbm,
                    tx_max_dbm,
                    r,
                    l,
                    c,
                    g,
                    f,
                })
            }
        }
    }
}

impl Charge {
    fn validate(val: parse::Charge) -> Result<Self> {
        let max = val.max.unwrap_or(u64::MAX);
        if val.quantity > max {
            bail!(
                "charge.quantity ({}) exceeds charge.max ({})",
                val.quantity,
                max
            );
        }
        Ok(Self {
            max,
            quantity: val.quantity,
            unit: EnergyUnit::validate(val.unit)
                .context("Failed to validate energy unit in charge.")?,
        })
    }
}

impl Energy {
    fn validate(val: parse::Energy) -> Result<Self> {
        Ok(Self {
            quantity: val.quantity,
            unit: EnergyUnit::validate(val.unit).context("Failed to validate energy unit.")?,
        })
    }
}

impl ChannelEnergy {
    fn validate(val: parse::ChannelEnergy) -> Result<Self> {
        let tx = val.tx.map(Energy::validate).transpose()?;
        let rx = val.rx.map(Energy::validate).transpose()?;
        Ok(Self { tx, rx })
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
    /// Validate an unsigned consumption rate (power_states).
    /// Stored as positive; callers handle the sign at accounting time.
    fn validate_rate(val: parse::PowerRate) -> Result<Self> {
        Ok(PowerRate {
            rate: val.rate,
            unit: PowerUnit::validate(val.unit).context("Failed to validate power unit")?,
            time: TimeUnit::validate(val.time).context("Failed to validate time unit")?,
        })
    }
}

impl PowerFlow {
    fn validate(def: parse::PowerFlowDef) -> Result<Self> {
        match def {
            parse::PowerFlowDef::Constant { rate, unit, time } => {
                let unit = PowerUnit::validate(unit).context("Failed to validate power unit")?;
                let time = TimeUnit::validate(time).context("Failed to validate time unit")?;
                Ok(Self::Constant(PowerRate { rate, unit, time }))
            }
            parse::PowerFlowDef::Scheduled {
                unit,
                time,
                schedule,
                repeat,
            } => {
                ensure!(
                    schedule.len() >= 2,
                    "Piecewise linear schedule must have at least 2 breakpoints"
                );
                let unit = PowerUnit::validate(unit).context("Failed to validate power unit")?;
                let time = TimeUnit::validate(time).context("Failed to validate time unit")?;
                let mut breakpoints = Vec::with_capacity(schedule.len());
                for bp in schedule {
                    let time_us = parse_duration_to_us(&bp.at)
                        .context(format!("Failed to parse breakpoint time \"{}\"", bp.at))?;
                    breakpoints.push((time_us, bp.rate));
                }
                // Validate breakpoints are sorted by time
                for i in 1..breakpoints.len() {
                    ensure!(
                        breakpoints[i].0 >= breakpoints[i - 1].0,
                        "Breakpoints must be in non-decreasing time order; \
                         got {} after {}",
                        breakpoints[i].0,
                        breakpoints[i - 1].0
                    );
                }
                let repeat_us = repeat
                    .map(|s| parse_duration_to_us(&s))
                    .transpose()
                    .context("Failed to parse repeat duration")?;
                if let Some(period) = repeat_us {
                    ensure!(period > 0, "repeat duration must be positive");
                    let last_time = breakpoints.last().unwrap().0;
                    ensure!(
                        last_time <= period,
                        "Last breakpoint time ({last_time} us) exceeds repeat period ({period} us)"
                    );
                }
                Ok(Self::PiecewiseLinear {
                    unit,
                    time,
                    breakpoints,
                    repeat_us,
                })
            }
        }
    }
}

impl Node {
    pub const SELF: &'static str = "self";
    fn validate(
        config_root: &PathBuf,
        val: parse::Node,
        default_start: &SystemTime,
        channel_handles: &HashSet<ChannelHandle>,
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

        // Validate per-node power states
        let power_states = val
            .power_states
            .unwrap_or_default()
            .into_iter()
            .map(|(name, rate)| {
                PowerRate::validate_rate(rate)
                    .context(format!("Failed to validate power state \"{name}\""))
                    .map(|r| (name, r))
            })
            .collect::<Result<HashMap<_, _>>>()?;

        // Validate power sources
        let power_sources = val
            .power_sources
            .unwrap_or_default()
            .into_iter()
            .map(|(name, def)| {
                PowerFlow::validate(def)
                    .context(format!("Failed to validate power source \"{name}\""))
                    .map(|f| (name, f))
            })
            .collect::<Result<HashMap<_, _>>>()?;

        // Validate power sinks
        let power_sinks = val
            .power_sinks
            .unwrap_or_default()
            .into_iter()
            .map(|(name, def)| {
                PowerFlow::validate(def)
                    .context(format!("Failed to validate power sink \"{name}\""))
                    .map(|f| (name, f))
            })
            .collect::<Result<HashMap<_, _>>>()?;

        // Validate per-node channel energy costs
        let channel_energy = val
            .channel_energy
            .unwrap_or_default()
            .into_iter()
            .map(|(ch, energy)| {
                ensure!(
                    valid_channels.contains(&ch),
                    "channel_energy references unknown channel \"{ch}\""
                );
                ChannelEnergy::validate(energy)
                    .context(format!("Failed to validate channel_energy for \"{ch}\""))
                    .map(|e| (ch, e))
            })
            .collect::<Result<HashMap<_, _>>>()?;

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
            initial_state,
            restart_threshold,
            start,
        } in deployments
        {
            let start = start
                .map(toml_datetime_to_system_time)
                .unwrap_or(*default_start);

            // Validate restart_threshold is in [0, 1]
            if let Some(rt) = restart_threshold {
                ensure!(
                    (0.0..=1.0).contains(&rt),
                    "restart_threshold must be between 0.0 and 1.0, got {rt}"
                );
            }

            // Validate initial_state references an existing power state
            if let Some(ref state) = initial_state {
                ensure!(
                    power_states.contains_key(state),
                    "initial_state \"{state}\" is not defined in power_states"
                );
            }

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
            let charge = charge.map(Charge::validate).transpose()?;

            nodes.push(Node {
                charge,
                position,
                resources: resources.clone(),
                internal_names: internal_names.iter().cloned().collect(),
                protocols,
                power_states: power_states.clone(),
                power_sources: power_sources.clone(),
                power_sinks: power_sinks.clone(),
                channel_energy: channel_energy.clone(),
                initial_state,
                restart_threshold,
                start,
            });
        }
        Ok(nodes)
    }
}

impl SignalShape {
    const MATCHES: &[(&str, SignalShape)] = &[
        ("omni", Self::Omnidirectional),
        ("omnidirectional", Self::Omnidirectional),
        ("cone", Self::Cone),
        ("direct", Self::Direct),
    ];
    fn validate(mut val: parse::SignalShape) -> Result<Self> {
        val.0.make_ascii_lowercase();
        if let Some((_, res)) = Self::MATCHES.iter().find(|(name, _)| *name == val.0) {
            Ok(*res)
        } else {
            let options = Self::MATCHES
                .iter()
                .map(|(name, _)| format!("\"{name}\""))
                .collect::<Vec<_>>()
                .join(" | ");
            bail!("Expected to find ({options}) but found {s}", s = val.0);
        }
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

fn toml_datetime_to_chrono(dt: toml::value::Datetime) -> DateTime<Utc> {
    let s = dt.to_string();
    let chrono_dt: chrono::DateTime<Utc> = s.parse().expect("invalid date format in toml file");
    chrono_dt
}

fn toml_datetime_to_system_time(dt: toml::value::Datetime) -> SystemTime {
    SystemTime::from(toml_datetime_to_chrono(dt))
}
