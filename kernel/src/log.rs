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

/// Top-level log record. Encoded as a bincode enum (u32 variant tag + payload).
#[derive(Decode, Encode, Serialize, Deserialize, Debug, PartialEq)]
pub enum LogRecord {
    Message(MessageRecord),
    Movement(MovementRecord),
}

// ── Visitors ──────────────────────────────────────────────────────────────────

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

// ── Layer ─────────────────────────────────────────────────────────────────────

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
            _ => return,
        };
        let cfg = config::standard();
        let mut file = lock.lock().unwrap();
        encode_into_std_write(record, &mut *file, cfg).unwrap();
        file.flush().unwrap();
    }
}
