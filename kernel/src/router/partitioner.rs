//! partitioner.rs
//! Assigns channels to workers for distributed simulation.
//!
//! The partitioner balances work across workers while trying to co-locate
//! channels that share many nodes (to minimize cross-worker state sharing).

use std::collections::HashSet;

use crate::types::ChannelIdx;
use crate::ResolvedChannels;

/// Assign channels to `num_workers` workers, returning a Vec of channel sets.
///
/// Strategy:
/// 1. Compute a weight for each channel (number of publishers * subscribers
///    as a proxy for message volume).
/// 2. Sort channels by weight descending (largest-first).
/// 3. Greedily assign each channel to the worker with the lowest total weight,
///    breaking ties by preferring the worker with the most node overlap
///    (to reduce cross-worker synchronization).
pub(crate) fn partition_channels(
    channels: &ResolvedChannels,
    num_workers: usize,
) -> Vec<HashSet<ChannelIdx>> {
    assert!(num_workers > 0, "need at least one worker");

    let num_channels = channels.channels.len();
    if num_channels == 0 || num_workers == 1 {
        // Single worker owns everything.
        let all: HashSet<ChannelIdx> = (0..num_channels).map(ChannelIdx).collect();
        let mut result = vec![all];
        result.resize_with(num_workers, HashSet::new);
        return result;
    }

    // Compute weights: publishers * subscribers (minimum 1 to avoid zero-weight channels).
    let weights: Vec<u64> = channels
        .channels
        .iter()
        .map(|ch| {
            let p = ch.publishers.len() as u64;
            let s = ch.subscribers.len() as u64;
            (p * s).max(1)
        })
        .collect();

    // Sort channel indices by weight descending.
    let mut sorted_indices: Vec<usize> = (0..num_channels).collect();
    sorted_indices.sort_by(|&a, &b| weights[b].cmp(&weights[a]));

    // Per-worker state: total weight and set of node indices.
    let mut worker_weights = vec![0u64; num_workers];
    let mut worker_nodes: Vec<HashSet<usize>> = vec![HashSet::new(); num_workers];
    let mut assignments: Vec<HashSet<ChannelIdx>> = vec![HashSet::new(); num_workers];

    for ch_idx in sorted_indices {
        let ch = &channels.channels[ch_idx];
        let ch_nodes: HashSet<usize> = ch
            .publishers
            .iter()
            .chain(ch.subscribers.iter())
            .map(|n| n.0)
            .collect();

        // Find the worker with the lowest weight. Among ties, prefer the one
        // with the most node overlap with this channel.
        let best_worker = (0..num_workers)
            .min_by_key(|&w| {
                let overlap = worker_nodes[w].intersection(&ch_nodes).count();
                // Primary key: total weight (lower is better).
                // Secondary key: negative overlap (more overlap is better).
                (worker_weights[w], usize::MAX - overlap)
            })
            .unwrap();

        assignments[best_worker].insert(ChannelIdx(ch_idx));
        worker_weights[best_worker] += weights[ch_idx];
        worker_nodes[best_worker].extend(&ch_nodes);
    }

    assignments
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Channel, Node, NodeIdx};
    use config::ast::{ChannelType, Link};
    use std::sync::Arc;

    fn make_channels(specs: &[(Vec<usize>, Vec<usize>)]) -> ResolvedChannels {
        let max_node = specs
            .iter()
            .flat_map(|(p, s)| p.iter().chain(s.iter()))
            .copied()
            .max()
            .unwrap_or(0);
        let num_nodes = max_node + 1;
        let nodes: Vec<Node> = (0..num_nodes)
            .map(|_| Node {
                energy: None,
                position: Default::default(),
                motion: Default::default(),
                start: std::time::SystemTime::UNIX_EPOCH,
                protocols: vec![],
                channel_energy: Default::default(),
            })
            .collect();
        let channels: Vec<Channel> = specs
            .iter()
            .map(|(pubs, subs)| Channel {
                link: Link::default(),
                r#type: ChannelType::new_internal(),
                publishers: pubs.iter().map(|&n| NodeIdx(n)).collect(),
                subscribers: subs.iter().map(|&n| NodeIdx(n)).collect(),
            })
            .collect();
        ResolvedChannels {
            node_names: (0..num_nodes).map(|i| Arc::from(format!("n{i}"))).collect(),
            channel_names: (0..specs.len())
                .map(|i| Arc::from(format!("ch{i}")))
                .collect(),
            handles: vec![],
            nodes,
            channels,
        }
    }

    #[test]
    fn single_worker_gets_all() {
        let rc = make_channels(&[(vec![0], vec![1]), (vec![1], vec![2])]);
        let result = partition_channels(&rc, 1);
        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0],
            [ChannelIdx(0), ChannelIdx(1)].into_iter().collect()
        );
    }

    #[test]
    fn two_workers_split_two_channels() {
        let rc = make_channels(&[
            (vec![0, 1], vec![2, 3]), // weight 4
            (vec![4, 5], vec![6, 7]), // weight 4
        ]);
        let result = partition_channels(&rc, 2);
        assert_eq!(result.len(), 2);
        // Each worker should get exactly one channel.
        assert_eq!(result[0].len(), 1);
        assert_eq!(result[1].len(), 1);
    }

    #[test]
    fn affinity_groups_shared_nodes() {
        // ch0: nodes 0,1 -> nodes 2,3 (weight 4)
        // ch1: nodes 0,1 -> nodes 2,3 (weight 1, shares all nodes with ch0)
        // ch2: nodes 4,5 -> nodes 6,7 (weight 4, disjoint)
        //
        // With unequal weights, after ch0 (weight 4) goes to worker 0 and
        // ch2 (weight 4) goes to worker 1, ch1 (weight 1) should prefer
        // the lighter worker (both equal at 4 now). Tie-broken by affinity
        // with worker 0 which shares nodes.
        let rc = make_channels(&[
            (vec![0, 1], vec![2, 3]),  // weight 4
            (vec![0], vec![2]),         // weight 1, shares nodes with ch0
            (vec![4, 5], vec![6, 7]),  // weight 4
        ]);
        let result = partition_channels(&rc, 2);
        // ch0 and ch1 share nodes; with these weights, affinity should co-locate them.
        let ch0_worker = result.iter().position(|s| s.contains(&ChannelIdx(0))).unwrap();
        let ch1_worker = result.iter().position(|s| s.contains(&ChannelIdx(1))).unwrap();
        assert_eq!(ch0_worker, ch1_worker, "channels sharing nodes should be co-located");
    }

    #[test]
    fn empty_channels() {
        let rc = make_channels(&[]);
        let result = partition_channels(&rc, 4);
        assert_eq!(result.len(), 4);
        for s in &result {
            assert!(s.is_empty());
        }
    }
}
