#[cfg(test)]
mod tests {
    use crate::{
        resolver::ResolvedChannels,
        router::{RoutingServer, table::RoutingTable},
        types::{self, EnergyState, PowerFlowState},
    };
    use config::ast::{
        ChannelEnergy, ChannelType, Energy, EnergyUnit, Link, Position, TimeUnit, TimestepConfig,
    };
    use rand::{SeedableRng, rngs::StdRng};
    use std::{
        collections::{BinaryHeap, HashMap, HashSet, VecDeque},
        num::NonZeroU64,
        path::PathBuf,
        sync::mpsc,
        time::SystemTime,
    };

    /// 1 ms timestep config for deterministic testing.
    fn test_ts_config() -> TimestepConfig {
        TimestepConfig {
            length: NonZeroU64::new(1).unwrap(),
            unit: TimeUnit::Milliseconds,
            count: NonZeroU64::new(1000).unwrap(),
            start: SystemTime::UNIX_EPOCH,
        }
    }

    /// Build a minimal node with an energy state and no protocols.
    fn make_node(energy: Option<EnergyState>) -> types::Node {
        types::Node {
            energy,
            position: Position::default(),
            motion: types::MotionPattern::Static,
            start: SystemTime::UNIX_EPOCH,
            protocols: vec![],
            channel_energy: HashMap::new(),
        }
    }

    /// Build a node with protocols that have channel energy costs.
    fn make_node_with_protocol(
        energy: Option<EnergyState>,
        subscribers: HashSet<usize>,
        publishers: HashSet<usize>,
        channel_energy: HashMap<usize, ChannelEnergy>,
    ) -> types::Node {
        types::Node {
            energy,
            position: Position::default(),
            motion: types::MotionPattern::Static,
            start: SystemTime::UNIX_EPOCH,
            channel_energy,
            protocols: vec![types::NodeProtocol {
                root: PathBuf::from("/tmp"),
                runner: config::ast::Cmd {
                    cmd: String::new(),
                    args: vec![],
                },
                subscribers,
                publishers,
            }],
        }
    }

    /// Build a RoutingServer with the given nodes and optional channel setup.
    /// Returns the server and fuse receiver (for draining messages).
    fn make_router(
        nodes: Vec<types::Node>,
        channels: Vec<types::Channel>,
        handles: Vec<(u32, usize, usize)>,
    ) -> (RoutingServer, mpsc::Receiver<fuse::KernelMessage>) {
        let (tx, rx) = mpsc::channel();
        let node_names: Vec<String> = (0..nodes.len()).map(|i| format!("node_{i}")).collect();
        let channel_names: Vec<String> = (0..channels.len()).map(|i| format!("ch_{i}")).collect();
        let resolved = ResolvedChannels {
            nodes,
            node_names,
            channels,
            channel_names,
            handles: handles.clone(),
        };
        let fuse_mapping = resolved.make_fuse_mapping();
        let routes = RoutingTable::new(&resolved);
        let mailbox_count = handles.len();
        let router = RoutingServer {
            timestep: 1,
            ts_config: test_ts_config(),
            channels: resolved,
            routes,
            queued: BinaryHeap::new(),
            fuse_mapping,
            mailboxes: vec![VecDeque::new(); mailbox_count],
            rng: StdRng::seed_from_u64(42),
            tx,
            newly_depleted: Vec::new(),
            newly_recovered: Vec::new(),
            pending_remaps: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
        };
        (router, rx)
    }

    /// Simple router with a single node, no channels.
    fn make_single_node_router(
        energy: EnergyState,
    ) -> (RoutingServer, mpsc::Receiver<fuse::KernelMessage>) {
        make_router(vec![make_node(Some(energy))], vec![], vec![])
    }

    fn basic_energy(charge_nj: u64, max_nj: u64) -> EnergyState {
        EnergyState {
            charge_nj,
            max_nj,
            power_sources: vec![],
            power_sinks: vec![],
            power_states_nj: HashMap::new(),
            current_state: None,
            restart_threshold_nj: None,
            is_dead: false,
        }
    }

    /// Helper: create an EnergyState with a constant power source.
    fn energy_with_source(charge_nj: u64, max_nj: u64, source_nj_per_ts: u64) -> EnergyState {
        EnergyState {
            power_sources: vec![(
                "ambient".into(),
                PowerFlowState::Constant {
                    nj_per_ts: source_nj_per_ts,
                },
            )],
            ..basic_energy(charge_nj, max_nj)
        }
    }

    // -----------------------------------------------------------------------
    // Test: per-timestep power source generation
    // -----------------------------------------------------------------------
    #[test]
    fn test_power_source_generation() {
        let energy = energy_with_source(500, 10_000, 100);
        let (mut router, _rx) = make_single_node_router(energy);

        router.step().unwrap();
        let charge = router.channels.nodes[0].energy.as_ref().unwrap().charge_nj;
        assert_eq!(charge, 600, "Source should add 100 nJ per step");
    }

    // -----------------------------------------------------------------------
    // Test: per-timestep power state drain
    // -----------------------------------------------------------------------
    #[test]
    fn test_power_state_drain() {
        let mut energy = basic_energy(1000, 10_000);
        energy.power_states_nj = HashMap::from([("active".into(), 150)]);
        energy.current_state = Some("active".into());
        let (mut router, _rx) = make_single_node_router(energy);

        router.step().unwrap();
        let charge = router.channels.nodes[0].energy.as_ref().unwrap().charge_nj;
        assert_eq!(charge, 850, "Active state should drain 150 nJ per step");
    }

