use bincode::{Encode, config, encode_into_std_write};
use std::fs::File;
use std::io::Write;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tracing::field::Visit;
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer};

use crate::types::ChannelHandle;

#[derive(Encode, Serialize, Deserialize, Debug, Default, PartialEq)]
struct BinaryLogRecord {
    timestep: u64,
    is_outbound: bool,
    pid: fuse::PID,
    channel: ChannelHandle,
    data: Vec<u8>,
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
            "pid" => {
                self.record.pid = value as u32;
            }
            _ => {}
        }
    }

    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        if field.name() == "tx" {
            self.record.is_outbound = value;
        }
    }

    fn record_bytes(&mut self, field: &tracing::field::Field, value: &[u8]) {
        if field.name() == "data" {
            self.record.data = value.to_vec();
        }
    }
}

pub struct BinaryLogLayer(Option<Mutex<File>>);
impl BinaryLogLayer {
    pub fn new(file: Option<File>) -> Self {
        Self(file.map(Mutex::new))
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
