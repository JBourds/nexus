//! energy.rs
//! Energy subsystem extracted from RoutingServer.
//!
//! Manages per-timestep energy sources, sinks, power-state drain,
//! death detection (charge == 0), and recovery (charge >= threshold).
//! Maintains `battery_nodes` so only battery-equipped nodes are iterated.

use tracing::{Level, event};

use crate::types::{ChannelHandle, EnergyState, Node};

/// Lightweight energy coordinator. Energy state lives in `Node.energy`;
/// this struct tracks which nodes have batteries and collects death/recovery
/// events each timestep.
#[derive(Debug)]
pub(super) struct EnergyManager {
    /// Indices of nodes that have an `EnergyState` (skips non-battery nodes).
    battery_nodes: Vec<usize>,
    /// Nodes whose charge reached 0 this step (drain -> output).
    pub newly_depleted: Vec<usize>,
    /// Nodes whose charge recovered above their restart threshold this step.
    pub newly_recovered: Vec<usize>,
}

impl EnergyManager {
    /// Build an `EnergyManager` from the initial node list.
    pub fn new(nodes: &[Node]) -> Self {
        let battery_nodes: Vec<usize> = nodes
            .iter()
            .enumerate()
            .filter_map(|(i, n)| n.energy.as_ref().map(|_| i))
            .collect();
        Self {
            battery_nodes,
            newly_depleted: Vec::new(),
            newly_recovered: Vec::new(),
        }
    }

    /// Run one timestep of energy bookkeeping on all battery-equipped nodes.
    ///
    /// After this call, `newly_depleted` and `newly_recovered` contain the
    /// node indices that transitioned this step.
    pub fn tick(&mut self, nodes: &mut [Node], timestep: u64, timestep_ns: u64) {
        let current_time_us = timestep * timestep_ns / 1000;

        for &node_idx in &self.battery_nodes {
            let Some(energy) = &mut nodes[node_idx].energy else {
                continue;
            };
            let was_dead = energy.is_dead;

            // Sources always apply (even when dead, e.g. solar charging)
            let source_nj: u64 = energy
                .power_sources
                .iter()
                .map(|(_, flow)| flow.nj_per_timestep(current_time_us))
                .sum();
            energy.charge_nj += source_nj;

            // Sinks always apply (saturating keeps charge at 0)
            let sink_nj: u64 = energy
                .power_sinks
                .iter()
                .map(|(_, flow)| flow.nj_per_timestep(current_time_us))
                .sum();
            energy.charge_nj = energy.charge_nj.saturating_sub(sink_nj);

            // Power state drain only when alive
            if !was_dead {
                let drain = energy
                    .current_state
                    .as_deref()
                    .and_then(|s| energy.power_states_nj.get(s).copied())
                    .unwrap_or(0);
                energy.charge_nj = energy.charge_nj.saturating_sub(drain);
            }
            energy.charge_nj = energy.charge_nj.min(energy.max_nj);

            // Detect transitions
            if !was_dead && energy.charge_nj == 0 {
                energy.is_dead = true;
                self.newly_depleted.push(node_idx);
            } else if was_dead
                && energy
                    .restart_threshold_nj
                    .is_some_and(|t| energy.charge_nj >= t)
            {
                energy.is_dead = false;
                self.newly_recovered.push(node_idx);
            }

            let charge_nj = energy.charge_nj;
            event!(target: "battery", Level::INFO, timestep, node = node_idx as u64, charge_nj);
        }
    }

    /// Deduct TX energy cost for a message sent on `channel` by node at `node_idx`.
    pub fn drain_tx(nodes: &mut [Node], node_idx: usize, channel: &ChannelHandle) {
        let tx_cost_nj: u64 = nodes[node_idx]
            .channel_energy
            .get(channel)
            .and_then(|ce| ce.tx.as_ref())
            .map(|e| e.unit.to_nj(e.quantity))
            .unwrap_or(0);
        if tx_cost_nj > 0
            && let Some(energy) = &mut nodes[node_idx].energy
        {
            energy.charge_nj = energy.charge_nj.saturating_sub(tx_cost_nj);
        }
    }

    /// Deduct RX energy cost for a message received on `channel` by node at `node_idx`.
    pub fn drain_rx(nodes: &mut [Node], node_idx: usize, channel: &ChannelHandle) {
        let rx_cost_nj: u64 = nodes[node_idx]
            .channel_energy
            .get(channel)
            .and_then(|ce| ce.rx.as_ref())
            .map(|e| e.unit.to_nj(e.quantity))
            .unwrap_or(0);
        if rx_cost_nj > 0
            && let Some(energy) = &mut nodes[node_idx].energy
        {
            energy.charge_nj = energy.charge_nj.saturating_sub(rx_cost_nj);
        }
    }

    /// Read the current charge in nanojoules for a node (0 if no battery).
    pub fn charge_nj(nodes: &[Node], node_idx: usize) -> u64 {
        nodes[node_idx]
            .energy
            .as_ref()
            .map_or(0, |e| e.charge_nj)
    }

    /// Read the current power state name for a node.
    pub fn current_state(nodes: &[Node], node_idx: usize) -> Option<String> {
        nodes[node_idx]
            .energy
            .as_ref()
            .and_then(|e| e.current_state.clone())
    }

    /// Set the power state for a node (only if the state is known).
    pub fn set_state(energy: &mut EnergyState, state: String) {
        if energy.power_states_nj.contains_key(&state) {
            energy.current_state = Some(state);
        }
    }
}
