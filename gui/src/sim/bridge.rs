use std::sync::{Arc, Mutex};

use crossbeam_channel::Sender;
use tracing::field::Visit;
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer};

use trace::format::{DropReason, TraceEvent, TraceRecord};
use trace::writer::TraceWriter;

/// Events sent from the simulation bridge to the GUI.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum GuiEvent {
    /// Protocol binaries are being compiled.
    BuildStarted,
    /// Protocol binaries compiled successfully.
    BuildComplete,
    Trace(TraceRecord),
    TimestepAdvanced(u64),
    SimulationComplete,
    SimulationError(String),
    /// A single line of stdout/stderr from a running node process.
    ProcessOutputLine {
        node: String,
        protocol: String,
        stream: OutputStream,
        line: String,
    },
}

/// Which output stream a line came from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OutputStream {
    Stdout,
    Stderr,
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
    bit_errors: bool,
    data: Vec<u8>,
    reason: Option<String>,
    motion_spec: Option<String>,
    charge_nj: u64,
    msg_id: u64,
    x: f64,
    y: f64,
    z: f64,
}

impl Visit for BridgeVisitor {
    fn record_debug(&mut self, _: &tracing::field::Field, _: &dyn std::fmt::Debug) {}

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        match field.name() {
            "timestep" => self.timestep = value,
            "channel" => self.channel = value as u32,
            "node" => self.node = value as u32,
            "charge_nj" => self.charge_nj = value,
            "msg_id" => self.msg_id = value,
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

    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        match field.name() {
            "tx" => self.is_tx = value,
            "bit_errors" => self.bit_errors = value,
            _ => {}
        }
    }

    fn record_bytes(&mut self, field: &tracing::field::Field, value: &[u8]) {
        if field.name() == "data" {
            self.data = value.to_vec();
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        match field.name() {
            "reason" => self.reason = Some(value.to_string()),
            "spec" => self.motion_spec = Some(value.to_string()),
            _ => {}
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
                    msg_id: visitor.msg_id,
                },
            };
            self.emit(record);
            return;
        }

        if target == "battery" {
            let mut visitor = BridgeVisitor::default();
            event.record(&mut visitor);
            let record = TraceRecord {
                timestep: visitor.timestep,
                event: TraceEvent::EnergyUpdate {
                    node: visitor.node,
                    energy_nj: visitor.charge_nj,
                },
            };
            self.emit(record);
            return;
        }

        if target == "movement" {
            let mut visitor = BridgeVisitor::default();
            event.record(&mut visitor);
            let record = TraceRecord {
                timestep: visitor.timestep,
                event: TraceEvent::PositionUpdate {
                    node: visitor.node,
                    x: visitor.x,
                    y: visitor.y,
                    z: visitor.z,
                },
            };
            self.emit(record);
            return;
        }

        if target == "motion" {
            let mut visitor = BridgeVisitor::default();
            event.record(&mut visitor);
            let record = TraceRecord {
                timestep: visitor.timestep,
                event: TraceEvent::MotionUpdate {
                    node: visitor.node,
                    spec: visitor.motion_spec.unwrap_or_default(),
                },
            };
            self.emit(record);
            return;
        }

        if target == "timestep" {
            let mut visitor = BridgeVisitor::default();
            event.record(&mut visitor);
            if let Some(tx) = self.sinks.gui_tx.lock().unwrap().as_ref() {
                let _ = tx.send(GuiEvent::TimestepAdvanced(visitor.timestep));
            }
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
                msg_id: visitor.msg_id,
            }
        } else {
            TraceEvent::MessageRecv {
                dst_node: visitor.node,
                channel: visitor.channel,
                data: visitor.data,
                bit_errors: visitor.bit_errors,
                msg_id: visitor.msg_id,
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
