//! powerctl.rs
//! Handlers for `ctl.power_flows` control file (read/write).
//!
//! **Read** returns current flows as text, one per line:
//! ```text
//! source solar 350 mw/s
//! sink mcu 80 mw/s
//! ```
//!
//! **Write** accepts commands to add/modify/remove flows:
//! ```text
//! source battery_charger 400 mw/s
//! sink radio 120 mw/s
//! remove mcu
//! ```
//! Dynamic flows created via the control file are always `Constant`.

use crate::router::{RouterError, RoutingServer};
use crate::types::PowerFlowState;

impl RoutingServer {
    /// Read handler: format all power sources and sinks as text.
    pub fn read_power_flows(
        &mut self,
        node_index: usize,
        mut msg: fuse::Message,
    ) -> Result<(), RouterError> {
        let mut output = String::new();
        if let Some(energy) = &self.channels.nodes[node_index].energy {
            for (name, flow) in &energy.power_sources {
                let nj = flow.nj_per_timestep(0);
                output.push_str(&format!("source {name} {nj} nj/ts\n"));
            }
            for (name, flow) in &energy.power_sinks {
                let nj = flow.nj_per_timestep(0);
                output.push_str(&format!("sink {name} {nj} nj/ts\n"));
            }
        }
        msg.data = output.into_bytes();
        self.tx
            .send(fuse::KernelMessage::Exclusive(msg))
            .map_err(RouterError::FuseSendError)
    }

    /// Write handler: parse commands to add/modify/remove flows.
    pub fn write_power_flows(
        &mut self,
        node_index: usize,
        msg: fuse::Message,
    ) -> Result<(), RouterError> {
        let text = String::from_utf8_lossy(&msg.data);
        let timestep_ns = self.ts_config.length.get() * self.ts_config.unit.to_ns_factor();

        let Some(energy) = &mut self.channels.nodes[node_index].energy else {
            return Ok(());
        };

        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let parts: Vec<&str> = line.split_whitespace().collect();
            match parts.as_slice() {
                ["remove", name] => {
                    let name = name.to_string();
                    energy.power_sources.retain(|(n, _)| *n != name);
                    energy.power_sinks.retain(|(n, _)| *n != name);
                }
                ["source", name, rate_str, unit_time] => {
                    if let Some(nj_per_ts) = Self::parse_flow_rate(rate_str, unit_time, timestep_ns)
                    {
                        let name = name.to_string();
                        energy.power_sources.retain(|(n, _)| *n != name);
                        energy
                            .power_sources
                            .push((name, PowerFlowState::Constant { nj_per_ts }));
                    }
                }
                ["sink", name, rate_str, unit_time] => {
                    if let Some(nj_per_ts) = Self::parse_flow_rate(rate_str, unit_time, timestep_ns)
                    {
                        let name = name.to_string();
                        energy.power_sinks.retain(|(n, _)| *n != name);
                        energy
                            .power_sinks
                            .push((name, PowerFlowState::Constant { nj_per_ts }));
                    }
                }
                _ => { /* silently ignore malformed lines */ }
            }
        }
        Ok(())
    }

    /// Parse a rate+unit string pair like ("400", "mw/s") into nJ per timestep.
    fn parse_flow_rate(rate_str: &str, unit_time: &str, timestep_ns: u64) -> Option<u64> {
        let rate: u64 = rate_str.parse().ok()?;

        // Special case: nj/ts is a direct passthrough
        if unit_time == "nj/ts" {
            return Some(rate);
        }

        let (unit_str, time_str) = unit_time.split_once('/')?;
        let nw_factor = match unit_str {
            "nw" => 1u64,
            "uw" => 1_000,
            "mw" => 1_000_000,
            "w" => 1_000_000_000,
            "kw" => 1_000_000_000_000,
            _ => return None,
        };
        let time_ns = match time_str {
            "h" => 3_600_000_000_000u64,
            "m" => 60_000_000_000,
            "s" => 1_000_000_000,
            "ms" => 1_000_000,
            "us" => 1_000,
            "ns" => 1,
            _ => return None,
        };
        Some(rate * nw_factor * timestep_ns / time_ns)
    }
}
