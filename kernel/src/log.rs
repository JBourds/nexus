use bincode::{Decode, Encode, config, encode_into_std_write};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tracing::field::Visit;
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer};

use crate::types::{ChannelHandle, NodeHandle};

/// A message sent or received on a channel.
#[derive(Decode, Encode, Serialize, Deserialize, Debug, Default, PartialEq)]
pub struct MessageRecord {
    pub timestep: u64,
    /// `true` = transmitted (TX), `false` = received (RX).
    pub tx: bool,
    pub node: NodeHandle,
    pub channel: ChannelHandle,
    pub data: Vec<u8>,
}

/// A node position update (direct write or motion-pattern step).
#[derive(Decode, Encode, Serialize, Deserialize, Debug, Default, PartialEq)]
pub struct MovementRecord {
    pub timestep: u64,
    pub node: NodeHandle,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub az: f64,
    pub el: f64,
    pub roll: f64,
}

/// A snapshot of a node's battery charge at a given timestep.
#[derive(Decode, Encode, Serialize, Deserialize, Debug, Default, PartialEq)]
pub struct BatteryRecord {
    pub timestep: u64,
    pub node: NodeHandle,
    pub charge_nj: u64,
}

/// Top-level log record. Encoded as a bincode enum (u32 variant tag + payload).
#[derive(Decode, Encode, Serialize, Deserialize, Debug, PartialEq)]
pub enum LogRecord {
    Message(MessageRecord),
    Movement(MovementRecord),
    Battery(BatteryRecord),
}

// Visitors

#[derive(Debug, Default)]
struct MessageVisitor {
    timestep: u64,
    tx: bool,
    node: usize,
    channel: usize,
    data: Vec<u8>,
}

impl Visit for MessageVisitor {
    fn record_debug(&mut self, _: &tracing::field::Field, _: &dyn std::fmt::Debug) {}

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        match field.name() {
            "timestep" => self.timestep = value,
            "channel" => self.channel = value as usize,
            "node" => self.node = value as usize,
            _ => {}
        }
    }

    fn record_i64(&mut self, _field: &tracing::field::Field, _value: i64) {}

    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        if field.name() == "tx" {
            self.tx = value;
        }
    }

    fn record_bytes(&mut self, field: &tracing::field::Field, value: &[u8]) {
        if field.name() == "data" {
            self.data = value.to_vec();
        }
    }
}

impl From<MessageVisitor> for LogRecord {
    fn from(v: MessageVisitor) -> Self {
        LogRecord::Message(MessageRecord {
            timestep: v.timestep,
            tx: v.tx,
            node: v.node,
            channel: v.channel,
            data: v.data,
        })
    }
}

#[derive(Debug, Default)]
struct MovementVisitor {
    timestep: u64,
    node: usize,
    x: f64,
    y: f64,
    z: f64,
    az: f64,
    el: f64,
    roll: f64,
}

impl Visit for MovementVisitor {
    fn record_debug(&mut self, _: &tracing::field::Field, _: &dyn std::fmt::Debug) {}

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        match field.name() {
            "timestep" => self.timestep = value,
            "node" => self.node = value as usize,
            _ => {}
        }
    }

    fn record_f64(&mut self, field: &tracing::field::Field, value: f64) {
        match field.name() {
            "x" => self.x = value,
            "y" => self.y = value,
            "z" => self.z = value,
            "az" => self.az = value,
            "el" => self.el = value,
            "roll" => self.roll = value,
            _ => {}
        }
    }
}

impl From<MovementVisitor> for LogRecord {
    fn from(v: MovementVisitor) -> Self {
        LogRecord::Movement(MovementRecord {
            timestep: v.timestep,
            node: v.node,
            x: v.x,
            y: v.y,
            z: v.z,
            az: v.az,
            el: v.el,
            roll: v.roll,
        })
    }
}

#[derive(Debug, Default)]
struct BatteryVisitor {
    timestep: u64,
    node: usize,
    charge_nj: u64,
}

impl Visit for BatteryVisitor {
    fn record_debug(&mut self, _: &tracing::field::Field, _: &dyn std::fmt::Debug) {}

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        match field.name() {
            "timestep" => self.timestep = value,
            "node" => self.node = value as usize,
            "charge_nj" => self.charge_nj = value,
            _ => {}
        }
    }
}

