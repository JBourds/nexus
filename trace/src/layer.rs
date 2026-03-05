use std::path::Path;
use std::sync::Mutex;

use tracing::field::Visit;
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer};

use crate::format::{TraceEvent, TraceHeader, TraceRecord};
use crate::writer::TraceWriter;

/// A `tracing_subscriber::Layer` that captures `tx` and `rx` target events
/// and writes them as `TraceRecord`s to a unified trace file.
pub struct TraceLayer {
    writer: Mutex<TraceWriter>,
}

impl TraceLayer {
    pub fn new(path: impl AsRef<Path>, header: &TraceHeader) -> std::io::Result<Self> {
        let writer = TraceWriter::create(path, header)?;
        Ok(Self {
            writer: Mutex::new(writer),
        })
    }
}

#[derive(Debug, Default)]
struct TraceVisitor {
    timestep: u64,
    channel: u32,
    node: u32,
    is_tx: bool,
    data: Vec<u8>,
}

impl Visit for TraceVisitor {
    fn record_debug(&mut self, _: &tracing::field::Field, _: &dyn std::fmt::Debug) {}

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        match field.name() {
            "timestep" => self.timestep = value,
            "channel" => self.channel = value as u32,
            "node" => self.node = value as u32,
            _ => {}
        }
    }

    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        if field.name() == "tx" {
            self.is_tx = value;
        }
    }

    fn record_bytes(&mut self, field: &tracing::field::Field, value: &[u8]) {
        if field.name() == "data" {
            self.data = value.to_vec();
        }
    }
}

impl<S: Subscriber> Layer<S> for TraceLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let target = event.metadata().target();

        // Handle drop events
        if target == "drop" {
            let mut visitor = DropVisitor::default();
            event.record(&mut visitor);
            let record = TraceRecord {
                timestep: visitor.timestep,
                event: visitor.into_event(),
            };
            let mut writer = self.writer.lock().unwrap();
            let _ = writer.write_record(&record);
            return;
        }

        // Only handle tx/rx events
        if !matches!(target, "tx" | "rx") {
            return;
        }

        let mut visitor = TraceVisitor::default();
        event.record(&mut visitor);

        let trace_event = if visitor.is_tx {
            TraceEvent::MessageSent {
                src_node: visitor.node,
                channel: visitor.channel,
                data: visitor.data,
            }
        } else {
            TraceEvent::MessageRecv {
                dst_node: visitor.node,
                channel: visitor.channel,
                data: visitor.data,
            }
        };

        let record = TraceRecord {
            timestep: visitor.timestep,
            event: trace_event,
        };

        let mut writer = self.writer.lock().unwrap();
        let _ = writer.write_record(&record);
    }
}

#[derive(Debug, Default)]
struct DropVisitor {
    timestep: u64,
    node: u32,
    channel: u32,
    reason: String,
}

impl Visit for DropVisitor {
    fn record_debug(&mut self, _: &tracing::field::Field, _: &dyn std::fmt::Debug) {}

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        match field.name() {
            "timestep" => self.timestep = value,
            "channel" => self.channel = value as u32,
            "node" => self.node = value as u32,
            _ => {}
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "reason" {
            self.reason = value.to_string();
        }
    }
}

impl DropVisitor {
    fn into_event(self) -> TraceEvent {
        let reason = match self.reason.as_str() {
            "below_sensitivity" => crate::format::DropReason::BelowSensitivity,
            "packet_loss" => crate::format::DropReason::PacketLoss,
            "ttl_expired" => crate::format::DropReason::TtlExpired,
            "buffer_full" => crate::format::DropReason::BufferFull,
            _ => crate::format::DropReason::PacketLoss,
        };
        TraceEvent::MessageDropped {
            src_node: self.node,
            channel: self.channel,
            reason,
        }
    }
}
