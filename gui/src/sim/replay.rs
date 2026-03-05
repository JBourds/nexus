use std::path::Path;

use trace::format::{TraceEvent, TraceRecord};
use trace::reader::{TraceReadError, TraceReader};

use crate::state::NodeState;

/// Controls replay of a trace file with seeking and state reconstruction.
pub struct ReplayController {
    reader: TraceReader,
    /// All records, loaded into memory for fast seeking.
    all_records: Vec<TraceRecord>,
    pub total_timesteps: u64,
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

        Ok(Self {
            reader,
            all_records,
            total_timesteps,
        })
    }

    pub fn node_names(&self) -> &[String] {
        &self.reader.header.node_names
    }

    pub fn channel_names(&self) -> &[String] {
        &self.reader.header.channel_names
    }

    /// Get all records for a specific timestep.
    pub fn records_at(&self, ts: u64) -> Vec<&TraceRecord> {
        self.all_records
            .iter()
            .filter(|r| r.timestep == ts)
            .collect()
    }

    /// Get all records from timestep 0 through ts (inclusive).
    pub fn records_through(&self, ts: u64) -> Vec<&TraceRecord> {
        self.all_records
            .iter()
            .filter(|r| r.timestep <= ts)
            .collect()
    }

    /// Reconstruct node states at a given timestep by replaying all
    /// PositionUpdate and EnergyUpdate events from 0..=ts.
    pub fn reconstruct_states(&self, ts: u64, initial_states: &[NodeState]) -> Vec<NodeState> {
        let mut states = initial_states.to_vec();
        for record in self.all_records.iter().filter(|r| r.timestep <= ts) {
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
                        // Compute ratio based on energy vs initial max
                        // For now just normalize against initial
                        if state.charge_ratio.is_some() {
                            let ratio = (*energy_nj as f32) / 1.0e9;
                            state.charge_ratio = Some(ratio.clamp(0.0, 1.0));
                        }
                    }
                }
                _ => {}
            }
        }
        states
    }
}
