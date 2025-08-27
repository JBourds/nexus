use bincode::{Decode, Encode, config, encode_into_std_write};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tracing::field::Visit;
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer};

use crate::types::{ChannelHandle, NodeHandle};

#[derive(Decode, Encode, Serialize, Deserialize, Debug, Default, PartialEq)]
pub struct BinaryLogRecord {
    pub timestep: u64,
    pub is_publisher: bool,
    pub node: NodeHandle,
    pub channel: ChannelHandle,
    pub data: Vec<u8>,
}

#[derive(Debug, Default, PartialEq)]
struct LogVisitor {
    record: BinaryLogRecord,
}

impl Visit for LogVisitor {
    fn record_debug(&mut self, _: &tracing::field::Field, _: &dyn std::fmt::Debug) {}

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        match field.name() {
            "timestep" => {
                self.record.timestep = value;
            }
            "channel" => {
                self.record.channel = value as usize;
            }
            "node" => {
                self.record.node = value as usize;
            }
            _ => {}
        }
    }

    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        if field.name() == "tx" {
            self.record.is_publisher = value;
        }
    }

    fn record_bytes(&mut self, field: &tracing::field::Field, value: &[u8]) {
        if field.name() == "data" {
            self.record.data = value.to_vec();
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
        let config = config::standard();
        let mut file = lock.lock().unwrap();
        encode_into_std_write(visitor.record, &mut *file, config).unwrap();
        file.flush().unwrap();
    }
}