    // -----------------------------------------------------------------------
    // Test: source + drain combined
    // -----------------------------------------------------------------------
    #[test]
    fn test_source_plus_drain() {
        let mut energy = energy_with_source(1000, 10_000, 50);
        energy.power_states_nj = HashMap::from([("active".into(), 200)]);
        energy.current_state = Some("active".into());
        let (mut router, _rx) = make_single_node_router(energy);

        router.step().unwrap();
        let charge = router.channels.nodes[0].energy.as_ref().unwrap().charge_nj;
        // 1000 + 50 (source) - 200 (drain) = 850
        assert_eq!(charge, 850);
    }

    // -----------------------------------------------------------------------
    // Test: charge capping at max
    // -----------------------------------------------------------------------
    #[test]
    fn test_charge_capped_at_max() {
        let energy = energy_with_source(9950, 10_000, 200);
        let (mut router, _rx) = make_single_node_router(energy);

        router.step().unwrap();
        let charge = router.channels.nodes[0].energy.as_ref().unwrap().charge_nj;
        // 9950 + 200 = 10150, capped to 10000
        assert_eq!(charge, 10_000, "Charge should be capped at max_nj");
    }

    // -----------------------------------------------------------------------
    // Test: node death when charge <= 0
    // -----------------------------------------------------------------------
    #[test]
    fn test_node_death() {
        let mut energy = basic_energy(100, 10_000);
        energy.power_states_nj = HashMap::from([("active".into(), 150)]);
        energy.current_state = Some("active".into());
        let (mut router, _rx) = make_single_node_router(energy);

        router.step().unwrap();
        let e = router.channels.nodes[0].energy.as_ref().unwrap();
        // 100 - 150 saturates to 0
        assert_eq!(e.charge_nj, 0);
        assert!(e.is_dead, "Node should be dead when charge == 0");
        // step() pushes to newly_depleted (serve() drains it after each poll)
        assert_eq!(router.newly_depleted, vec![0]);
    }

    // -----------------------------------------------------------------------
    // Test: depleted vector populated on death
    // -----------------------------------------------------------------------
    #[test]
    fn test_newly_depleted_populated() {
        let mut energy = basic_energy(100, 10_000);
        energy.power_states_nj = HashMap::from([("active".into(), 150)]);
        energy.current_state = Some("active".into());
        let (mut router, _rx) = make_single_node_router(energy);

        // step() pushes to newly_depleted but doesn't drain it
        router.step().unwrap();
        // In the serve() loop, newly_depleted gets drained after each poll.
        // Since we call step() directly, it should still be there.
        assert_eq!(router.newly_depleted, vec![0]);
    }

    // -----------------------------------------------------------------------
    // Test: dead node gets sources and sinks but no state drain
    // -----------------------------------------------------------------------
    #[test]
    fn test_dead_node_sources_and_sinks() {
        let mut energy = energy_with_source(0, 10_000, 50);
        energy.power_sinks.push((
            "mcu".into(),
            PowerFlowState::Constant { nj_per_ts: 20 },
        ));
        energy.power_states_nj = HashMap::from([("active".into(), 200)]);
        energy.current_state = Some("active".into());
        energy.is_dead = true;
        let (mut router, _rx) = make_single_node_router(energy);

        router.step().unwrap();
        let charge = router.channels.nodes[0].energy.as_ref().unwrap().charge_nj;
        // 0 + 50 (source) - 20 (sink) - 0 (no power state drain while dead) = 30
        assert_eq!(
            charge, 30,
            "Dead node should get sources and sinks, but no power state drain"
        );
    }

    // -----------------------------------------------------------------------
    // Test: node restart when charge reaches threshold
    // -----------------------------------------------------------------------
    #[test]
    fn test_node_restart_at_threshold() {
        let mut energy = energy_with_source(0, 10_000, 100);
        energy.is_dead = true;
        // restart at 50% = 5000 nJ — won't trigger yet
        energy.restart_threshold_nj = Some(5000);
        let (mut router, _rx) = make_single_node_router(energy);

        // Step 1: 0 + 100 = 100, still below 5000
        router.step().unwrap();
        let e = router.channels.nodes[0].energy.as_ref().unwrap();
        assert_eq!(e.charge_nj, 100);
        assert!(e.is_dead, "Should still be dead at 100 nJ");

        // Now set charge close to threshold
        router.channels.nodes[0].energy.as_mut().unwrap().charge_nj = 4950;

        // Step: 4950 + 100 = 5050 >= 5000 threshold
        router.step().unwrap();
        let e = router.channels.nodes[0].energy.as_ref().unwrap();
        assert_eq!(e.charge_nj, 5050);
        assert!(!e.is_dead, "Should restart when charge >= threshold");
        assert_eq!(router.newly_recovered, vec![0]);
    }

    // -----------------------------------------------------------------------
    // Test: permanent death (no restart threshold)
    // -----------------------------------------------------------------------
    #[test]
    fn test_permanent_death_without_threshold() {
        let mut energy = energy_with_source(0, 10_000, 10_000);
        energy.is_dead = true;
        energy.restart_threshold_nj = None; // no restart
        let (mut router, _rx) = make_single_node_router(energy);

        // Even with massive source, node stays dead
        router.step().unwrap();
        let e = router.channels.nodes[0].energy.as_ref().unwrap();
        assert!(
            e.is_dead,
            "Node without restart_threshold should stay dead permanently"
        );
        // Charge still accumulates (capped at max)
        assert_eq!(e.charge_nj, 10_000);
    }

    // -----------------------------------------------------------------------
    // Test: no energy (None) — node without battery works fine
    // -----------------------------------------------------------------------
    #[test]
    fn test_no_battery_node() {
        let (mut router, _rx) = make_router(vec![make_node(None)], vec![], vec![]);
        // Should not panic
        router.step().unwrap();
        assert!(router.channels.nodes[0].energy.is_none());
    }

