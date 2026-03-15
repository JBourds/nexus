use serde_json::{Value, json};

use crate::format::{DropReason, TraceEvent, TraceHeader, TraceRecord};

/// Look up a node name by index, falling back to "node_N".
pub fn node_name(header: &TraceHeader, idx: u32) -> String {
    header
        .node_names
        .get(idx as usize)
        .cloned()
        .unwrap_or_else(|| format!("node_{idx}"))
}

/// Look up a channel name by index, falling back to "ch_N".
pub fn channel_name(header: &TraceHeader, idx: u32) -> String {
    header
        .channel_names
        .get(idx as usize)
        .cloned()
        .unwrap_or_else(|| format!("ch_{idx}"))
}

/// Format the timestep field width based on total timestep count.
fn ts_width(header: &TraceHeader) -> usize {
    let digits = if 0 == header.timestep_count {
        1
    } else {
        (header.timestep_count as f64).log10().floor() as usize + 1
    };
    digits.max(1)
}

/// Format a single trace record as a human-readable line.
pub fn format_record(header: &TraceHeader, record: &TraceRecord) -> String {
    let w = ts_width(header);
    let ts = record.timestep;
    match &record.event {
        TraceEvent::MessageSent {
            src_node,
            channel,
            data,
            msg_id,
        } => {
            let node = node_name(header, *src_node);
            let ch = channel_name(header, *channel);
            let hex: String = data.iter().map(|b| format!("{b:02x}")).collect();
            format!(
                "[t={ts:0>w$}] TX   {node} -> {ch}   ({len} bytes) msg={msg_id} {hex}",
                len = data.len()
            )
        }
        TraceEvent::MessageRecv {
            dst_node,
            channel,
            data,
            bit_errors,
            msg_id,
        } => {
            let node = node_name(header, *dst_node);
            let ch = channel_name(header, *channel);
            let hex: String = data.iter().map(|b| format!("{b:02x}")).collect();
            let err_flag = if *bit_errors { " [bit_errors]" } else { "" };
            format!(
                "[t={ts:0>w$}] RX   {node} <- {ch}   ({len} bytes) msg={msg_id}{err_flag} {hex}",
                len = data.len()
            )
        }
        TraceEvent::MessageDropped {
            src_node,
            channel,
            reason,
            msg_id,
        } => {
            let node = node_name(header, *src_node);
            let ch = channel_name(header, *channel);
            let reason_str = match reason {
                DropReason::BelowSensitivity => "BelowSensitivity",
                DropReason::PacketLoss => "PacketLoss",
                DropReason::TtlExpired => "TtlExpired",
                DropReason::BufferFull => "BufferFull",
            };
            format!("[t={ts:0>w$}] DROP {node} -> {ch}   msg={msg_id} reason={reason_str}")
        }
        TraceEvent::PositionUpdate { node, x, y, z } => {
            let name = node_name(header, *node);
            format!("[t={ts:0>w$}] POS  {name}  ({x:.2}, {y:.2}, {z:.2})")
        }
        TraceEvent::EnergyUpdate { node, energy_nj } => {
            let name = node_name(header, *node);
            format!("[t={ts:0>w$}] NRG  {name}  {energy_nj} nJ")
        }
        TraceEvent::MotionUpdate { node, spec } => {
            let name = node_name(header, *node);
            format!("[t={ts:0>w$}] MOT  {name}  {spec}")
        }
    }
}

