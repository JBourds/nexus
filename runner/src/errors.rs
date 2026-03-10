use std::fmt;
use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug)]
pub struct RunnerDetail {
    pub node: String,
    pub protocol: String,
    pub root: PathBuf,
    pub msg: String,
}

impl RunnerDetail {
    pub(crate) fn new(node: String, protocol: String, root: PathBuf, msg: String) -> Self {
        Self {
            node,
            protocol,
            root,
            msg,
        }
    }
}

impl fmt::Display for RunnerDetail {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[{}/{}] {}: {}",
            self.node,
            self.protocol,
            self.root.display(),
            self.msg
        )
    }
}

#[derive(Error, Debug)]
pub enum ProtocolError {
    #[error("Runner Error: {0}")]
    RunnerError(RunnerDetail),
    #[error("Build Error: {0}")]
    BuildError(RunnerDetail),
    #[error("Build Errors:\n{}", .0.iter().map(|e| format!("  - {e}")).collect::<Vec<_>>().join("\n"))]
    BuildErrors(Vec<RunnerDetail>),
    #[error("Unable to run process: {0:#?}.")]
    UnableToRun(std::io::Error),
}
