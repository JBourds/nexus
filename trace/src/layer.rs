use std::path::Path;
use std::sync::Mutex;

use tracing::field::Visit;
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer};

use crate::format::{TraceEvent, TraceHeader, TraceRecord};
use crate::writer::TraceWriter;

/// Generate a `Visit` impl with a `record_u64` method that matches field names
/// to struct fields. Each arm is specified as `"field_name" => self.field: type`.
/// A no-op `record_debug` is always included. Additional visitor methods (e.g.
/// `record_str`, `record_bool`, `record_bytes`, `record_f64`) can be provided
/// in an `extras` block.
macro_rules! impl_visitor {
    (
        $ty:ty,
        u64_fields: { $( $name:literal => $field:ident : $cast:ty ),* $(,)? }
        $(, extras: { $($extra:tt)* })?
    ) => {
        impl Visit for $ty {
            fn record_debug(&mut self, _: &tracing::field::Field, _: &dyn std::fmt::Debug) {}

            fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
                match field.name() {
                    $( $name => self.$field = value as $cast, )*
                    _ => {}
                }
            }

            $( $($extra)* )?
        }
    };
}

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

impl_visitor!(TraceVisitor,
    u64_fields: {
        "timestep" => timestep: u64,
        "channel" => channel: u32,
        "node" => node: u32,
    },
    extras: {
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
);

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

impl_visitor!(DropVisitor,
    u64_fields: {
        "timestep" => timestep: u64,
        "channel" => channel: u32,
        "node" => node: u32,
    },
    extras: {
        fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
            if field.name() == "reason" {
                self.reason = value.to_string();
            }
        }
    }
);

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

impl_visitor!(BatteryVisitor,
    u64_fields: {
        "timestep" => timestep: u64,
        "node" => node: u32,
        "charge_nj" => charge_nj: u64,
    }
);

#[derive(Debug, Default)]
struct MovementVisitor {
    timestep: u64,
    node: u32,
    x: f64,
    y: f64,
    z: f64,
}

impl_visitor!(MovementVisitor,
    u64_fields: {
        "timestep" => timestep: u64,
        "node" => node: u32,
    },
    extras: {
        fn record_f64(&mut self, field: &tracing::field::Field, value: f64) {
            match field.name() {
                "x" => self.x = value,
                "y" => self.y = value,
                "z" => self.z = value,
                _ => {}
            }
        }
    }
);

#[derive(Debug, Default)]
struct MotionVisitor {
    timestep: u64,
    node: u32,
    spec: String,
}

impl_visitor!(MotionVisitor,
    u64_fields: {
        "timestep" => timestep: u64,
        "node" => node: u32,
    },
    extras: {
        fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
            if field.name() == "spec" {
                self.spec = value.to_string();
            }
        }
    }
);
