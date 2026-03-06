use std::ops::Range;
use std::path::Path;

use trace::format::{TraceEvent, TraceRecord};
use trace::reader::{TraceReadError, TraceReader};

use crate::state::NodeState;

/// Controls replay of a trace file with seeking and state reconstruction.
pub struct ReplayController {
    reader: TraceReader,
    /// All records, loaded into memory for fast seeking.
    all_records: Vec<TraceRecord>,
    /// Sorted index: (timestep, range into all_records).
    ts_ranges: Vec<(u64, Range<usize>)>,
    pub total_timesteps: u64,
    /// Cached state from last reconstruction to allow incremental replay.
    last_reconstructed: Option<(u64, Vec<NodeState>)>,
}

impl ReplayController {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, TraceReadError> {
        let mut reader = TraceReader::open(path)?;
        let total_timesteps = reader.header.timestep_count;

        // Load all records into memory for fast seeking
        let mut all_records = Vec::new();
        reader.rewind()?;
        while let Some(record) = reader.next_record()? {
            all_records.push(record);
        }

        // Build timestep index
        let ts_ranges = build_ts_index(&all_records);

        Ok(Self {
            reader,
            all_records,
            ts_ranges,
            total_timesteps,
            last_reconstructed: None,
        })
    }

    pub fn node_names(&self) -> &[String] {
        &self.reader.header.node_names
    }

    pub fn channel_names(&self) -> &[String] {
        &self.reader.header.channel_names
    }

    pub fn node_max_nj(&self) -> &[Option<u64>] {
        &self.reader.header.node_max_nj
    }

    /// Get all records for a specific timestep (binary search).
    pub fn records_at(&self, ts: u64) -> &[TraceRecord] {
        match self.ts_ranges.binary_search_by_key(&ts, |(t, _)| *t) {
            Ok(idx) => &self.all_records[self.ts_ranges[idx].1.clone()],
            Err(_) => &[],
        }
    }

    /// Get all records from timestep 0 through ts (inclusive).
    pub fn records_through(&self, ts: u64) -> &[TraceRecord] {
        // Find the upper bound in ts_ranges
        let end_idx = match self.ts_ranges.binary_search_by_key(&ts, |(t, _)| *t) {
            Ok(idx) => self.ts_ranges[idx].1.end,
            Err(idx) => {
                if idx == 0 {
                    return &[];
                }
                self.ts_ranges[idx - 1].1.end
            }
        };
        &self.all_records[..end_idx]
    }

    /// Reconstruct node states at a given timestep by replaying
    /// PositionUpdate and EnergyUpdate events. Uses incremental caching.
    pub fn reconstruct_states(&mut self, ts: u64, initial_states: &[NodeState]) -> Vec<NodeState> {
        // Check if we can build incrementally from cached state
        if let Some((cached_ts, ref cached_states)) = self.last_reconstructed {
            if cached_ts == ts {
                return cached_states.clone();
            }
            if cached_ts < ts {
                // Apply only records in (cached_ts, ts]
                let mut states = cached_states.clone();
                let start_idx = match self.ts_ranges.binary_search_by_key(&cached_ts, |(t, _)| *t)
                {
                    Ok(idx) => self.ts_ranges[idx].1.end,
                    Err(idx) => {
                        if idx == 0 {
                            0
                        } else {
                            self.ts_ranges[idx - 1].1.end
                        }
                    }
                };
                let end_idx = match self.ts_ranges.binary_search_by_key(&ts, |(t, _)| *t) {
                    Ok(idx) => self.ts_ranges[idx].1.end,
                    Err(idx) => {
                        if idx == 0 {
                            0
                        } else {
                            self.ts_ranges[idx - 1].1.end
                        }
                    }
                };
                if start_idx < end_idx {
                    apply_state_updates(&mut states, &self.all_records[start_idx..end_idx]);
                }
                self.last_reconstructed = Some((ts, states.clone()));
                return states;
            }
            // cached_ts > ts: seeking backward, rebuild from scratch
        }

        // Full rebuild from initial
        let mut states = initial_states.to_vec();
        let records = self.records_through(ts);
        apply_state_updates(&mut states, records);
        self.last_reconstructed = Some((ts, states.clone()));
        states
    }
}

fn apply_state_updates(states: &mut [NodeState], records: &[TraceRecord]) {
    for record in records {
        match &record.event {
            TraceEvent::PositionUpdate { node, x, y, z } => {
                if let Some(state) = states.get_mut(*node as usize) {
                    state.x = *x;
                    state.y = *y;
                    state.z = *z;
                }
            }
            TraceEvent::EnergyUpdate { node, energy_nj } => {
                if let Some(state) = states.get_mut(*node as usize) {
                    if let Some(max) = state.max_nj {
                        let ratio = if max == 0 { 1.0 } else { *energy_nj as f32 / max as f32 };
                        state.charge_ratio = Some(ratio.clamp(0.0, 1.0));
                        state.is_dead = *energy_nj == 0 && max > 0;
                    }
                }
            }
            _ => {}
        }
    }
}

/// Build an index mapping each timestep to its range in the records vec.
fn build_ts_index(records: &[TraceRecord]) -> Vec<(u64, Range<usize>)> {
    let mut ranges = Vec::new();
    if records.is_empty() {
        return ranges;
    }
    let mut start = 0;
    let mut current_ts = records[0].timestep;
    for (i, record) in records.iter().enumerate() {
        if record.timestep != current_ts {
            ranges.push((current_ts, start..i));
            current_ts = record.timestep;
            start = i;
        }
    }
    ranges.push((current_ts, start..records.len()));
    ranges
}