impl From<BatteryVisitor> for LogRecord {
    fn from(v: BatteryVisitor) -> Self {
        LogRecord::Battery(BatteryRecord {
            timestep: v.timestep,
            node: v.node,
            charge_nj: v.charge_nj,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(record: &LogRecord) -> LogRecord {
        let cfg = config::standard();
        let bytes = bincode::encode_to_vec(record, cfg).unwrap();
        let (decoded, _): (LogRecord, _) = bincode::decode_from_slice(&bytes, cfg).unwrap();
        decoded
    }

    #[test]
    fn message_record_roundtrip() {
        let original = LogRecord::Message(MessageRecord {
            timestep: 42,
            tx: true,
            node: 3,
            channel: 7,
            data: vec![0xDE, 0xAD, 0xBE, 0xEF],
        });
        assert_eq!(roundtrip(&original), original);
    }

    #[test]
    fn message_record_rx_roundtrip() {
        let original = LogRecord::Message(MessageRecord {
            timestep: 100,
            tx: false,
            node: 0,
            channel: 1,
            data: vec![],
        });
        assert_eq!(roundtrip(&original), original);
    }

    #[test]
    fn movement_record_roundtrip() {
        let original = LogRecord::Movement(MovementRecord {
            timestep: 999,
            node: 2,
            x: 1.5,
            y: -3.7,
            z: 0.0,
            az: 45.0,
            el: 10.0,
            roll: 0.0,
        });
        assert_eq!(roundtrip(&original), original);
    }

    #[test]
    fn battery_record_roundtrip() {
        let original = LogRecord::Battery(BatteryRecord {
            timestep: 500,
            node: 1,
            charge_nj: 123456789,
        });
        assert_eq!(roundtrip(&original), original);
    }

    #[test]
    fn mixed_records_sequential_roundtrip() {
        let records = vec![
            LogRecord::Message(MessageRecord {
                timestep: 1,
                tx: true,
                node: 0,
                channel: 0,
                data: vec![1, 2, 3],
            }),
            LogRecord::Movement(MovementRecord {
                timestep: 2,
                node: 0,
                x: 10.0,
                y: 20.0,
                z: 0.0,
                az: 0.0,
                el: 0.0,
                roll: 0.0,
            }),
            LogRecord::Battery(BatteryRecord {
                timestep: 2,
                node: 0,
                charge_nj: 50000,
            }),
            LogRecord::Message(MessageRecord {
                timestep: 3,
                tx: false,
                node: 1,
                channel: 0,
                data: vec![1, 2, 3],
            }),
        ];

        let cfg = config::standard();
        let mut buf: Vec<u8> = Vec::new();
        for r in &records {
            bincode::encode_into_std_write(r, &mut buf, cfg).unwrap();
        }

        let mut reader = std::io::BufReader::new(std::io::Cursor::new(buf));
        let mut decoded = Vec::new();
        loop {
            match bincode::decode_from_reader::<LogRecord, _, _>(&mut reader, cfg) {
                Ok(r) => decoded.push(r),
                Err(bincode::error::DecodeError::Io { inner, .. })
                    if inner.kind() == std::io::ErrorKind::UnexpectedEof =>
                {
                    break;
                }
                Err(e) => panic!("unexpected decode error: {e}"),
            }
        }
        assert_eq!(decoded, records);
    }
}

// Layer

pub struct BinaryLogLayer(Option<Mutex<BufWriter<File>>>);

impl BinaryLogLayer {
    pub fn new(file: Option<File>) -> Self {
        Self(file.map(|f| Mutex::new(BufWriter::new(f))))
    }
}

impl<S: Subscriber> Layer<S> for BinaryLogLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let Some(ref lock) = self.0 else {
            return;
        };
        let record: LogRecord = match event.metadata().target() {
            "tx" | "rx" => {
                let mut visitor = MessageVisitor::default();
                event.record(&mut visitor);
                visitor.into()
            }
            "movement" => {
                let mut visitor = MovementVisitor::default();
                event.record(&mut visitor);
                visitor.into()
            }
            "battery" => {
                let mut visitor = BatteryVisitor::default();
                event.record(&mut visitor);
                visitor.into()
            }
            _ => return,
        };
        let cfg = config::standard();
        let mut file = lock.lock().unwrap();
        encode_into_std_write(record, &mut *file, cfg).unwrap();
        file.flush().unwrap();
    }
}
