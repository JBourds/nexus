//! Microbenchmark contrasting linear-scan vs hashmap entry lookup.
//!
//! Models `NexusFs::lookup` which today scans `entries: Vec<FsEntry>` to
//! find an (parent_inode, name) match. With M total entries (= sum across
//! all PIDs of control + channel files), each FUSE syscall pays O(M).
//!
//! Run with: `cargo run --release -p fuse --bin lookup_bench`

use std::collections::HashMap;
use std::time::Instant;

#[derive(Clone)]
struct Entry {
    name: String,
    parent_inode: u64,
}

fn build_entries(nodes: usize, channels_per_node: usize) -> Vec<Entry> {
    let mut entries = Vec::new();
    // Control files at root (per-pid mapping is by buffer; entries are global)
    let ctl_names = [
        "ctl.energy_left",
        "ctl.energy_state",
        "ctl.power_flows",
        "ctl.time",
        "ctl.elapsed",
        "ctl.pos",
    ];
    for n in ctl_names.iter() {
        entries.push(Entry {
            name: n.to_string(),
            parent_inode: 1,
        });
    }
    // Per-channel directories and 3 sub-files
    for n in 0..nodes {
        for c in 0..channels_per_node {
            let dir = format!("n{n}_ch{c}");
            let parent = entries.len() as u64 + 2;
            entries.push(Entry {
                name: dir.clone(),
                parent_inode: 1,
            });
            entries.push(Entry {
                name: "channel".into(),
                parent_inode: parent,
            });
            entries.push(Entry {
                name: "rssi".into(),
                parent_inode: parent,
            });
            entries.push(Entry {
                name: "snr".into(),
                parent_inode: parent,
            });
        }
    }
    entries
}

fn build_index(entries: &[Entry]) -> HashMap<(u64, String), usize> {
    entries
        .iter()
        .enumerate()
        .map(|(i, e)| ((e.parent_inode, e.name.clone()), i))
        .collect()
}

fn bench_linear(entries: &[Entry], queries: &[(u64, &str)], iters: u32) -> u128 {
    let t0 = Instant::now();
    let mut sink: u64 = 0;
    for _ in 0..iters {
        for (p, n) in queries {
            for (i, e) in entries.iter().enumerate() {
                if e.parent_inode == *p && e.name.as_str() == *n {
                    sink ^= i as u64;
                    break;
                }
            }
        }
    }
    std::hint::black_box(sink);
    t0.elapsed().as_nanos()
}

fn bench_hashmap(
    index: &HashMap<(u64, String), usize>,
    queries: &[(u64, &str)],
    iters: u32,
) -> u128 {
    let t0 = Instant::now();
    let mut sink: u64 = 0;
    for _ in 0..iters {
        for (p, n) in queries {
            // Avoid allocating: use (p, n) tuple via the entry API
            if let Some(&i) = index.get(&(*p, n.to_string())) {
                sink ^= i as u64;
            }
        }
    }
    std::hint::black_box(sink);
    t0.elapsed().as_nanos()
}

fn main() {
    println!("# FUSE entry lookup microbenchmark");
    println!("# nodes  chans  entries  q/iter  linear_ns/q  hashmap_ns/q  speedup");
    for &(nodes, chans) in &[(10usize, 2usize), (100, 2), (500, 4), (1000, 4), (2000, 4)] {
        let entries = build_entries(nodes, chans);
        let index = build_index(&entries);
        // Build a representative query mix: probe a leaf in the middle and
        // worst-case last-element name.
        let mid_parent = (entries.len() as u64) / 2 + 2;
        let last_parent = entries.len() as u64; // one past
        let queries: [(u64, &str); 4] = [
            (mid_parent, "channel"),
            (last_parent, "channel"),
            (1, "ctl.pos"),
            (1, "ctl.elapsed"),
        ];
        let iters = if entries.len() > 5_000 { 2_000 } else { 20_000 };
        let lin = bench_linear(&entries, &queries, iters);
        let hm = bench_hashmap(&index, &queries, iters);
        let total_q = (queries.len() as u128) * (iters as u128);
        let lin_ns = lin / total_q;
        let hm_ns = hm.max(1) / total_q;
        let speedup = lin as f64 / hm.max(1) as f64;
        println!(
            "  {nodes:>5}  {chans:>4}  {:>7}  {iters:>6}  {lin_ns:>11}  {hm_ns:>12}  {speedup:>6.1}x",
            entries.len()
        );
    }
}
