use std::io;

use runner::ProtocolHandle;
use tracing::error;

/// issue kill commands to all processes in set of handles
pub fn kill(handles: &mut [ProtocolHandle]) -> io::Result<()> {
    for handle in handles {
        handle.process.kill()?;
    }
    Ok(())
}

/// returns a vector of PIDs with errors
pub fn check(handles: &mut [ProtocolHandle]) -> Vec<usize> {
    let mut errors = vec![];
    for (index, handle) in handles.iter_mut().enumerate() {
        if let Ok(Some(_)) = handle.process.try_wait() {
            error!("Process prematurely exited");
            errors.push(index);
        }
    }
    errors
}
