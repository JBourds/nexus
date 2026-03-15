use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Command, Stdio};

use super::display;
use super::format::{TraceEvent, TraceHeader, TraceRecord};
use super::reader::TraceReader;
use anyhow::{Result, bail};
use runner::cli::{EventFilter, ParseOutput};

/// Resolved filter that maps user-provided names to indices for fast matching.
pub struct ResolvedFilter {
    event_filters: Option<Vec<EventFilter>>,
    node_indices: Option<Vec<u32>>,
    channel_indices: Option<Vec<u32>>,
    from_ts: Option<u64>,
    to_ts: Option<u64>,
}

impl ResolvedFilter {
    pub fn new(
        header: &TraceHeader,
        events: Option<Vec<EventFilter>>,
        nodes: Option<Vec<String>>,
        channels: Option<Vec<String>>,
        from: Option<u64>,
        to: Option<u64>,
    ) -> Result<Self> {
        let node_indices = if let Some(ref names) = nodes {
            let mut indices = Vec::with_capacity(names.len());
            for name in names {
                // Exact match first, then try prefix match (e.g. "sensor" matches "sensor.0", "sensor.1")
                let exact = header.node_names.iter().position(|n| n == name);
                if let Some(i) = exact {
                    indices.push(i as u32);
                } else {
                    // Match nodes whose base name (everything before the last '.')
                    // equals the filter name. This handles deployment expansion
                    // where "sensor" becomes "sensor.0", "sensor.1", and also
                    // "my.node" becomes "my.node.0", "my.node.1".
                    let prefix_matches: Vec<u32> = header
                        .node_names
                        .iter()
                        .enumerate()
                        .filter(|(_, n)| {
                            n.rsplit_once('.')
                                .is_some_and(|(base, _suffix)| base == name)
                        })
                        .map(|(i, _)| i as u32)
                        .collect();
                    if prefix_matches.is_empty() {
                        bail!("unknown node name: {name:?}");
                    }
                    indices.extend(prefix_matches);
                }
            }
            Some(indices)
        } else {
            None
        };

        let channel_indices = if let Some(ref names) = channels {
            let mut indices = Vec::with_capacity(names.len());
            for name in names {
                let pos = header.channel_names.iter().position(|n| n == name);
                match pos {
                    Some(i) => indices.push(i as u32),
                    None => bail!("unknown channel name: {name:?}"),
                }
            }
            Some(indices)
        } else {
            None
        };

        Ok(Self {
            event_filters: events,
            node_indices,
            channel_indices,
            from_ts: from,
            to_ts: to,
        })
    }

    /// Returns true if the record passes all active filters.
    pub fn matches(&self, record: &TraceRecord) -> bool {
        // Timestep range
        if let Some(from) = self.from_ts
            && record.timestep < from
        {
            return false;
        }
        if let Some(to) = self.to_ts
            && record.timestep > to
        {
            return false;
        }

        // Event type filter
        if let Some(ref filters) = self.event_filters {
            let matched = match &record.event {
                TraceEvent::MessageSent { .. } => filters.contains(&EventFilter::Tx),
                TraceEvent::MessageRecv { .. } => filters.contains(&EventFilter::Rx),
                TraceEvent::MessageDropped { .. } => filters.contains(&EventFilter::Drop),
                TraceEvent::PositionUpdate { .. } => filters.contains(&EventFilter::Position),
                TraceEvent::EnergyUpdate { .. } => filters.contains(&EventFilter::Energy),
                TraceEvent::MotionUpdate { .. } => filters.contains(&EventFilter::Motion),
            };
            if !matched {
                return false;
            }
        }

        // Node filter
        if let Some(ref indices) = self.node_indices {
            let node_id = event_node_id(&record.event);
            if !indices.contains(&node_id) {
                return false;
            }
        }

        // Channel filter
        if let Some(ref indices) = self.channel_indices {
            let ch = event_channel_id(&record.event);
            // Events without a channel (position, energy, motion) pass channel filter
            if let Some(id) = ch
                && !indices.contains(&id)
            {
                return false;
            }
        }

        true
    }
}

