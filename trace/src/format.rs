use bincode::{Decode, Encode};
use serde::{Deserialize, Serialize};

pub const MAGIC: [u8; 4] = *b"NXTR";
pub const VERSION: u16 = 3;

#[derive(Encode, Decode, Serialize, Deserialize, Debug, Clone)]
pub struct TraceHeader {
    pub node_names: Vec<String>,
    pub channel_names: Vec<String>,
    pub timestep_count: u64,
    /// Max energy (in nanojoules) per node. None = no charge tracking.
    pub node_max_nj: Vec<Option<u64>>,
}

#[derive(Encode, Decode, Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum DropReason {
    BelowSensitivity,
    PacketLoss,
    TtlExpired,
    BufferFull,
}

#[derive(Encode, Decode, Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum TraceEvent {
    MessageSent {
        src_node: u32,
        channel: u32,
        data: Vec<u8>,
    },
    MessageRecv {
        dst_node: u32,
        channel: u32,
        data: Vec<u8>,
        /// True when the received data was corrupted by bit errors.
        bit_errors: bool,
    },
    MessageDropped {
        src_node: u32,
        channel: u32,
        reason: DropReason,
    },
    PositionUpdate {
        node: u32,
        x: f64,
        y: f64,
        z: f64,
    },
    EnergyUpdate {
        node: u32,
        energy_nj: u64,
    },
    MotionUpdate {
        node: u32,
        spec: String,
    },
}

#[derive(Encode, Decode, Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct TraceRecord {
    pub timestep: u64,
    pub event: TraceEvent,
}