    // -----------------------------------------------------------------------
    // Test: multiple timesteps accumulate correctly
    // -----------------------------------------------------------------------
    #[test]
    fn test_multi_step_accumulation() {
        let mut energy = energy_with_source(1000, 10_000, 10);
        energy.power_states_nj = HashMap::from([("idle".into(), 3)]);
        energy.current_state = Some("idle".into());
        let (mut router, _rx) = make_single_node_router(energy);

        for _ in 0..100 {
            router.step().unwrap();
        }
        let charge = router.channels.nodes[0].energy.as_ref().unwrap().charge_nj;
        // 1000 + 100*(10 - 3) = 1000 + 700 = 1700
        assert_eq!(charge, 1700);
    }

    // -----------------------------------------------------------------------
    // Test: power state with no current_state means no drain
    // -----------------------------------------------------------------------
    #[test]
    fn test_no_current_state_no_drain() {
        let mut energy = basic_energy(1000, 10_000);
        energy.power_states_nj = HashMap::from([("active".into(), 200)]);
        energy.current_state = None; // no state selected
        let (mut router, _rx) = make_single_node_router(energy);

        router.step().unwrap();
        let charge = router.channels.nodes[0].energy.as_ref().unwrap().charge_nj;
        assert_eq!(charge, 1000, "No current state means zero drain");
    }

    // -----------------------------------------------------------------------
    // Test: TX energy cost deducted from sender
    // -----------------------------------------------------------------------
    #[test]
    fn test_tx_energy_deduction() {
        let energy = basic_energy(5000, 10_000);
        let ch_handle: usize = 0;

        // Create a channel energy cost: 100 µJ per TX
        let channel_energy = HashMap::from([(
            ch_handle,
            ChannelEnergy {
                tx: Some(Energy {
                    quantity: 100,
                    unit: EnergyUnit::MicroJoule,
                }),
                rx: None,
            },
        )]);

        let node = make_node_with_protocol(
            Some(energy),
            HashSet::new(),
            HashSet::from([ch_handle]),
            channel_energy,
        );

        // Create a simple exclusive channel
        let channel = types::Channel {
            link: Link::default(),
            r#type: ChannelType::new_internal(),
            subscribers: HashSet::new(),
            publishers: HashSet::from([0]),
        };

        // Handle: PID=1, node=0, channel=0
        let handles = vec![(1u32, 0usize, 0usize)];
        let (mut router, _rx) = make_router(vec![node], vec![channel], handles);

        // Simulate a write
        let msg = fuse::Message {
            id: (1, "ch_0".to_string()),
            data: vec![0x42],
        };
        router.write_channel_file(0, msg).unwrap();

        let charge = router.channels.nodes[0].energy.as_ref().unwrap().charge_nj;
        // 5000 saturating_sub 100_000 (100 µJ = 100,000 nJ) = 0
        assert_eq!(charge, 0, "TX cost exceeds charge, should saturate to 0");
    }

    // -----------------------------------------------------------------------
    // Test: RX energy cost deducted on delivery
    // -----------------------------------------------------------------------
    #[test]
    fn test_rx_energy_deduction() {
        let ch_handle: usize = 0;

        // Publisher node (node 0) — no energy tracking needed for this test
        let pub_node = make_node_with_protocol(
            Some(basic_energy(10_000, 10_000)),
            HashSet::new(),
            HashSet::from([ch_handle]),
            HashMap::new(),
        );

        // Subscriber node (node 1) — with RX cost
        let rx_energy = HashMap::from([(
            ch_handle,
            ChannelEnergy {
                tx: None,
                rx: Some(Energy {
                    quantity: 50,
                    unit: EnergyUnit::MicroJoule,
                }),
            },
        )]);
        let sub_node = make_node_with_protocol(
            Some(basic_energy(8000_000, 10_000_000)),
            HashSet::from([ch_handle]),
            HashSet::new(),
            rx_energy,
        );

        // Channel with both pub (node 0) and sub (node 1)
        let channel = types::Channel {
            link: Link::default(),
            r#type: ChannelType::new_internal(),
            subscribers: HashSet::from([1]),
            publishers: HashSet::from([0]),
        };

        // Handles: (PID, node, channel)
        // handle 0: publisher (pid=1, node=0, ch=0)
        // handle 1: subscriber (pid=2, node=1, ch=0)
        let handles = vec![(1u32, 0usize, 0usize), (2u32, 1usize, 0usize)];
        let (mut router, _rx) = make_router(vec![pub_node, sub_node], vec![channel], handles);

        let initial_charge = router.channels.nodes[1].energy.as_ref().unwrap().charge_nj;

        // Queue a message from node 0 to node 1 via channel 0
        router.queue_message(0, 0, vec![0xAB]).unwrap();

        // Step to deliver the queued message (it becomes active at a future timestep)
        // The link default has zero delays, so it should arrive at current timestep
        router.step().unwrap();

        let charge = router.channels.nodes[1].energy.as_ref().unwrap().charge_nj;
        // RX cost: 50 µJ = 50_000 nJ
        // Also source (0) and drain (0) applied during step
        let expected = initial_charge - 50_000;
        assert_eq!(charge, expected, "RX should deduct 50 µJ = 50,000 nJ");
    }

