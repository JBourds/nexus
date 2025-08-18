use crate::PID;
use crate::errors::SocketError;
use std::os::unix::net::UnixDatagram;

pub fn recv(
    socket: &mut UnixDatagram,
    data: &mut [u8],
    pid: PID,
    channel_name: impl AsRef<str>,
) -> Result<usize, SocketError> {
    socket
        .recv(data)
        .map_err(|ioerr| SocketError::SocketReadError {
            ioerr,
            pid,
            channel_name: channel_name.as_ref().to_string(),
        })
}

pub fn send(
    socket: &mut UnixDatagram,
    data: &[u8],
    pid: PID,
    channel_name: impl AsRef<str>,
) -> Result<usize, SocketError> {
    socket
        .send(data)
        .map_err(|ioerr| SocketError::SocketWriteError {
            ioerr,
            pid,
            channel_name: channel_name.as_ref().to_string(),
        })
}
