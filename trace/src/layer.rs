use std::path::Path;
use std::sync::{Arc, Mutex};

use tracing::field::Visit;
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer};

use crate::format::{TraceEvent, TraceHeader, TraceRecord};
use crate::writer::TraceWriter;

/// A handle to the trace writer that flushes on drop.
///
/// The global tracing subscriber set by `.init()` is never dropped, so the
/// `TraceWriter`'s `BufWriter` would never be flushed. This handle keeps a
/// clone of the shared writer and flushes it when dropped, ensuring all
/// buffered trace records reach disk.
pub struct TraceHandle(Arc<Mutex<TraceWriter>>);

impl Drop for TraceHandle {
    fn drop(&mut self) {
        if let Ok(mut w) = self.0.lock() {
            let _ = w.flush();
        }
    }
}

/// A `tracing_subscriber::Layer` that captures `tx` and `rx` target events
/// and writes them as `TraceRecord`s to a unified trace file.
pub struct TraceLayer {
    writer: Arc<Mutex<TraceWriter>>,
}

impl TraceLayer {
    /// Create a new trace layer and a [`TraceHandle`] that must be held alive
    /// until the simulation ends. Dropping the handle flushes all buffered
    /// records to disk.
    pub fn new(path: impl AsRef<Path>, header: &TraceHeader) -> std::io::Result<(Self, TraceHandle)> {
        let writer = Arc::new(Mutex::new(TraceWriter::create(path, header)?));
        Ok((
            Self {
                writer: Arc::clone(&writer),
            },
            TraceHandle(writer),
        ))
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

        let record = match target {
            "drop" => {
                let mut visitor = DropVisitor::default();
                event.record(&mut visitor);
                TraceRecord {
                    timestep: visitor.timestep,
                    event: visitor.into_event(),
                }
            }
            "tx" | "rx" => {
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
                TraceRecord {
                    timestep: visitor.timestep,
                    event: trace_event,
                }
            }
            "battery" => {
                let mut visitor = BatteryVisitor::default();
                event.record(&mut visitor);
                TraceRecord {
                    timestep: visitor.timestep,
                    event: TraceEvent::EnergyUpdate {
                        node: visitor.node,
                        energy_nj: visitor.charge_nj,
                    },
                }
            }
            "movement" => {
                let mut visitor = MovementVisitor::default();
                event.record(&mut visitor);
                TraceRecord {
                    timestep: visitor.timestep,
                    event: TraceEvent::PositionUpdate {
                        node: visitor.node,
                        x: visitor.x,
                        y: visitor.y,
                        z: visitor.z,
                    },
                }
            }
            "motion" => {
                let mut visitor = MotionVisitor::default();
                event.record(&mut visitor);
                TraceRecord {
                    timestep: visitor.timestep,
                    event: TraceEvent::MotionUpdate {
                        node: visitor.node,
                        spec: visitor.spec,
                    },
                }
            }
            _ => return,
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

#[derive(Debug, Default)]
struct BatteryVisitor {
    timestep: u64,
    node: u32,
    charge_nj: u64,
}

impl Visit for BatteryVisitor {
    fn record_debug(&mut self, _: &tracing::field::Field, _: &dyn std::fmt::Debug) {}

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        match field.name() {
            "timestep" => self.timestep = value,
            "node" => self.node = value as u32,
            "charge_nj" => self.charge_nj = value,
            _ => {}
        }
    }
}

#[derive(Debug, Default)]
struct MovementVisitor {
    timestep: u64,
    node: u32,
    x: f64,
    y: f64,
    z: f64,
}

impl Visit for MovementVisitor {
    fn record_debug(&mut self, _: &tracing::field::Field, _: &dyn std::fmt::Debug) {}

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        match field.name() {
            "timestep" => self.timestep = value,
            "node" => self.node = value as u32,
            _ => {}
        }
    }

    fn record_f64(&mut self, field: &tracing::field::Field, value: f64) {
        match field.name() {
            "x" => self.x = value,
            "y" => self.y = value,
            "z" => self.z = value,
            _ => {}
        }
    }
}

#[derive(Debug, Default)]
struct MotionVisitor {
    timestep: u64,
    node: u32,
    spec: String,
}

impl Visit for MotionVisitor {
    fn record_debug(&mut self, _: &tracing::field::Field, _: &dyn std::fmt::Debug) {}

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        match field.name() {
            "timestep" => self.timestep = value,
            "node" => self.node = value as u32,
            _ => {}
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "spec" {
            self.spec = value.to_string();
        }
    }
}
