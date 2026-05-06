//! timectl.rs
//! Functionality for time-based control files.

use std::cmp::Ordering;
use std::time::{Duration, SystemTime};

use config::ast::TimeUnit;
use fuser::ReplyWrite;

use crate::router::{RouterError, RoutingServer};

#[derive(Debug)]
pub(crate) struct SleepAlarm {
    pub timestep: u64,
    pub pid: fuse::PID,
    pub bytes_consumed: u32,
    pub reply: ReplyWrite,
}

impl PartialEq for SleepAlarm {
    fn eq(&self, other: &Self) -> bool {
        self.timestep == other.timestep && self.pid == other.pid
    }
}
impl Eq for SleepAlarm {}

impl Ord for SleepAlarm {
    fn cmp(&self, other: &Self) -> Ordering {
        self.timestep
            .cmp(&other.timestep)
            .then_with(|| self.pid.cmp(&other.pid))
    }
}

impl PartialOrd for SleepAlarm {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl RoutingServer {
    pub fn update_time(
        &mut self,
        node_index: usize,
        msg: fuse::Message,
    ) -> Result<(), RouterError> {
        let (val, unit) = Self::msg_to_time_units(&msg)?;
        let to_units = |val| match unit {
            TimeUnit::Seconds => Duration::from_secs(val),
            TimeUnit::Milliseconds => Duration::from_millis(val),
            TimeUnit::Microseconds => Duration::from_micros(val),
            TimeUnit::Nanoseconds => Duration::from_nanos(val),
            _ => unreachable!(),
        };
        // Updating time requires updating the node's "start time" based on the
        // specified time and time elapsed
        let new_time = to_units(val);
        let elapsed = to_units(self.ts_config.elapsed(self.timestep, unit));
        let new_start = new_time.saturating_sub(elapsed);
        self.channels.nodes[node_index].start = SystemTime::UNIX_EPOCH
            .checked_add(new_start)
            .ok_or(RouterError::InvalidString(msg.data))?;
        Ok(())
    }

    pub fn send_time(
        &mut self,
        node_index: usize,
        req: fuse::ReadRequest,
    ) -> Result<(), RouterError> {
        let node_start = &self.channels.nodes[node_index].start;
        let Some(unit) = Self::suffix_to_time(req.id.1.as_str()) else {
            let path = req.id.1.clone();
            drop(req.reply);
            return Err(RouterError::UnknownFile(path));
        };
        let s = self
            .ts_config
            .time_from(self.timestep, unit, node_start)
            .to_string();
        Self::reply_capped(req.reply, req.size, s.as_bytes());
        Ok(())
    }

    pub fn send_elapsed(&mut self, req: fuse::ReadRequest) -> Result<(), RouterError> {
        let Some(unit) = Self::suffix_to_time(req.id.1.as_str()) else {
            let path = req.id.1.clone();
            drop(req.reply);
            return Err(RouterError::UnknownFile(path));
        };
        let s = self.ts_config.elapsed(self.timestep, unit).to_string();
        Self::reply_capped(req.reply, req.size, s.as_bytes());
        Ok(())
    }

    fn msg_to_time_units(msg: &fuse::Message) -> Result<(u64, TimeUnit), RouterError> {
        let unit = Self::suffix_to_time(msg.id.1.as_str())
            .ok_or_else(|| RouterError::UnknownFile(msg.id.1.clone()))?;
        let s = String::from_utf8_lossy(&msg.data);
        let val: u64 = s
            .trim()
            .parse()
            .map_err(|_| RouterError::InvalidString(msg.data.clone()))?;
        Ok((val, unit))
    }

    fn suffix_to_time(s: &str) -> Option<TimeUnit> {
        match s.rsplit('/').next()? {
            "us" => Some(TimeUnit::Microseconds),
            "ms" => Some(TimeUnit::Milliseconds),
            "s" => Some(TimeUnit::Seconds),
            "ns" => Some(TimeUnit::Nanoseconds),
            _ => None,
        }
    }
}
