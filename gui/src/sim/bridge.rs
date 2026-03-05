use crossbeam_channel::Sender;
use tracing::field::Visit;
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer};

use trace::format::{DropReason, TraceEvent, TraceRecord};

/// Events sent from the simulation bridge to the GUI.
#[derive(Debug, Clone)]
pub enum GuiEvent {
    Trace(TraceRecord),
    TimestepAdvanced(u64),
    SimulationComplete,
    SimulationError(String),
}

/// A `tracing_subscriber::Layer` that sends trace events to the GUI via a channel.
pub struct GuiBridgeLayer {
    tx: Sender<GuiEvent>,
}

impl GuiBridgeLayer {
    pub fn new(tx: Sender<GuiEvent>) -> Self {
        Self { tx }
    }
}

#[derive(Debug, Default)]
struct BridgeVisitor {
    timestep: u64,
    channel: u32,
    node: u32,
    is_tx: bool,
    data: Vec<u8>,
    reason: Option<String>,
}

impl Visit for BridgeVisitor {
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

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "reason" {
            self.reason = Some(value.to_string());
        }
    }
}

impl<S: Subscriber> Layer<S> for GuiBridgeLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let target = event.metadata().target();

        if target == "drop" {
            let mut visitor = BridgeVisitor::default();
            event.record(&mut visitor);
            let reason = match visitor.reason.as_deref() {
                Some("below_sensitivity") => DropReason::BelowSensitivity,
                Some("packet_loss") => DropReason::PacketLoss,
                Some("ttl_expired") => DropReason::TtlExpired,
                Some("buffer_full") => DropReason::BufferFull,
                _ => DropReason::PacketLoss,
            };
            let record = TraceRecord {
                timestep: visitor.timestep,
                event: TraceEvent::MessageDropped {
                    src_node: visitor.node,
                    channel: visitor.channel,
                    reason,
                },
            };
            let _ = self.tx.send(GuiEvent::Trace(record));
            return;
        }

        if !matches!(target, "tx" | "rx") {
            return;
        }

        let mut visitor = BridgeVisitor::default();
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
        let _ = self.tx.send(GuiEvent::Trace(record));
    }
}
