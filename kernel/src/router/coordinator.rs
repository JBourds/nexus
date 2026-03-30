//! coordinator.rs
//! Orchestrates multiple workers for distributed simulation.
//!
//! The coordinator owns the canonical node state (positions, energy), runs
//! the energy manager and motion updates, then dispatches channel work to
//! workers. In single-worker mode, it behaves identically to the original
//! `RoutingServer` step logic.

use rand::Rng;
use rand::SeedableRng;
use rand::rngs::StdRng;

use super::energy;
use super::errors::RouterError;
use super::partitioner::partition_channels;
use super::worker::{EnergyDelta, Worker};
use super::Timestep;
use crate::ResolvedChannels;

/// Manages one or more workers, synchronizing shared node state between steps.
#[derive(Debug)]
pub(crate) struct Coordinator {
    /// The workers, each owning a subset of channels.
    pub workers: Vec<Worker>,
    /// Mapping from handle_ptr to the worker index that owns it.
    pub handle_to_worker: Vec<usize>,
    /// Energy subsystem (runs on coordinator, not workers).
    pub(super) energy_mgr: energy::EnergyManager,
}

impl Coordinator {
    /// Build a coordinator with `num_workers` workers partitioned across channels.
    pub fn new(
        num_workers: usize,
        channels: &ResolvedChannels,
        base_rng: &mut StdRng,
    ) -> Self {
        let assignments = partition_channels(channels, num_workers.max(1));
        let energy_mgr = energy::EnergyManager::new(&channels.nodes);

        // Pre-generate deterministic seeds for each worker from the base RNG.
        let worker_seeds: Vec<u64> = (0..assignments.len())
            .map(|_| base_rng.random::<u64>())
            .collect();

        let workers: Vec<Worker> = assignments
            .into_iter()
            .enumerate()
            .map(|(id, owned_channels)| {
                let worker_rng = StdRng::seed_from_u64(worker_seeds[id]);
                Worker::new(id, owned_channels, channels, worker_rng)
            })
            .collect();

        // Build handle -> worker mapping.
        let mut handle_to_worker = vec![0usize; channels.handles.len()];
        for (worker_id, worker) in workers.iter().enumerate() {
            for &handle_ptr in &worker.owned_handles {
                handle_to_worker[handle_ptr] = worker_id;
            }
        }

        Self {
            workers,
            handle_to_worker,
            energy_mgr,
        }
    }

    /// Which worker owns the given handle index?
    pub fn worker_for_handle(&self, handle_ptr: usize) -> usize {
        self.handle_to_worker[handle_ptr]
    }

    /// Get a mutable reference to the worker that owns a handle.
    pub fn worker_for_handle_mut(&mut self, handle_ptr: usize) -> &mut Worker {
        let idx = self.handle_to_worker[handle_ptr];
        &mut self.workers[idx]
    }

    /// Run one simulation step: energy tick, motion, expire, deliver.
    ///
    /// In single-worker mode, this runs synchronously on the calling thread.
    /// In multi-worker mode, the expire + deliver phases run in parallel
    /// via `std::thread::scope`.
    pub fn step(
        &mut self,
        timestep: Timestep,
        channels: &mut ResolvedChannels,
        timestep_ns: u64,
    ) -> Result<(), RouterError> {
        // Phase 0: Coordinator updates energy and positions (shared state).
        self.energy_mgr
            .tick(&mut channels.nodes, timestep, timestep_ns);

        // Phase 1 & 2: Workers expire messages and deliver from local queues.
        if self.workers.len() == 1 {
            // Fast path: single worker, no threading overhead.
            let worker = &mut self.workers[0];
            worker.expire_messages(timestep);
            worker.deliver_queued_messages(timestep, channels)?;

            // Apply energy deltas from delivery directly.
            for delta in worker.take_energy_deltas() {
                Self::apply_energy_delta(&mut channels.nodes, delta);
            }
        } else {
            // Multi-worker: run expire + deliver in parallel.
            // We use thread::scope to borrow `channels` immutably across threads.
            // Energy deltas are collected after all workers finish.
            let workers = &mut self.workers;
            let channels_ref = &*channels; // immutable borrow for workers

            // Collect results from all workers.
            let mut all_deltas: Vec<Vec<EnergyDelta>> = Vec::with_capacity(workers.len());
            let mut first_error: Option<RouterError> = None;

            std::thread::scope(|s| {
                let handles: Vec<_> = workers
                    .iter_mut()
                    .map(|worker| {
                        s.spawn(move || {
                            worker.expire_messages(timestep);
                            worker.deliver_queued_messages(timestep, channels_ref)?;
                            Ok(worker.take_energy_deltas())
                        })
                    })
                    .collect();

                for handle in handles {
                    match handle.join() {
                        Ok(Ok(deltas)) => all_deltas.push(deltas),
                        Ok(Err(e)) => {
                            if first_error.is_none() {
                                first_error = Some(e);
                            }
                        }
                        Err(_) => {
                            if first_error.is_none() {
                                first_error = Some(RouterError::StepError);
                            }
                        }
                    }
                }
            });

            if let Some(e) = first_error {
                return Err(e);
            }

            // Phase 3: Apply energy deltas in worker-id order for determinism.
            for deltas in all_deltas {
                for delta in deltas {
                    Self::apply_energy_delta(&mut channels.nodes, delta);
                }
            }
        }

        Ok(())
    }

    /// Apply a single energy delta to the canonical node state.
    fn apply_energy_delta(nodes: &mut [crate::types::Node], delta: EnergyDelta) {
        if let Some(energy) = &mut nodes[delta.node_idx].energy {
            if delta.delta_nj < 0 {
                energy.charge_nj = energy
                    .charge_nj
                    .saturating_sub((-delta.delta_nj) as u64);
            } else {
                energy.charge_nj = energy
                    .charge_nj
                    .saturating_add(delta.delta_nj as u64)
                    .min(energy.max_nj);
            }
        }
    }

    /// Clear mailboxes for handles matching a PID (used during PID remap).
    pub fn clear_mailboxes_for_pid(&mut self, pid: u32, channels: &ResolvedChannels) {
        for worker in &mut self.workers {
            worker.clear_mailboxes_for_pid(pid, channels);
        }
    }
}