fn event_node_id(event: &TraceEvent) -> u32 {
    match event {
        TraceEvent::MessageSent { src_node, .. } => *src_node,
        TraceEvent::MessageRecv { dst_node, .. } => *dst_node,
        TraceEvent::MessageDropped { src_node, .. } => *src_node,
        TraceEvent::PositionUpdate { node, .. } => *node,
        TraceEvent::EnergyUpdate { node, .. } => *node,
        TraceEvent::MotionUpdate { node, .. } => *node,
    }
}

fn event_channel_id(event: &TraceEvent) -> Option<u32> {
    match event {
        TraceEvent::MessageSent { channel, .. } => Some(*channel),
        TraceEvent::MessageRecv { channel, .. } => Some(*channel),
        TraceEvent::MessageDropped { channel, .. } => Some(*channel),
        TraceEvent::PositionUpdate { .. } => None,
        TraceEvent::EnergyUpdate { .. } => None,
        TraceEvent::MotionUpdate { .. } => None,
    }
}

/// Main entry point for the `nexus parse` subcommand.
#[allow(clippy::too_many_arguments)]
pub fn run_parse(
    trace_path: &Path,
    events: Option<Vec<EventFilter>>,
    nodes: Option<Vec<String>>,
    channels: Option<Vec<String>>,
    from: Option<u64>,
    to: Option<u64>,
    output: ParseOutput,
    adapter: Option<String>,
    header_only: bool,
) -> Result<()> {
    let mut reader = TraceReader::open(trace_path)?;
    let header = &reader.header.clone();
    let path_str = trace_path.display().to_string();

    // Always print the header summary
    let summary = display::format_header_summary(header, &path_str);
    println!("{summary}");

    if header_only {
        return Ok(());
    }

    println!("---");

    let filter = ResolvedFilter::new(header, events, nodes, channels, from, to)?;

    // Seek to start timestep if possible
    if let Some(from_ts) = filter.from_ts {
        let _ = reader.seek_to_timestep(from_ts);
    }

    // Optional adapter process
    let mut adapter_proc = if let Some(ref cmd) = adapter {
        Some(spawn_adapter(cmd)?)
    } else {
        None
    };

    let mut json_records: Vec<serde_json::Value> = Vec::new();

    while let Some(record) = reader.next_record()? {
        // Early exit if past the `to` timestep
        if let Some(to_ts) = filter.to_ts
            && record.timestep > to_ts
        {
            break;
        }

        if !filter.matches(&record) {
            continue;
        }

        match output {
            ParseOutput::Text => {
                let line = display::format_record(header, &record);
                let line = maybe_decode_adapter(&mut adapter_proc, header, &record, line)?;
                println!("{line}");
            }
            ParseOutput::Json => {
                json_records.push(display::record_to_json(header, &record));
            }
            ParseOutput::JsonLines => {
                let val = display::record_to_json(header, &record);
                println!("{}", serde_json::to_string(&val)?);
            }
        }
    }

    if let ParseOutput::Json = output {
        println!("{}", serde_json::to_string_pretty(&json_records)?);
    }

    // Clean up adapter process
    if let Some(ref mut proc) = adapter_proc {
        drop(proc.stdin.take());
        proc.wait()?;
    }

    Ok(())
}

struct AdapterProcess {
    stdin: Option<std::process::ChildStdin>,
    stdout: BufReader<std::process::ChildStdout>,
    child: std::process::Child,
}

impl AdapterProcess {
    fn wait(&mut self) -> Result<()> {
        self.child.wait()?;
        Ok(())
    }
}

fn spawn_adapter(cmd: &str) -> Result<AdapterProcess> {
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;

    let stdin = child.stdin.take();
    let stdout = BufReader::new(
        child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("failed to open adapter stdout"))?,
    );

    Ok(AdapterProcess {
        stdin,
        stdout,
        child,
    })
}