    // -----------------------------------------------------------------------
    // Test: energy state transition via write_control_file
    // -----------------------------------------------------------------------
    #[test]
    fn test_energy_state_transition() {
        let mut energy = basic_energy(5000, 10_000);
        energy.power_states_nj = HashMap::from([("idle".into(), 10), ("active".into(), 200)]);
        energy.current_state = Some("idle".into());

        let node =
            make_node_with_protocol(Some(energy), HashSet::new(), HashSet::new(), HashMap::new());

        // We need a control file handle. The channel_names need to include the control prefix.
        let (tx, _rx) = mpsc::channel();
        let node_names = vec!["node_0".to_string()];
        let channel_names = vec!["ctl.energy_state".to_string()];
        let handles = vec![(1u32, 0usize, 0usize)];
        let resolved = ResolvedChannels {
            nodes: vec![node],
            node_names,
            channels: vec![types::Channel {
                link: Link::default(),
                r#type: ChannelType::new_internal(),
                subscribers: HashSet::new(),
                publishers: HashSet::new(),
            }],
            channel_names,
            handles: handles.clone(),
        };
        let fuse_mapping = resolved.make_fuse_mapping();
        let routes = RoutingTable::new(&resolved);
        let mut router = RoutingServer {
            timestep: 1,
            ts_config: test_ts_config(),
            channels: resolved,
            routes,
            queued: BinaryHeap::new(),
            fuse_mapping,
            mailboxes: vec![VecDeque::new(); handles.len()],
            rng: StdRng::seed_from_u64(42),
            tx,
            newly_depleted: Vec::new(),
            newly_recovered: Vec::new(),
            pending_remaps: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
        };

        // Write "active" to ctl.energy_state
        let msg = fuse::Message {
            id: (1, "ctl.energy_state".to_string()),
            data: b"active".to_vec(),
        };
        router.write_control_file(0, msg).unwrap();

        let state = router.channels.nodes[0]
            .energy
            .as_ref()
            .unwrap()
            .current_state
            .as_deref();
        assert_eq!(state, Some("active"));

        // Step and verify it now drains at "active" rate
        router.step().unwrap();
        let charge = router.channels.nodes[0].energy.as_ref().unwrap().charge_nj;
        assert_eq!(charge, 5000 - 200);
    }

    // -----------------------------------------------------------------------
    // Test: writing unknown state is ignored
    // -----------------------------------------------------------------------
    #[test]
    fn test_unknown_energy_state_ignored() {
        let mut energy = basic_energy(5000, 10_000);
        energy.power_states_nj = HashMap::from([("idle".into(), 10)]);
        energy.current_state = Some("idle".into());

        let node =
            make_node_with_protocol(Some(energy), HashSet::new(), HashSet::new(), HashMap::new());

        let (tx, _rx) = mpsc::channel();
        let handles = vec![(1u32, 0usize, 0usize)];
        let resolved = ResolvedChannels {
            nodes: vec![node],
            node_names: vec!["node_0".to_string()],
            channels: vec![types::Channel {
                link: Link::default(),
                r#type: ChannelType::new_internal(),
                subscribers: HashSet::new(),
                publishers: HashSet::new(),
            }],
            channel_names: vec!["ctl.energy_state".to_string()],
            handles: handles.clone(),
        };
        let fuse_mapping = resolved.make_fuse_mapping();
        let routes = RoutingTable::new(&resolved);
        let mut router = RoutingServer {
            timestep: 1,
            ts_config: test_ts_config(),
            channels: resolved,
            routes,
            queued: BinaryHeap::new(),
            fuse_mapping,
            mailboxes: vec![VecDeque::new(); handles.len()],
            rng: StdRng::seed_from_u64(42),
            tx,
            newly_depleted: Vec::new(),
            newly_recovered: Vec::new(),
            pending_remaps: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
        };

        // Write unknown state
        let msg = fuse::Message {
            id: (1, "ctl.energy_state".to_string()),
            data: b"turbo".to_vec(),
        };
        router.write_control_file(0, msg).unwrap();

        // Should still be "idle"
        let state = router.channels.nodes[0]
            .energy
            .as_ref()
            .unwrap()
            .current_state
            .as_deref();
        assert_eq!(state, Some("idle"));
    }

    // -----------------------------------------------------------------------
    // Test: alive → dead → restart full lifecycle
    // -----------------------------------------------------------------------
    #[test]
    fn test_full_lifecycle() {
        let mut energy = energy_with_source(300, 10_000, 50);
        energy.power_states_nj = HashMap::from([("active".into(), 200)]);
        energy.current_state = Some("active".into());
        energy.restart_threshold_nj = Some(500);
        let (mut router, _rx) = make_single_node_router(energy);

        // Step 1: 300 + 50 - 200 = 150 (alive)
        router.step().unwrap();
        let e = router.channels.nodes[0].energy.as_ref().unwrap();
        assert_eq!(e.charge_nj, 150);
        assert!(!e.is_dead);

        // Step 2: 150 + 50 - 200 = 0 (dead! charge <= 0)
        router.newly_depleted.clear();
        router.step().unwrap();
        let e = router.channels.nodes[0].energy.as_ref().unwrap();
        assert_eq!(e.charge_nj, 0);
        assert!(e.is_dead);
        assert_eq!(router.newly_depleted, vec![0]);

        // Steps 3-12: dead, only sources. 0 + 10*50 = 500 (at threshold!)
        for _ in 0..10 {
            router.newly_recovered.clear();
            router.step().unwrap();
        }
        let e = router.channels.nodes[0].energy.as_ref().unwrap();
        assert_eq!(e.charge_nj, 500);
        assert!(!e.is_dead, "Should restart at threshold");
        assert_eq!(router.newly_recovered, vec![0]);

        // Step 13: alive again, draining. 500 + 50 - 200 = 350
        router.step().unwrap();
        let e = router.channels.nodes[0].energy.as_ref().unwrap();
        assert_eq!(e.charge_nj, 350);
        assert!(!e.is_dead);
    }

