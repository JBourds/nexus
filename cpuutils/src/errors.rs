use thiserror::Error;

#[derive(Debug, Error)]
pub enum CpusetError {
    #[error("Error from libc: {0}")]
    Libc(LibcError),
    #[error("Byte index out of bounds. Index: {index}, Length: {length}")]
    ByteIndexRange { index: usize, length: usize },
}

#[derive(Debug, Error)]
pub enum LibcError {
    #[error("Unable to get value from sysconf")]
    Sysconf,
    #[error("Unable to set scheduler affinity")]
    SchedSetAffinity,
    #[error("Unable to get scheduler affinity")]
    SchedGetAffinity,
}

impl From<LibcError> for CpusetError {
    fn from(val: LibcError) -> Self {
        CpusetError::Libc(val)
    }
}
