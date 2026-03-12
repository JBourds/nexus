//! timectl.rs
//! Functionality for time-based control files.

use std::time::{Duration, SystemTime};

use config::ast::TimeUnit;

use crate::router::{RouterError, RoutingServer};

impl RoutingServer {
    pub fn update_time(
        &mut self,
        node_index: usize,
        msg: fuse::Message,
    ) -> Result<(), RouterError> {
        let unit = Self::suffix_to_time(msg.id.1.as_str())
            .ok_or_else(|| RouterError::UnknownFile(msg.id.1.clone()))?;
        let s = String::from_utf8_lossy(&msg.data);
        let val: u64 = s
            .parse()
            .map_err(|_| RouterError::InvalidString(msg.data.clone()))?;
        let to_units = |val| match unit {
            TimeUnit::Seconds => Duration::from_secs(val),
            TimeUnit::Milliseconds => Duration::from_millis(val),
            TimeUnit::Microseconds => Duration::from_micros(val),
            TimeUnit::Nanoseconds => Duration::from_nanos(val),
            _ => unreachable!(),
        };
        // Updating time requires updating the node's "start time" based on the
        // specified time and time elapsed
        let time_from_epoch = to_units(val);
        let node_start = &self.channels.nodes[node_index].start;
        let elapsed = to_units(self.ts_config.time_from(self.timestep, unit, node_start));
        self.channels.nodes[node_index].start = SystemTime::UNIX_EPOCH
            .checked_add(time_from_epoch - elapsed)
            .ok_or(RouterError::InvalidString(msg.data))?;
        Ok(())
    }

    pub fn send_time(
        &mut self,
        node_index: usize,
        mut msg: fuse::Message,
    ) -> Result<(), RouterError> {
        let node_start = &self.channels.nodes[node_index].start;
        let unit = Self::suffix_to_time(msg.id.1.as_str())
            .ok_or_else(|| RouterError::UnknownFile(msg.id.1.clone()))?;
        let s = self
            .ts_config
            .time_from(self.timestep, unit, node_start)
            .to_string();
        msg.data = s.bytes().collect();
        self.tx
            .send(fuse::KernelMessage::Exclusive(msg))
            .map_err(RouterError::FuseSendError)
    }

    pub fn send_elapsed(&mut self, mut msg: fuse::Message) -> Result<(), RouterError> {
        let unit = Self::suffix_to_time(msg.id.1.as_str())
            .ok_or_else(|| RouterError::UnknownFile(msg.id.1.clone()))?;
        let s = self.ts_config.elapsed(self.timestep, unit).to_string();
        msg.data = s.bytes().collect();
        self.tx
            .send(fuse::KernelMessage::Exclusive(msg))
            .map_err(RouterError::FuseSendError)
    }

    fn suffix_to_time(s: &str) -> Option<TimeUnit> {
        match &s[s.len() - 2..s.len()] {
            "us" => Some(TimeUnit::Microseconds),
            "ms" => Some(TimeUnit::Milliseconds),
            ".s" => Some(TimeUnit::Seconds),
            _ => None,
        }
    }
}
