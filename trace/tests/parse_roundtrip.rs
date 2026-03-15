use trace::display;
use trace::format::*;
use trace::reader::TraceReader;
use trace::writer::TraceWriter;

fn test_header() -> TraceHeader {
    TraceHeader {
        node_names: vec!["alice".into(), "bob".into(), "carol".into()],
        channel_names: vec!["lora0".into(), "wired1".into()],
        timestep_count: 1000,
        node_max_nj: vec![Some(5_000_000), Some(5_000_000), None],
    }
}

fn all_event_records() -> Vec<TraceRecord> {
    vec![
        TraceRecord {
            timestep: 1,
            event: TraceEvent::MessageSent {
                src_node: 0,
                channel: 0,
                data: vec![0x48, 0x65, 0x6c, 0x6c, 0x6f],
            },
        },
        TraceRecord {
            timestep: 1,
            event: TraceEvent::MessageRecv {
                dst_node: 1,
                channel: 0,
                data: vec![0x48, 0x65, 0x6c, 0x6c, 0x6f],
            },
        },
        TraceRecord {
            timestep: 1,
            event: TraceEvent::MessageDropped {
                src_node: 2,
                channel: 0,
                reason: DropReason::BelowSensitivity,
            },
        },
        TraceRecord {
            timestep: 10,
            event: TraceEvent::PositionUpdate {
                node: 0,
                x: 1.5,
                y: 2.3,
                z: 0.0,
            },
        },
        TraceRecord {
            timestep: 10,
            event: TraceEvent::EnergyUpdate {
                node: 0,
                energy_nj: 4_500_000,
            },
        },
        TraceRecord {
            timestep: 50,
            event: TraceEvent::MotionUpdate {
                node: 1,
                spec: "velocity 1.0 0.0 0.0".into(),
            },
        },
    ]
}

#[test]
fn roundtrip_write_read_format() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.nxs");
    let header = test_header();
    let records = all_event_records();

    // Write
    {
        let mut writer = TraceWriter::create(&path, &header).unwrap();
        for rec in &records {
            writer.write_record(rec).unwrap();
        }
        writer.flush().unwrap();
    }

    // Read back and verify
    let mut reader = TraceReader::open(&path).unwrap();
    assert_eq!(reader.header.node_names, header.node_names);
    assert_eq!(reader.header.channel_names, header.channel_names);
    assert_eq!(reader.header.timestep_count, header.timestep_count);

    let mut read_records = Vec::new();
    while let Some(rec) = reader.next_record().unwrap() {
        read_records.push(rec);
    }
    assert_eq!(records.len(), read_records.len());

    for (orig, read) in records.iter().zip(read_records.iter()) {
        assert_eq!(orig.timestep, read.timestep);
        assert_eq!(orig.event, read.event);
    }
}

#[test]
fn format_text_all_event_types() {
    let header = test_header();
    let records = all_event_records();

    let lines: Vec<String> = records
        .iter()
        .map(|r| display::format_record(&header, r))
        .collect();

    // TX
    assert!(lines[0].contains("TX"));
    assert!(lines[0].contains("alice"));
    assert!(lines[0].contains("lora0"));
    assert!(lines[0].contains("48656c6c6f"));

    // RX
    assert!(lines[1].contains("RX"));
    assert!(lines[1].contains("bob"));
    assert!(lines[1].contains("<-"));

    // DROP
    assert!(lines[2].contains("DROP"));
    assert!(lines[2].contains("carol"));
    assert!(lines[2].contains("BelowSensitivity"));

    // POS
    assert!(lines[3].contains("POS"));
    assert!(lines[3].contains("1.50"));
    assert!(lines[3].contains("2.30"));

    // NRG
    assert!(lines[4].contains("NRG"));
    assert!(lines[4].contains("4500000 nJ"));

    // MOT
    assert!(lines[5].contains("MOT"));
    assert!(lines[5].contains("velocity 1.0 0.0 0.0"));
}

#[test]
fn json_output_all_event_types() {
    let header = test_header();
    let records = all_event_records();

    for rec in &records {
        let val = display::record_to_json(&header, rec);
        // All records should have timestep and event fields
        assert!(val.get("timestep").is_some());
        assert!(val.get("event").is_some());
        // Verify the JSON is valid by serializing
        let json_str = serde_json::to_string(&val).unwrap();
        let _: serde_json::Value = serde_json::from_str(&json_str).unwrap();
    }

    // Verify specific fields
    let tx_json = display::record_to_json(&header, &records[0]);
    assert_eq!("MessageSent", tx_json["event"]);
    assert_eq!("alice", tx_json["node"]);
    assert_eq!("lora0", tx_json["channel"]);
    assert_eq!(5, tx_json["data_len"]);

    let drop_json = display::record_to_json(&header, &records[2]);
    assert_eq!("MessageDropped", drop_json["event"]);
    assert_eq!("BelowSensitivity", drop_json["reason"]);

    let pos_json = display::record_to_json(&header, &records[3]);
    assert_eq!("PositionUpdate", pos_json["event"]);
    assert_eq!(1.5, pos_json["x"]);

    let energy_json = display::record_to_json(&header, &records[4]);
    assert_eq!("EnergyUpdate", energy_json["event"]);
    assert_eq!(4_500_000, energy_json["energy_nj"]);
}

#[test]
fn header_summary_format() {
    let header = test_header();
    let summary = display::format_header_summary(&header, "test.nxs");
    assert!(summary.contains("=== Trace: test.nxs ==="));
    assert!(summary.contains("Nodes (3)"));
    assert!(summary.contains("alice, bob, carol"));
    assert!(summary.contains("Channels (2)"));
    assert!(summary.contains("lora0, wired1"));
    assert!(summary.contains("Timesteps: 1000"));
    assert!(summary.contains("alice (max 5000000 nJ)"));
    assert!(summary.contains("carol (none)"));
}
