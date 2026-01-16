//! status/mod.rs
//!
//! Module for the kernel's status server. This server handles both resource
//! allocation/accounting (CPU and in the future energy) as well as health
//! checks (premature exit).
//!
//! This server performs real-time computations based on actual
//! CPU frequency (if a governor is enabled) t0 determine the correct bandwidth
//! proportions for each executing program given target clock rates and time
//! dilation.

use std::{
    sync::mpsc,
    thread::{self, JoinHandle},
};

use runner::{CgroupController, ProtocolHandle};

use crate::{KernelServer, errors::KernelError, status::errors::StatusError};

pub mod errors;
mod health;
pub mod messages;

use messages::*;

type HandleInner = Result<Vec<ProtocolHandle>, KernelError>;
type ServerHandle = JoinHandle<HandleInner>;

impl KernelServer<ServerHandle, KernelMessage, StatusMessage> {
    pub fn check_health(&mut self) -> Result<StatusMessage, KernelError> {
        self.tx
            .send(KernelMessage::HealthCheck)
            .map_err(|e| KernelError::StatusError(StatusError::KernelSendError(e)))?;
        self.rx
            .recv()
            .map_err(|e| KernelError::StatusError(StatusError::RecvError(e)))
    }

    pub fn shutdown(self) -> HandleInner {
        self.tx
            .send(KernelMessage::Shutdown)
            .map_err(|e| KernelError::StatusError(StatusError::KernelSendError(e)))?;
        self.handle.join().expect("thread panic!")
    }
}

pub struct StatusServer {
    /// Scalar value to try and speed up or slow down requested cycles with.
    time_dilation: f64,
    /// Instance to simplify controlling resource allocation through cgroups.
    cgroup_controller: CgroupController,
    /// Handles to use for location information about running process requested
    /// resources and cgroups.
    handles: Vec<ProtocolHandle>,
}

impl StatusServer {
    pub fn serve(
        time_dilation: f64,
        mut cgroup_controller: CgroupController,
        handles: Vec<ProtocolHandle>,
    ) -> Result<KernelServer<ServerHandle, KernelMessage, StatusMessage>, KernelError> {
        let (kernel_tx, kernel_rx) = mpsc::channel::<KernelMessage>();
        let (status_tx, status_rx) = mpsc::channel::<StatusMessage>();

        cgroup_controller.unfreeze_nodes();
        thread::Builder::new()
            .name("nexus_status_server".to_string())
            .spawn(move || {
                let mut server = Self {
                    time_dilation,
                    cgroup_controller,
                    handles,
                };
                loop {
                    match kernel_rx.recv() {
                        Ok(KernelMessage::HealthCheck) => {
                            let premature_exits = health::check(&mut server.handles);
                            if premature_exits.is_empty() {
                                status_tx.send(StatusMessage::Ok).map_err(|e| {
                                    KernelError::StatusError(StatusError::StatusSendError(e))
                                })?;
                                continue;
                            }

                            status_tx.send(StatusMessage::PrematureExit).map_err(|e| {
                                KernelError::StatusError(StatusError::StatusSendError(e))
                            })?;
                            health::kill(&mut server.handles).expect("unable to kill processes");
                            server.cgroup_controller.freeze_nodes();
                        }
                        Ok(KernelMessage::Shutdown) => {
                            return Ok(server.handles);
                        }
                        Ok(KernelMessage::Freeze) => {
                            server.cgroup_controller.freeze_nodes();
                        }
                        Ok(KernelMessage::Unfreeze) => {
                            server.cgroup_controller.unfreeze_nodes();
                        }
                        Err(e) => {
                            break Err(KernelError::StatusError(StatusError::RecvError(e)));
                        }
                    };
                }
            })
            .map_err(|e| KernelError::StatusError(StatusError::ThreadCreation(e)))
            .map(|handle| KernelServer::new(handle, kernel_tx, status_rx))
    }
}