/// For message events, pipe the JSON representation to the adapter and read back
/// decoded text. For non-message events, return the line unchanged.
fn maybe_decode_adapter(
    adapter: &mut Option<AdapterProcess>,
    header: &TraceHeader,
    record: &TraceRecord,
    fallback: String,
) -> Result<String> {
    let proc = match adapter {
        Some(p) => p,
        None => return Ok(fallback),
    };

    // Only send message events to the adapter
    let has_data = matches!(
        &record.event,
        TraceEvent::MessageSent { .. } | TraceEvent::MessageRecv { .. }
    );
    if !has_data {
        return Ok(fallback);
    }

    let json_val = display::record_to_json(header, record);
    let json_line = serde_json::to_string(&json_val)?;

    if let Some(ref mut stdin) = proc.stdin {
        writeln!(stdin, "{json_line}")?;
        stdin.flush()?;
    }

    let mut decoded = String::new();
    proc.stdout.read_line(&mut decoded)?;
    let decoded = decoded.trim_end();

    if decoded.is_empty() {
        Ok(fallback)
    } else {
        Ok(format!("{fallback}  [{decoded}]"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::{DropReason, TraceEvent, TraceHeader, TraceRecord};

    fn test_header() -> TraceHeader {
        TraceHeader {
            node_names: vec!["alice".into(), "bob".into()],
            channel_names: vec!["lora0".into()],
            timestep_count: 100,
            node_max_nj: vec![None, None],
        }
    }

    #[test]
    fn test_filter_matches_all() {
        let h = test_header();
        let filter = ResolvedFilter::new(&h, None, None, None, None, None).unwrap();
        let rec = TraceRecord {
            timestep: 5,
            event: TraceEvent::MessageSent {
                src_node: 0,
                channel: 0,
                data: vec![1],
                msg_id: 1,
            },
        };
        assert!(filter.matches(&rec));
    }

    #[test]
    fn test_filter_event_type() {
        let h = test_header();
        let filter =
            ResolvedFilter::new(&h, Some(vec![EventFilter::Rx]), None, None, None, None).unwrap();
        let tx = TraceRecord {
            timestep: 1,
            event: TraceEvent::MessageSent {
                src_node: 0,
                channel: 0,
                data: vec![],
                msg_id: 1,
            },
        };
        let rx = TraceRecord {
            timestep: 1,
            event: TraceEvent::MessageRecv {
                dst_node: 1,
                channel: 0,
                data: vec![],
                bit_errors: false,
                msg_id: 1,
            },
        };
        assert!(!filter.matches(&tx));
        assert!(filter.matches(&rx));
    }

    #[test]
    fn test_filter_node_name() {
        let h = test_header();
        let filter =
            ResolvedFilter::new(&h, None, Some(vec!["bob".into()]), None, None, None).unwrap();
        let alice_rec = TraceRecord {
            timestep: 1,
            event: TraceEvent::EnergyUpdate {
                node: 0,
                energy_nj: 100,
            },
        };
        let bob_rec = TraceRecord {
            timestep: 1,
            event: TraceEvent::EnergyUpdate {
                node: 1,
                energy_nj: 100,
            },
        };
        assert!(!filter.matches(&alice_rec));
        assert!(filter.matches(&bob_rec));
    }

    #[test]
    fn test_filter_unknown_node_errors() {
        let h = test_header();
        let result = ResolvedFilter::new(&h, None, Some(vec!["unknown".into()]), None, None, None);
        assert!(result.is_err());
    }

    #[test]
    fn test_filter_unknown_channel_errors() {
        let h = test_header();
        let result =
            ResolvedFilter::new(&h, None, None, Some(vec!["nonexistent".into()]), None, None);
        assert!(result.is_err());
    }

    #[test]
    fn test_filter_timestep_range() {
        let h = test_header();
        let filter = ResolvedFilter::new(&h, None, None, None, Some(5), Some(10)).unwrap();
        let before = TraceRecord {
            timestep: 3,
            event: TraceEvent::PositionUpdate {
                node: 0,
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
        };
        let inside = TraceRecord {
            timestep: 7,
            event: TraceEvent::PositionUpdate {
                node: 0,
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
        };
        let after = TraceRecord {
            timestep: 15,
            event: TraceEvent::PositionUpdate {
                node: 0,
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
        };
        assert!(!filter.matches(&before));
        assert!(filter.matches(&inside));
        assert!(!filter.matches(&after));
    }

    #[test]
    fn test_filter_channel() {
        let h = test_header();
        let filter =
            ResolvedFilter::new(&h, None, None, Some(vec!["lora0".into()]), None, None).unwrap();
        // Message with matching channel passes
        let rec = TraceRecord {
            timestep: 1,
            event: TraceEvent::MessageSent {
                src_node: 0,
                channel: 0,
                data: vec![],
                msg_id: 1,
            },
        };
        assert!(filter.matches(&rec));

        // Position update has no channel, so it passes the channel filter
        let pos = TraceRecord {
            timestep: 1,
            event: TraceEvent::PositionUpdate {
                node: 0,
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
        };
        assert!(filter.matches(&pos));
    }

    #[test]
    fn test_filter_drop_event() {
        let h = test_header();
        let filter =
            ResolvedFilter::new(&h, Some(vec![EventFilter::Drop]), None, None, None, None).unwrap();
        let drop_rec = TraceRecord {
            timestep: 1,
            event: TraceEvent::MessageDropped {
                src_node: 0,
                channel: 0,
                reason: DropReason::PacketLoss,
                msg_id: 1,
            },
        };
        assert!(filter.matches(&drop_rec));
    }

    #[test]
    fn test_filter_node_prefix_match() {
        // Simulates expanded node names like "sensor.0", "sensor.1"
        let h = TraceHeader {
            node_names: vec!["sensor.0".into(), "sensor.1".into(), "gateway.0".into()],
            channel_names: vec!["lora0".into()],
            timestep_count: 100,
            node_max_nj: vec![None, None, None],
        };
        // "sensor" should match both sensor.0 (idx 0) and sensor.1 (idx 1)
        let filter =
            ResolvedFilter::new(&h, None, Some(vec!["sensor".into()]), None, None, None).unwrap();

        let sensor0 = TraceRecord {
            timestep: 1,
            event: TraceEvent::EnergyUpdate {
                node: 0,
                energy_nj: 100,
            },
        };
        let sensor1 = TraceRecord {
            timestep: 1,
            event: TraceEvent::EnergyUpdate {
                node: 1,
                energy_nj: 100,
            },
        };
        let gateway = TraceRecord {
            timestep: 1,
            event: TraceEvent::EnergyUpdate {
                node: 2,
                energy_nj: 100,
            },
        };
        assert!(filter.matches(&sensor0));
        assert!(filter.matches(&sensor1));
        assert!(!filter.matches(&gateway));
    }

    #[test]
    fn test_filter_node_exact_over_prefix() {
        // If "alice" exists as an exact name, don't also match "alice.0"
        let h = TraceHeader {
            node_names: vec!["alice".into(), "alice.0".into()],
            channel_names: vec![],
            timestep_count: 100,
            node_max_nj: vec![None, None],
        };
        let filter =
            ResolvedFilter::new(&h, None, Some(vec!["alice".into()]), None, None, None).unwrap();

        let alice_exact = TraceRecord {
            timestep: 1,
            event: TraceEvent::EnergyUpdate {
                node: 0,
                energy_nj: 100,
            },
        };
        let alice_dot0 = TraceRecord {
            timestep: 1,
            event: TraceEvent::EnergyUpdate {
                node: 1,
                energy_nj: 100,
            },
        };
        assert!(filter.matches(&alice_exact));
        assert!(!filter.matches(&alice_dot0));
    }

    #[test]
    fn test_filter_node_dotted_base_name() {
        // A node named "my.node" in config becomes "my.node.0", "my.node.1"
        // "--nodes my.node" should match both by splitting on the last '.'
        let h = TraceHeader {
            node_names: vec!["my.node.0".into(), "my.node.1".into(), "other.0".into()],
            channel_names: vec![],
            timestep_count: 100,
            node_max_nj: vec![None, None, None],
        };
        let filter =
            ResolvedFilter::new(&h, None, Some(vec!["my.node".into()]), None, None, None).unwrap();

        let node0 = TraceRecord {
            timestep: 1,
            event: TraceEvent::EnergyUpdate {
                node: 0,
                energy_nj: 100,
            },
        };
        let node1 = TraceRecord {
            timestep: 1,
            event: TraceEvent::EnergyUpdate {
                node: 1,
                energy_nj: 100,
            },
        };
        let other = TraceRecord {
            timestep: 1,
            event: TraceEvent::EnergyUpdate {
                node: 2,
                energy_nj: 100,
            },
        };
        assert!(filter.matches(&node0));
        assert!(filter.matches(&node1));
        assert!(!filter.matches(&other));
    }
}
