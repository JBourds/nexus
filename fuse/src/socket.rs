use crate::PID;
use crate::errors::SocketError;
use std::os::unix::net::UnixDatagram;

pub fn recv(
    socket: &mut UnixDatagram,
    data: &mut [u8],
    pid: PID,
    link_name: impl AsRef<str>,
) -> Result<usize, SocketError> {
    socket
        .recv(data)
        .map_err(|ioerr| SocketError::SocketReadError {
            ioerr,
            pid,
            link_name: link_name.as_ref().to_string(),
        })
        .map(|n_read| {
            if n_read != data.len() {
                Err(SocketError::ReadSizeMismatch {
                    expected: data.len(),
                    actual: n_read,
                })
            } else {
                Ok(n_read)
            }
        })?
}

pub fn send(
    socket: &mut UnixDatagram,
    data: &[u8],
    pid: PID,
    link_name: impl AsRef<str>,
) -> Result<usize, SocketError> {
    socket
        .send(data)
        .map_err(|ioerr| SocketError::SocketWriteError {
            ioerr,
            pid,
            link_name: link_name.as_ref().to_string(),
        })
        .map(|n_written| {
            if n_written != data.len() {
                Err(SocketError::WriteSizeMismatch {
                    expected: data.len(),
                    actual: n_written,
                })
            } else {
                Ok(n_written)
            }
        })?
}