    // -----------------------------------------------------------------------
    // Test: EnergyState::from_node conversion
    // -----------------------------------------------------------------------
    #[test]
    fn test_from_node_basic() {
        let node = config::ast::Node {
            position: Position::default(),
            charge: Some(config::ast::Charge {
                max: 1000,
                quantity: 500,
                unit: EnergyUnit::MicroJoule,
            }),
            protocols: HashMap::new(),
            internal_names: vec![],
            resources: config::ast::Resources::default(),
            power_states: HashMap::from([(
                "active".into(),
                config::ast::PowerRate {
                    rate: 10,
                    unit: config::ast::PowerUnit::MilliWatt,
                    time: TimeUnit::Seconds,
                },
            )]),
            power_sources: HashMap::from([(
                "solar".into(),
                config::ast::PowerFlow::Constant(config::ast::PowerRate {
                    rate: 2,
                    unit: config::ast::PowerUnit::MilliWatt,
                    time: TimeUnit::Seconds,
                }),
            )]),
            power_sinks: HashMap::new(),
            channel_energy: HashMap::new(),
            initial_state: Some("active".into()),
            restart_threshold: Some(0.5),
            start: SystemTime::UNIX_EPOCH,
        };
        let ts = test_ts_config(); // 1 ms timesteps
        let e = EnergyState::from_node(&node, &ts).unwrap();

        // 1000 µJ * 1000 nJ/µJ = 1_000_000 nJ
        assert_eq!(e.max_nj, 1_000_000);
        // 500 µJ * 1000 = 500_000 nJ
        assert_eq!(e.charge_nj, 500_000);
        // 10 mW = 10_000_000 nW; timestep = 1ms = 1_000_000 ns; per second = 1e9 ns
        // nj/ts = 10_000_000 * 1_000_000 / 1_000_000_000 = 10_000
        assert_eq!(*e.power_states_nj.get("active").unwrap(), 10_000);
        // Power source "solar": 2 mW → 2_000_000 nW; same formula → 2_000
        assert_eq!(e.power_sources.len(), 1);
        assert_eq!(e.power_sources[0].0, "solar");
        assert_eq!(e.power_sources[0].1.nj_per_timestep(0), 2_000);
        assert_eq!(e.current_state.as_deref(), Some("active"));
        // restart_threshold = 0.5 * 1_000_000 = 500_000
        assert_eq!(e.restart_threshold_nj, Some(500_000));
        assert!(!e.is_dead);
    }

    #[test]
    fn test_from_node_no_charge_returns_none() {
        let node = config::ast::Node {
            position: Position::default(),
            charge: None,
            protocols: HashMap::new(),
            internal_names: vec![],
            resources: config::ast::Resources::default(),
            power_states: HashMap::new(),
            power_sources: HashMap::new(),
            power_sinks: HashMap::new(),
            channel_energy: HashMap::new(),
            initial_state: None,
            restart_threshold: None,
            start: SystemTime::UNIX_EPOCH,
        };
        assert!(EnergyState::from_node(&node, &test_ts_config()).is_none());
    }

    #[test]
    fn test_from_node_zero_charge_is_dead() {
        let node = config::ast::Node {
            position: Position::default(),
            charge: Some(config::ast::Charge {
                max: 100,
                quantity: 0,
                unit: EnergyUnit::NanoJoule,
            }),
            protocols: HashMap::new(),
            internal_names: vec![],
            resources: config::ast::Resources::default(),
            power_states: HashMap::new(),
            power_sources: HashMap::new(),
            power_sinks: HashMap::new(),
            channel_energy: HashMap::new(),
            initial_state: None,
            restart_threshold: None,
            start: SystemTime::UNIX_EPOCH,
        };
        let e = EnergyState::from_node(&node, &test_ts_config()).unwrap();
        assert!(e.is_dead, "Zero initial charge means node starts dead");
    }

    // -----------------------------------------------------------------------
    // Test: nj_per_timestep unit conversion
    // -----------------------------------------------------------------------
    #[test]
    fn test_nj_per_timestep_milliwatt_per_second() {
        use config::ast::{PowerRate, PowerUnit};
        let rate = PowerRate {
            rate: 100,
            unit: PowerUnit::MilliWatt,
            time: TimeUnit::Seconds,
        };
        // 100 mW = 100_000_000 nW; timestep = 1ms = 1_000_000 ns
        // nj/ts = 100_000_000 * 1_000_000 / 1_000_000_000 = 100_000
        assert_eq!(rate.nj_per_timestep(1_000_000), 100_000);
    }

    #[test]
    fn test_nj_per_timestep_watt_per_millisecond() {
        use config::ast::{PowerRate, PowerUnit};
        let rate = PowerRate {
            rate: 1,
            unit: PowerUnit::Watt,
            time: TimeUnit::Milliseconds,
        };
        // 1 W = 1_000_000_000 nW; time = 1ms = 1_000_000 ns
        // For a 1ms timestep: nj/ts = 1_000_000_000 * 1_000_000 / 1_000_000 = 1_000_000_000
        assert_eq!(rate.nj_per_timestep(1_000_000), 1_000_000_000);
    }

    // -----------------------------------------------------------------------
    // Test: two nodes with independent energy tracking
    // -----------------------------------------------------------------------
    #[test]
    fn test_two_nodes_independent() {
        let energy_a = {
            let mut e = energy_with_source(1000, 10_000, 10);
            e.power_states_nj = HashMap::from([("on".into(), 100)]);
            e.current_state = Some("on".into());
            e
        };
        let energy_b = energy_with_source(500, 5000, 200);
        let (mut router, _rx) = make_router(
            vec![make_node(Some(energy_a)), make_node(Some(energy_b))],
            vec![],
            vec![],
        );

        router.step().unwrap();

        let a = router.channels.nodes[0].energy.as_ref().unwrap().charge_nj;
        let b = router.channels.nodes[1].energy.as_ref().unwrap().charge_nj;
        // A: 1000 + 10 - 100 = 910
        assert_eq!(a, 910);
        // B: 500 + 200 = 700
        assert_eq!(b, 700);
    }

