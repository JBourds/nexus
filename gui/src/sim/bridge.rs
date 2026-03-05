use std::sync::{Arc, Mutex};

use crossbeam_channel::Sender;
use tracing::field::Visit;
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer};

use trace::format::{DropReason, TraceEvent, TraceRecord};
use trace::writer::TraceWriter;

/// Events sent from the simulation bridge to the GUI.
#[derive(Debug, Clone)]
pub enum GuiEvent {
    Trace(TraceRecord),
    TimestepAdvanced(u64),
    SimulationComplete,
    SimulationError(String),
}

/// Shared, swappable sinks for the simulation tracing layer.
///
/// Populated before each simulation run, cleared when the simulation ends.
/// The `Arc<Mutex<Option<...>>>` pattern lets a single global subscriber
/// serve multiple sequential simulation runs.
#[derive(Clone)]
pub struct SimSinks {
    pub gui_tx: Arc<Mutex<Option<Sender<GuiEvent>>>>,
    pub trace_writer: Arc<Mutex<Option<TraceWriter>>>,
}

impl SimSinks {
    pub fn new() -> Self {
        Self {
            gui_tx: Arc::new(Mutex::new(None)),
            trace_writer: Arc::new(Mutex::new(None)),
        }
    }

    /// Install sinks for a new simulation run.
    pub fn install(&self, gui_tx: Sender<GuiEvent>, writer: TraceWriter) {
        *self.gui_tx.lock().unwrap() = Some(gui_tx);
        *self.trace_writer.lock().unwrap() = Some(writer);
    }

    /// Clear sinks (flushes/drops the trace writer).
    pub fn clear(&self) {
        *self.gui_tx.lock().unwrap() = None;
        *self.trace_writer.lock().unwrap() = None;
    }
}

/// A `tracing_subscriber::Layer` with swappable sinks, allowing the same
/// global subscriber to serve multiple sequential simulation runs.
///
/// Handles `tx`, `rx`, and `drop` target events — forwarding each record
/// to both the GUI channel and the trace file writer (when installed).
pub struct ReloadableSimLayer {
    sinks: SimSinks,
}

impl ReloadableSimLayer {
    pub fn new(sinks: SimSinks) -> Self {
        Self { sinks }
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

impl<S: Subscriber> Layer<S> for ReloadableSimLayer {
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
            self.emit(record);
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
        self.emit(record);
    }
}

impl ReloadableSimLayer {
    fn emit(&self, record: TraceRecord) {
        // Send to GUI
        if let Some(tx) = self.sinks.gui_tx.lock().unwrap().as_ref() {
            let _ = tx.send(GuiEvent::Trace(record.clone()));
        }
        // Write to trace file
        if let Some(writer) = self.sinks.trace_writer.lock().unwrap().as_mut() {
            let _ = writer.write_record(&record);
        }
    }
}
