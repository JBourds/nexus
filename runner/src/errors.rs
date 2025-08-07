use thiserror::Error;

#[derive(Error, Debug)]
pub enum ProtocolError {
    #[error("Node {node_name}: Error for protocol {protocol_name}: {msg}")]
    RunnerError {
        node_name: String,
        protocol_name: String,
        msg: String,
    },
}