    // -----------------------------------------------------------------------
    // Test: apply_pid_remaps updates handles and rebuilds fuse_mapping
    // -----------------------------------------------------------------------
    #[test]
    fn test_pid_remap_updates_handles_and_fuse_mapping() {
        let energy = basic_energy(5000, 10_000);
        let ch_handle: usize = 0;
        let node = make_node_with_protocol(
            Some(energy),
            HashSet::from([ch_handle]),
            HashSet::from([ch_handle]),
            HashMap::new(),
        );
        let channel = types::Channel {
            link: Link::default(),
            r#type: ChannelType::new_internal(),
            subscribers: HashSet::from([0]),
            publishers: HashSet::from([0]),
        };
        // Handle: PID=100, node=0, channel=0
        let handles = vec![(100u32, 0usize, 0usize)];
        let (mut router, _rx) = make_router(vec![node], vec![channel], handles);

        // Verify initial state
        assert_eq!(router.channels.handles[0].0, 100);
        assert!(router.fuse_mapping.contains_key(&(100, "ch_0".to_string())));

        // Remap PID 100 → 200
        router.apply_pid_remaps(&[(100, 200)]);

        // Handle PID should be updated
        assert_eq!(router.channels.handles[0].0, 200);
        // fuse_mapping should reflect new PID
        assert!(!router.fuse_mapping.contains_key(&(100, "ch_0".to_string())));
        assert!(router.fuse_mapping.contains_key(&(200, "ch_0".to_string())));
    }

    // -----------------------------------------------------------------------
    // Test: apply_pid_remaps clears mailboxes for remapped handles
    // -----------------------------------------------------------------------
    #[test]
    fn test_pid_remap_clears_mailboxes() {
        let energy = basic_energy(10_000, 10_000);
        let ch_handle: usize = 0;

        // Publisher node 0, subscriber node 1
        let pub_node = make_node_with_protocol(
            Some(basic_energy(10_000, 10_000)),
            HashSet::new(),
            HashSet::from([ch_handle]),
            HashMap::new(),
        );
        let sub_node = make_node_with_protocol(
            Some(energy),
            HashSet::from([ch_handle]),
            HashSet::new(),
            HashMap::new(),
        );
        let channel = types::Channel {
            link: Link::default(),
            r#type: ChannelType::new_internal(),
            subscribers: HashSet::from([1]),
            publishers: HashSet::from([0]),
        };
        let handles = vec![(1u32, 0usize, 0usize), (2u32, 1usize, 0usize)];
        let (mut router, _rx) = make_router(vec![pub_node, sub_node], vec![channel], handles);

        // Queue a message and deliver it
        router.queue_message(0, 0, vec![0xAB]).unwrap();
        router.step().unwrap();

        // Subscriber mailbox should have the message
        assert!(
            !router.mailboxes[1].is_empty(),
            "Mailbox should have message before remap"
        );

        // Remap subscriber PID 2 → 3 (simulates respawn)
        router.apply_pid_remaps(&[(2, 3)]);

        // Mailbox for the remapped handle should be cleared
        assert!(
            router.mailboxes[1].is_empty(),
            "Mailbox should be cleared after remap"
        );
        // Publisher mailbox (not remapped) should still have its message
        assert!(
            !router.mailboxes[0].is_empty(),
            "Publisher mailbox should be untouched by remap"
        );
    }

    // -----------------------------------------------------------------------
    // Test: apply_pid_remaps does not affect unrelated handles
    // -----------------------------------------------------------------------
    #[test]
    fn test_pid_remap_unrelated_handles_untouched() {
        let ch_handle: usize = 0;
        let node_a = make_node_with_protocol(
            Some(basic_energy(5000, 10_000)),
            HashSet::from([ch_handle]),
            HashSet::new(),
            HashMap::new(),
        );
        let node_b = make_node_with_protocol(
            Some(basic_energy(5000, 10_000)),
            HashSet::from([ch_handle]),
            HashSet::new(),
            HashMap::new(),
        );
        let channel = types::Channel {
            link: Link::default(),
            r#type: ChannelType::new_internal(),
            subscribers: HashSet::from([0, 1]),
            publishers: HashSet::new(),
        };
        let handles = vec![(10u32, 0usize, 0usize), (20u32, 1usize, 0usize)];
        let (mut router, _rx) = make_router(vec![node_a, node_b], vec![channel], handles);

        // Only remap PID 10 → 11
        router.apply_pid_remaps(&[(10, 11)]);

        // Node A's handle remapped
        assert_eq!(router.channels.handles[0].0, 11);
        // Node B's handle NOT remapped
        assert_eq!(router.channels.handles[1].0, 20);
    }

    // -----------------------------------------------------------------------
    // Test: apply_pid_remaps pushes pairs to shared FUSE queue
    // -----------------------------------------------------------------------
    #[test]
    fn test_pid_remap_pushes_to_shared_queue() {
        let (mut router, _rx) = make_single_node_router(basic_energy(1000, 10_000));

        let queue = router.pending_remaps.clone();
        router.apply_pid_remaps(&[(5, 6), (7, 8)]);

        let pairs = queue.lock().unwrap();
        assert_eq!(*pairs, vec![(5, 6), (7, 8)]);
    }

    // -----------------------------------------------------------------------
    // Test: multiple pid remaps applied in batch
    // -----------------------------------------------------------------------
    #[test]
    fn test_pid_remap_batch() {
        let ch_handle: usize = 0;
        let node = make_node_with_protocol(
            Some(basic_energy(5000, 10_000)),
            HashSet::from([ch_handle]),
            HashSet::from([ch_handle]),
            HashMap::new(),
        );
        let channel = types::Channel {
            link: Link::default(),
            r#type: ChannelType::new_internal(),
            subscribers: HashSet::from([0]),
            publishers: HashSet::from([0]),
        };
        // Two handles with different PIDs for the same node (two protocols)
        let handles = vec![(100u32, 0usize, 0usize), (101u32, 0usize, 0usize)];
        let (mut router, _rx) = make_router(vec![node], vec![channel], handles);

        // Batch remap both
        router.apply_pid_remaps(&[(100, 200), (101, 201)]);

        assert_eq!(router.channels.handles[0].0, 200);
        assert_eq!(router.channels.handles[1].0, 201);
        assert!(router.fuse_mapping.contains_key(&(200, "ch_0".to_string())));
        assert!(router.fuse_mapping.contains_key(&(201, "ch_0".to_string())));
    }