/// Convert a trace record to a structured JSON value.
pub fn record_to_json(header: &TraceHeader, record: &TraceRecord) -> Value {
    let ts = record.timestep;
    match &record.event {
        TraceEvent::MessageSent {
            src_node,
            channel,
            data,
            msg_id,
        } => json!({
            "timestep": ts,
            "event": "MessageSent",
            "node": node_name(header, *src_node),
            "channel": channel_name(header, *channel),
            "data_hex": data.iter().map(|b| format!("{b:02x}")).collect::<String>(),
            "data_len": data.len(),
            "msg_id": msg_id,
        }),
        TraceEvent::MessageRecv {
            dst_node,
            channel,
            data,
            bit_errors,
            msg_id,
        } => json!({
            "timestep": ts,
            "event": "MessageRecv",
            "node": node_name(header, *dst_node),
            "channel": channel_name(header, *channel),
            "data_hex": data.iter().map(|b| format!("{b:02x}")).collect::<String>(),
            "data_len": data.len(),
            "bit_errors": bit_errors,
            "msg_id": msg_id,
        }),
        TraceEvent::MessageDropped {
            src_node,
            channel,
            reason,
            msg_id,
        } => json!({
            "timestep": ts,
            "event": "MessageDropped",
            "node": node_name(header, *src_node),
            "channel": channel_name(header, *channel),
            "reason": format!("{reason:?}"),
            "msg_id": msg_id,
        }),
        TraceEvent::PositionUpdate { node, x, y, z } => json!({
            "timestep": ts,
            "event": "PositionUpdate",
            "node": node_name(header, *node),
            "x": x,
            "y": y,
            "z": z,
        }),
        TraceEvent::EnergyUpdate { node, energy_nj } => json!({
            "timestep": ts,
            "event": "EnergyUpdate",
            "node": node_name(header, *node),
            "energy_nj": energy_nj,
        }),
        TraceEvent::MotionUpdate { node, spec } => json!({
            "timestep": ts,
            "event": "MotionUpdate",
            "node": node_name(header, *node),
            "spec": spec,
        }),
    }
}

