use bincode::{Decode, Encode, config, encode_into_std_write};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tracing::field::Visit;
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer};

use crate::types::{ChannelHandle, NodeHandle};

#[derive(Decode, Encode, Serialize, Deserialize, Debug, PartialEq)]
pub enum LogRecord {
    /// A message transmitted or received on a channel.
    Message {
        timestep: u64,
        /// True if this node is the publisher (TX), false if subscriber (RX).
        is_publisher: bool,
        node: NodeHandle,
        channel: ChannelHandle,
        data: Vec<u8>,
    },
    /// A snapshot of a node's battery charge at a given timestep.
    Battery {
        timestep: u64,
        node: NodeHandle,
        charge_nj: u64,
    },
}

impl LogRecord {
    pub fn timestep(&self) -> u64 {
        match self {
            Self::Message { timestep, .. } | Self::Battery { timestep, .. } => *timestep,
        }
    }
}

#[derive(Debug, Default)]
struct LogVisitor {
    timestep: u64,
    node: usize,
    channel: Option<usize>,
    is_publisher: Option<bool>,
    data: Option<Vec<u8>>,
    charge_nj: Option<u64>,
}

impl Visit for LogVisitor {
    fn record_debug(&mut self, _: &tracing::field::Field, _: &dyn std::fmt::Debug) {}

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        match field.name() {
            "timestep" => self.timestep = value,
            "channel" => self.channel = Some(value as usize),
            "node" => self.node = value as usize,
            "charge_nj" => self.charge_nj = Some(value),
            _ => {}
        }
    }

    fn record_i64(&mut self, _field: &tracing::field::Field, _value: i64) {}

    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        if field.name() == "tx" {
            self.is_publisher = Some(value);
        }
    }

    fn record_bytes(&mut self, field: &tracing::field::Field, value: &[u8]) {
        if field.name() == "data" {
            self.data = Some(value.to_vec());
        }
    }
}

impl LogVisitor {
    fn into_record(self) -> Option<LogRecord> {
        if let (Some(channel), Some(is_publisher), Some(data)) =
            (self.channel, self.is_publisher, self.data)
        {
            Some(LogRecord::Message {
                timestep: self.timestep,
                is_publisher,
                node: self.node,
                channel,
                data,
            })
        } else if let Some(charge_nj) = self.charge_nj {
            Some(LogRecord::Battery {
                timestep: self.timestep,
                node: self.node,
                charge_nj,
            })
        } else {
            None
        }
    }
}

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
        let mut visitor = LogVisitor::default();
        event.record(&mut visitor);
        let Some(record) = visitor.into_record() else {
            return;
        };
        let config = config::standard();
        let mut file = lock.lock().unwrap();
        encode_into_std_write(record, &mut *file, config).unwrap();
        file.flush().unwrap();
    }
}