    // -----------------------------------------------------------------------
    // Test: remap + subsequent message delivery works with new PID
    // -----------------------------------------------------------------------
    #[test]
    fn test_pid_remap_then_deliver_message() {
        let ch_handle: usize = 0;

        let pub_node = make_node_with_protocol(
            Some(basic_energy(10_000, 10_000)),
            HashSet::new(),
            HashSet::from([ch_handle]),
            HashMap::new(),
        );
        let sub_node = make_node_with_protocol(
            Some(basic_energy(10_000, 10_000)),
            HashSet::from([ch_handle]),
            HashSet::new(),
            HashMap::new(),
        );
        let channel = types::Channel {
            link: Link::default(),
            r#type: ChannelType::new_internal(),
            subscribers: HashSet::from([1]),
            publishers: HashSet::from([0]),
        };
        let handles = vec![(1u32, 0usize, 0usize), (2u32, 1usize, 0usize)];
        let (mut router, _rx) = make_router(vec![pub_node, sub_node], vec![channel], handles);

        // Remap subscriber's PID before any message delivery
        router.apply_pid_remaps(&[(2, 42)]);

        // Queue and deliver a message — should work with new PID in handle
        router.queue_message(0, 0, vec![0xCD]).unwrap();
        router.step().unwrap();

        // Subscriber mailbox should have received the message
        assert_eq!(router.mailboxes[1].len(), 1);
    }

    // -----------------------------------------------------------------------
    // Test: piecewise linear evaluation — interpolation
    // -----------------------------------------------------------------------
    #[test]
    fn test_piecewise_linear_interpolation() {
        // Schedule: 0us → 0 nj/ts, 1000us → 100 nj/ts
        let flow = PowerFlowState::PiecewiseLinear {
            breakpoints: vec![(0, 0), (1000, 100)],
            repeat_us: None,
        };
        assert_eq!(flow.nj_per_timestep(0), 0);
        assert_eq!(flow.nj_per_timestep(500), 50);
        assert_eq!(flow.nj_per_timestep(1000), 100);
        // Beyond last breakpoint: clamp to last value
        assert_eq!(flow.nj_per_timestep(2000), 100);
    }

    // -----------------------------------------------------------------------
    // Test: piecewise linear evaluation — repeat wraparound
    // -----------------------------------------------------------------------
    #[test]
    fn test_piecewise_linear_repeat() {
        // Schedule: 0us → 0 nj/ts, 500us → 100 nj/ts, 1000us → 0 nj/ts
        // Repeats every 1000us
        let flow = PowerFlowState::PiecewiseLinear {
            breakpoints: vec![(0, 0), (500, 100), (1000, 0)],
            repeat_us: Some(1000),
        };
        assert_eq!(flow.nj_per_timestep(250), 50);
        assert_eq!(flow.nj_per_timestep(500), 100);
        assert_eq!(flow.nj_per_timestep(750), 50);
        // After one full cycle, should wrap around
        assert_eq!(flow.nj_per_timestep(1250), 50);
        assert_eq!(flow.nj_per_timestep(1500), 100);
    }

    // -----------------------------------------------------------------------
    // Test: piecewise linear — boundary values
    // -----------------------------------------------------------------------
    #[test]
    fn test_piecewise_linear_boundary() {
        let flow = PowerFlowState::PiecewiseLinear {
            breakpoints: vec![(100, 10), (200, 20), (300, 30)],
            repeat_us: None,
        };
        // Before first breakpoint
        assert_eq!(flow.nj_per_timestep(0), 10);
        // Exactly on breakpoints
        assert_eq!(flow.nj_per_timestep(100), 10);
        assert_eq!(flow.nj_per_timestep(200), 20);
        assert_eq!(flow.nj_per_timestep(300), 30);
        // After last breakpoint
        assert_eq!(flow.nj_per_timestep(500), 30);
    }

    // -----------------------------------------------------------------------
    // Test: multiple sources + sinks combining correctly
    // -----------------------------------------------------------------------
    #[test]
    fn test_multiple_sources_and_sinks() {
        let mut energy = basic_energy(1000, 100_000);
        energy.power_sources = vec![
            (
                "solar".into(),
                PowerFlowState::Constant { nj_per_ts: 100 },
            ),
            (
                "charger".into(),
                PowerFlowState::Constant { nj_per_ts: 50 },
            ),
        ];
        energy.power_sinks = vec![
            ("mcu".into(), PowerFlowState::Constant { nj_per_ts: 30 }),
            ("radio".into(), PowerFlowState::Constant { nj_per_ts: 20 }),
        ];
        let (mut router, _rx) = make_single_node_router(energy);

        router.step().unwrap();
        let charge = router.channels.nodes[0].energy.as_ref().unwrap().charge_nj;
        // 1000 + (100 + 50) - (30 + 20) = 1000 + 150 - 50 = 1100
        assert_eq!(charge, 1100);
    }

    // -----------------------------------------------------------------------
    // Test: dead node — sinks keep charge at 0 via saturating_sub
    // -----------------------------------------------------------------------
    #[test]
    fn test_dead_node_sinks_keep_zero() {
        let mut energy = basic_energy(0, 10_000);
        energy.power_sinks = vec![(
            "mcu".into(),
            PowerFlowState::Constant { nj_per_ts: 100 },
        )];
        energy.is_dead = true;
        let (mut router, _rx) = make_single_node_router(energy);

        router.step().unwrap();
        let charge = router.channels.nodes[0].energy.as_ref().unwrap().charge_nj;
        // 0 + 0 (no sources) - 100 (sink, saturates to 0) = 0
        assert_eq!(charge, 0, "Sink on dead node should saturate to 0");
    }

