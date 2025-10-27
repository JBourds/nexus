use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug)]
#[allow(dead_code)]
pub struct Error {
    node: String,
    protocol: String,
    root: PathBuf,
    msg: String,
}

impl Error {
    pub(crate) fn new(node: String, protocol: String, root: PathBuf, msg: String) -> Self {
        Self {
            node,
            protocol,
            root,
            msg,
        }
    }
}

#[derive(Error, Debug)]
pub enum ProtocolError {
    #[error("Runner Error: {0:?}")]
    RunnerError(Error),
    #[error("Build Error: {0:?}")]
    BuildError(Error),
    #[error("{0:#?}")]
    BuildErrors(Vec<Error>),
    #[error("Unable to run process: {0:#?}.")]
    UnableToRun(std::io::Error),
}
