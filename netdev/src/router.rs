//! TAP frame router.
//!
//! The `TapRouter` owns one [`TapDevice`] per network-enabled node.  It polls
//! all TAP file descriptors using `mio`, reads outgoing Ethernet frames, and
//! delivers them to every other node's TAP after applying the link simulation
//! provided by a caller-supplied callback.
//!
//! The router runs on its own thread and communicates with the kernel via
//! an mpsc channel.

use std::collections::HashMap;
use std::io;
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use anyhow::{Context, Result};
use mio::unix::SourceFd;
use mio::{Events, Interest, Poll, Token};
use tracing::{debug, info, warn};

use crate::tap::{self, TapDevice};

/// A frame read from a TAP device, tagged with the source node index.
#[derive(Clone, Debug)]
pub struct TapFrame {
    /// Index of the source node (into the `devices` vec).
    pub src: usize,
    /// Raw Ethernet frame bytes.
    pub data: Vec<u8>,
}

/// Messages sent from the kernel to the TAP router thread.
#[derive(Debug)]
pub enum TapRouterCommand {
    /// Deliver a (possibly delayed/modified) frame to a destination node's TAP.
    Deliver {
        /// Destination node index.
        dst: usize,
        /// Raw Ethernet frame bytes.
        data: Vec<u8>,
    },
    /// Shut down the router thread.
    Shutdown,
}

/// Handle returned by [`TapRouter::spawn`] for communicating with the router
/// thread.
pub struct TapRouterHandle {
    /// Send commands (deliver, shutdown) to the router thread.
    pub cmd_tx: mpsc::Sender<TapRouterCommand>,
    /// Receive frames read from TAP devices.
    pub frame_rx: mpsc::Receiver<TapFrame>,
    /// Join handle for the router thread.
    handle: JoinHandle<Result<()>>,
}

impl TapRouterHandle {
    /// Deliver a frame to a destination node's TAP device.
    pub fn deliver(&self, dst: usize, data: Vec<u8>) -> Result<()> {
        self.cmd_tx
            .send(TapRouterCommand::Deliver { dst, data })
            .map_err(|_| anyhow::anyhow!("TAP router thread has exited"))
    }

    /// Drain all pending frames from TAP devices (non-blocking).
    pub fn drain_frames(&self) -> Vec<TapFrame> {
        let mut frames = Vec::new();
        while let Ok(frame) = self.frame_rx.try_recv() {
            frames.push(frame);
        }
        frames
    }

    /// Shut down the router thread and wait for it to finish.
    pub fn shutdown(self) -> Result<()> {
        let _ = self.cmd_tx.send(TapRouterCommand::Shutdown);
        self.handle
            .join()
            .expect("TAP router thread panicked")
    }
}

/// Manages TAP devices and routes frames between them.
pub struct TapRouter {
    /// One TAP device per network-enabled node, indexed by node position.
    devices: Vec<TapDevice>,
    /// Map from node name to device index.
    node_index: HashMap<String, usize>,
}

impl TapRouter {
    /// Create a new router with the given TAP devices.
    ///
    /// `devices` is a vec of `(node_name, TapDevice)` pairs.  The order
    /// defines the node indices used in [`TapFrame`] and
    /// [`TapRouterCommand::Deliver`].
    pub fn new(devices: Vec<(String, TapDevice)>) -> Self {
        let node_index: HashMap<String, usize> = devices
            .iter()
            .enumerate()
            .map(|(i, (name, _))| (name.clone(), i))
            .collect();
        let devices: Vec<TapDevice> = devices.into_iter().map(|(_, dev)| dev).collect();
        Self {
            devices,
            node_index,
        }
    }

    /// Look up the device index for a node name.
    pub fn node_to_index(&self, name: &str) -> Option<usize> {
        self.node_index.get(name).copied()
    }

    /// Spawn the router on a background thread.
    ///
    /// The returned [`TapRouterHandle`] lets the kernel:
    /// - receive frames via `frame_rx` (or `drain_frames()`)
    /// - deliver frames via `deliver(dst, data)` after link simulation
    /// - shut down via `shutdown()`
    pub fn spawn(self) -> Result<TapRouterHandle> {
        let (cmd_tx, cmd_rx) = mpsc::channel::<TapRouterCommand>();
        let (frame_tx, frame_rx) = mpsc::channel::<TapFrame>();

        let handle = thread::Builder::new()
            .name("nexus_tap_router".to_string())
            .spawn(move || self.run(cmd_rx, frame_tx))
            .context("Failed to spawn TAP router thread")?;

        Ok(TapRouterHandle {
            cmd_tx,
            frame_rx,
            handle,
        })
    }

    /// Main loop: poll TAP fds for incoming frames and process commands.
    fn run(
        self,
        cmd_rx: mpsc::Receiver<TapRouterCommand>,
        frame_tx: mpsc::Sender<TapFrame>,
    ) -> Result<()> {
        let mut poll = Poll::new().context("Failed to create mio::Poll")?;
        let mut events = Events::with_capacity(64);

        // Register all TAP fds with mio.
        for (i, dev) in self.devices.iter().enumerate() {
            dev.set_nonblocking()?;
            let fd = dev.raw_fd();
            poll.registry()
                .register(
                    &mut SourceFd(&fd),
                    Token(i),
                    Interest::READABLE,
                )
                .with_context(|| format!("Failed to register TAP fd for {}", dev.name()))?;
        }

        info!("TAP router started with {} devices", self.devices.len());

        let mut buf = vec![0u8; tap::MAX_FRAME_SIZE];

        loop {
            // Process any pending commands first (non-blocking).
            loop {
                match cmd_rx.try_recv() {
                    Ok(TapRouterCommand::Deliver { dst, data }) => {
                        if dst < self.devices.len() {
                            if let Err(e) = self.devices[dst].write_frame(&data) {
                                warn!("Failed to write frame to TAP {}: {e}", self.devices[dst].name());
                            }
                        } else {
                            warn!("Deliver to invalid TAP index {dst}");
                        }
                    }
                    Ok(TapRouterCommand::Shutdown) => {
                        info!("TAP router shutting down");
                        return Ok(());
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        debug!("TAP router command channel disconnected, shutting down");
                        return Ok(());
                    }
                }
            }

            // Poll TAP fds with a short timeout so we also check commands.
            if let Err(e) = poll.poll(&mut events, Some(Duration::from_millis(1))) {
                if e.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(e).context("mio poll failed");
            }

            for event in events.iter() {
                let idx = event.token().0;
                if idx >= self.devices.len() {
                    continue;
                }
                // Read all available frames from this TAP.
                loop {
                    match self.devices[idx].read_frame(&mut buf) {
                        Ok(n) if n > 0 => {
                            let frame = TapFrame {
                                src: idx,
                                data: buf[..n].to_vec(),
                            };
                            if frame_tx.send(frame).is_err() {
                                debug!("Frame receiver disconnected");
                                return Ok(());
                            }
                        }
                        Ok(_) => break,
                        Err(e) => {
                            let inner = e.downcast_ref::<io::Error>();
                            if inner.is_some_and(|ie| ie.kind() == io::ErrorKind::WouldBlock) {
                                break;
                            }
                            warn!("Error reading TAP {}: {e}", self.devices[idx].name());
                            break;
                        }
                    }
                }
            }
        }
    }
}