    // -----------------------------------------------------------------------
    // Test: control file read/write round-trip
    // -----------------------------------------------------------------------
    #[test]
    fn test_power_flows_control_file_roundtrip() {
        let energy = basic_energy(5000, 10_000);
        let node =
            make_node_with_protocol(Some(energy), HashSet::new(), HashSet::new(), HashMap::new());

        let (tx, rx) = mpsc::channel();
        let handles = vec![(1u32, 0usize, 0usize)];
        let resolved = ResolvedChannels {
            nodes: vec![node],
            node_names: vec!["node_0".to_string()],
            channels: vec![types::Channel {
                link: Link::default(),
                r#type: ChannelType::new_internal(),
                subscribers: HashSet::new(),
                publishers: HashSet::new(),
            }],
            channel_names: vec!["ctl.power_flows".to_string()],
            handles: handles.clone(),
        };
        let fuse_mapping = resolved.make_fuse_mapping();
        let routes = RoutingTable::new(&resolved);
        let mut router = RoutingServer {
            timestep: 1,
            ts_config: test_ts_config(),
            channels: resolved,
            routes,
            queued: BinaryHeap::new(),
            fuse_mapping,
            mailboxes: vec![VecDeque::new(); handles.len()],
            rng: StdRng::seed_from_u64(42),
            tx,
            newly_depleted: Vec::new(),
            newly_recovered: Vec::new(),
            pending_remaps: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
        };

        // Write a source and a sink via control file (nj/ts passthrough)
        let write_msg = fuse::Message {
            id: (1, "ctl.power_flows".to_string()),
            data: b"source solar 200 nj/ts\nsink mcu 50 nj/ts\n".to_vec(),
        };
        router.write_control_file(0, write_msg).unwrap();

        // Verify they were added
        let e = router.channels.nodes[0].energy.as_ref().unwrap();
        assert_eq!(e.power_sources.len(), 1);
        assert_eq!(e.power_sources[0].0, "solar");
        assert_eq!(e.power_sources[0].1.nj_per_timestep(0), 200);
        assert_eq!(e.power_sinks.len(), 1);
        assert_eq!(e.power_sinks[0].0, "mcu");
        assert_eq!(e.power_sinks[0].1.nj_per_timestep(0), 50);

        // Read the flows back
        let read_msg = fuse::Message {
            id: (1, "ctl.power_flows".to_string()),
            data: vec![],
        };
        router.read_control_file(0, read_msg).unwrap();

        let response = rx.recv().unwrap();
        let data = match response {
            fuse::KernelMessage::Exclusive(msg) => String::from_utf8(msg.data).unwrap(),
            _ => panic!("Expected Exclusive message"),
        };
        assert!(data.contains("source solar 200 nj/ts"));
        assert!(data.contains("sink mcu 50 nj/ts"));

        // Step and verify sources/sinks are applied
        router.step().unwrap();
        let charge = router.channels.nodes[0].energy.as_ref().unwrap().charge_nj;
        // 5000 + 200 - 50 = 5150
        assert_eq!(charge, 5150);

        // Remove the sink via control file
        let remove_msg = fuse::Message {
            id: (1, "ctl.power_flows".to_string()),
            data: b"remove mcu\n".to_vec(),
        };
        router.write_control_file(0, remove_msg).unwrap();
        assert_eq!(
            router.channels.nodes[0]
                .energy
                .as_ref()
                .unwrap()
                .power_sinks
                .len(),
            0
        );
    }

    // -----------------------------------------------------------------------
    // Test: control file write with mw/s units
    // -----------------------------------------------------------------------
    #[test]
    fn test_power_flows_control_file_unit_conversion() {
        let energy = basic_energy(0, 100_000_000);
        let node =
            make_node_with_protocol(Some(energy), HashSet::new(), HashSet::new(), HashMap::new());

        let (tx, _rx) = mpsc::channel();
        let handles = vec![(1u32, 0usize, 0usize)];
        let resolved = ResolvedChannels {
            nodes: vec![node],
            node_names: vec!["node_0".to_string()],
            channels: vec![types::Channel {
                link: Link::default(),
                r#type: ChannelType::new_internal(),
                subscribers: HashSet::new(),
                publishers: HashSet::new(),
            }],
            channel_names: vec!["ctl.power_flows".to_string()],
            handles: handles.clone(),
        };
        let fuse_mapping = resolved.make_fuse_mapping();
        let routes = RoutingTable::new(&resolved);
        let mut router = RoutingServer {
            timestep: 1,
            ts_config: test_ts_config(),
            channels: resolved,
            routes,
            queued: BinaryHeap::new(),
            fuse_mapping,
            mailboxes: vec![VecDeque::new(); handles.len()],
            rng: StdRng::seed_from_u64(42),
            tx,
            newly_depleted: Vec::new(),
            newly_recovered: Vec::new(),
            pending_remaps: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
        };

        // Write "source solar 100 mw/s" — 100 mW per second with 1ms timestep
        // nj/ts = 100 * 1_000_000 * 1_000_000 / 1_000_000_000 = 100_000
        let write_msg = fuse::Message {
            id: (1, "ctl.power_flows".to_string()),
            data: b"source solar 100 mw/s\n".to_vec(),
        };
        router.write_control_file(0, write_msg).unwrap();

        let e = router.channels.nodes[0].energy.as_ref().unwrap();
        assert_eq!(e.power_sources[0].1.nj_per_timestep(0), 100_000);
    }
}
