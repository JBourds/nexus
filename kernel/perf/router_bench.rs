//! Microbenchmarks for the router hot paths.
//!
//! Targets:
//! 1. `expire_messages`: walks every mailbox per timestep. With M nodes ×
//!    C channels = thousands of mailboxes, even a no-op sweep dominates.
//! 2. mpsc round-trip: kernel and router exchange ~2 messages per kernel
//!    inner-loop iteration, the kernel spins for `delta` real-time per
//!    timestep, so this is one of the highest-frequency operations.
//!
//! Run: `cargo run --release -p kernel --bin router_bench`

use std::collections::VecDeque;
use std::num::NonZeroU64;
use std::sync::mpsc;
use std::time::{Duration, Instant};

#[derive(Clone)]
struct QueuedMsg {
    expiration: Option<NonZeroU64>,
}

fn bench_full_sweep(mailboxes: &mut [VecDeque<QueuedMsg>], ts: u64, sweeps: u32) -> u128 {
    let t0 = Instant::now();
    for _ in 0..sweeps {
        for mailbox in mailboxes.iter_mut() {
            while mailbox
                .front()
                .is_some_and(|m| m.expiration.is_some_and(|e| e.get() < ts))
            {
                mailbox.pop_front();
            }
        }
    }
    t0.elapsed().as_nanos()
}

fn bench_dirty_sweep(
    mailboxes: &mut [VecDeque<QueuedMsg>],
    dirty: &[usize],
    ts: u64,
    sweeps: u32,
) -> u128 {
    let t0 = Instant::now();
    for _ in 0..sweeps {
        for &i in dirty {
            let mailbox = &mut mailboxes[i];
            while mailbox
                .front()
                .is_some_and(|m| m.expiration.is_some_and(|e| e.get() < ts))
            {
                mailbox.pop_front();
            }
        }
    }
    t0.elapsed().as_nanos()
}

fn mailbox_sweep_bench() {
    println!("# Router mailbox sweep");
    println!("# mailboxes  full_ns/sweep  dirty_ns/sweep  speedup");
    for &count in &[100usize, 1_000, 5_000, 20_000, 50_000] {
        let mut full: Vec<VecDeque<QueuedMsg>> = (0..count).map(|_| VecDeque::new()).collect();
        let mut dirty_buf = full.clone();
        let dirty_idx: Vec<usize> = vec![0, count / 2, count - 1]; // representative active set
        let sweeps = 1_000;
        let full_ns = bench_full_sweep(&mut full, 100, sweeps) / sweeps as u128;
        let dirty_ns = bench_dirty_sweep(&mut dirty_buf, &dirty_idx, 100, sweeps).max(1)
            / sweeps as u128;
        let speedup = full_ns as f64 / dirty_ns.max(1) as f64;
        println!("  {count:>9}  {full_ns:>13}  {dirty_ns:>14}  {speedup:>6.1}x");
    }
}

fn mpsc_roundtrip_bench() {
    use std::thread;
    println!("\n# IPC round-trip latency (kernel <-> router)");
    println!("# channel  iters  ns/round-trip");

    // std::sync::mpsc
    {
        let (req_tx, req_rx) = mpsc::channel::<u64>();
        let (rep_tx, rep_rx) = mpsc::channel::<u64>();
        let h = thread::spawn(move || {
            for v in req_rx.iter() {
                if v == u64::MAX {
                    break;
                }
                let _ = rep_tx.send(v + 1);
            }
        });
        let iters = 200_000u32;
        let t0 = Instant::now();
        for i in 0..iters {
            req_tx.send(i as u64).unwrap();
            let _ = rep_rx.recv().unwrap();
        }
        let dt = t0.elapsed();
        let _ = req_tx.send(u64::MAX);
        let _ = h.join();
        let ns = dt.as_nanos() / iters as u128;
        println!("  std::mpsc  {iters}  {ns}");
    }

    // crossbeam-channel: not available in kernel deps, but we can show the
    // expected difference at the design level by skipping. (Handled by the
    // implementation switch.)
    let _ = Duration::from_nanos(1);
}

fn main() {
    mailbox_sweep_bench();
    mpsc_roundtrip_bench();
}