/// Format a summary of the trace header for display.
pub fn format_header_summary(header: &TraceHeader, path: &str) -> String {
    let mut lines = Vec::new();
    lines.push(format!("=== Trace: {path} ==="));
    lines.push(format!(
        "Nodes ({}): {}",
        header.node_names.len(),
        header.node_names.join(", ")
    ));
    lines.push(format!(
        "Channels ({}): {}",
        header.channel_names.len(),
        header.channel_names.join(", ")
    ));
    lines.push(format!("Timesteps: {}", header.timestep_count));

    // Energy info
    let energy_parts: Vec<String> = header
        .node_names
        .iter()
        .zip(header.node_max_nj.iter())
        .map(|(name, max)| match max {
            Some(nj) => format!("{name} (max {nj} nJ)"),
            None => format!("{name} (none)"),
        })
        .collect();
    if !energy_parts.is_empty() {
        lines.push(format!("Energy: {}", energy_parts.join(", ")));
    }

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_header() -> TraceHeader {
        TraceHeader {
            node_names: vec!["alice".into(), "bob".into(), "carol".into()],
            channel_names: vec!["lora0".into(), "wired1".into()],
            timestep_count: 1000,
            node_max_nj: vec![Some(5_000_000), Some(5_000_000), None],
        }
    }

    #[test]
    fn test_node_name_valid() {
        let h = test_header();
        assert_eq!("alice", node_name(&h, 0));
        assert_eq!("carol", node_name(&h, 2));
    }

    #[test]
    fn test_node_name_fallback() {
        let h = test_header();
        assert_eq!("node_99", node_name(&h, 99));
    }

    #[test]
    fn test_channel_name_valid() {
        let h = test_header();
        assert_eq!("lora0", channel_name(&h, 0));
    }

    #[test]
    fn test_channel_name_fallback() {
        let h = test_header();
        assert_eq!("ch_5", channel_name(&h, 5));
    }

    #[test]
    fn test_format_record_tx() {
        let h = test_header();
        let rec = TraceRecord {
            timestep: 1,
            event: TraceEvent::MessageSent {
                src_node: 0,
                channel: 0,
                data: vec![0x48, 0x65, 0x6c],
                msg_id: 1,
            },
        };
        let out = format_record(&h, &rec);
        assert!(out.contains("TX"));
        assert!(out.contains("alice"));
        assert!(out.contains("lora0"));
        assert!(out.contains("48656c"));
        assert!(out.contains("3 bytes"));
    }

    #[test]
    fn test_format_record_rx() {
        let h = test_header();
        let rec = TraceRecord {
            timestep: 1,
            event: TraceEvent::MessageRecv {
                dst_node: 1,
                channel: 0,
                data: vec![0xab],
                bit_errors: false,
                msg_id: 2,
            },
        };
        let out = format_record(&h, &rec);
        assert!(out.contains("RX"));
        assert!(out.contains("bob"));
        assert!(out.contains("<-"));
    }

    #[test]
    fn test_format_record_drop() {
        let h = test_header();
        let rec = TraceRecord {
            timestep: 1,
            event: TraceEvent::MessageDropped {
                src_node: 2,
                channel: 0,
                reason: DropReason::BelowSensitivity,
                msg_id: 3,
            },
        };
        let out = format_record(&h, &rec);
        assert!(out.contains("DROP"));
        assert!(out.contains("carol"));
        assert!(out.contains("BelowSensitivity"));
    }

    #[test]
    fn test_format_record_position() {
        let h = test_header();
        let rec = TraceRecord {
            timestep: 10,
            event: TraceEvent::PositionUpdate {
                node: 0,
                x: 1.5,
                y: 2.3,
                z: 0.0,
            },
        };
        let out = format_record(&h, &rec);
        assert!(out.contains("POS"));
        assert!(out.contains("alice"));
        assert!(out.contains("1.50"));
        assert!(out.contains("2.30"));
    }

    #[test]
    fn test_format_record_energy() {
        let h = test_header();
        let rec = TraceRecord {
            timestep: 10,
            event: TraceEvent::EnergyUpdate {
                node: 0,
                energy_nj: 4_500_000,
            },
        };
        let out = format_record(&h, &rec);
        assert!(out.contains("NRG"));
        assert!(out.contains("4500000 nJ"));
    }

    #[test]
    fn test_format_record_motion() {
        let h = test_header();
        let rec = TraceRecord {
            timestep: 50,
            event: TraceEvent::MotionUpdate {
                node: 1,
                spec: "velocity 1.0 0.0 0.0".into(),
            },
        };
        let out = format_record(&h, &rec);
        assert!(out.contains("MOT"));
        assert!(out.contains("bob"));
        assert!(out.contains("velocity 1.0 0.0 0.0"));
    }

    #[test]
    fn test_record_to_json_tx() {
        let h = test_header();
        let rec = TraceRecord {
            timestep: 5,
            event: TraceEvent::MessageSent {
                src_node: 0,
                channel: 1,
                data: vec![0xff],
                msg_id: 10,
            },
        };
        let val = record_to_json(&h, &rec);
        assert_eq!(5, val["timestep"]);
        assert_eq!("MessageSent", val["event"]);
        assert_eq!("alice", val["node"]);
        assert_eq!("wired1", val["channel"]);
        assert_eq!("ff", val["data_hex"]);
        assert_eq!(1, val["data_len"]);
    }

    #[test]
    fn test_record_to_json_drop() {
        let h = test_header();
        let rec = TraceRecord {
            timestep: 3,
            event: TraceEvent::MessageDropped {
                src_node: 1,
                channel: 0,
                reason: DropReason::BufferFull,
                msg_id: 11,
            },
        };
        let val = record_to_json(&h, &rec);
        assert_eq!("MessageDropped", val["event"]);
        assert_eq!("BufferFull", val["reason"]);
    }

    #[test]
    fn test_header_summary() {
        let h = test_header();
        let summary = format_header_summary(&h, "trace.nxs");
        assert!(summary.contains("=== Trace: trace.nxs ==="));
        assert!(summary.contains("Nodes (3): alice, bob, carol"));
        assert!(summary.contains("Channels (2): lora0, wired1"));
        assert!(summary.contains("Timesteps: 1000"));
        assert!(summary.contains("alice (max 5000000 nJ)"));
        assert!(summary.contains("carol (none)"));
    }

    #[test]
    fn test_ts_width() {
        let mut h = test_header();
        assert_eq!(4, ts_width(&h)); // 1000 => 4 digits
        h.timestep_count = 1;
        assert_eq!(1, ts_width(&h));
        h.timestep_count = 0;
        assert_eq!(1, ts_width(&h));
        h.timestep_count = 100_000;
        assert_eq!(6, ts_width(&h));
    }
}
