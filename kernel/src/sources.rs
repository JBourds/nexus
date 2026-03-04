//! sources.rs
//! Abstraction for different ways which messages get delivered to the router.
use bincode::config;
use bincode::error::DecodeError;
use std::io;
use std::path::Path;
use std::sync::mpsc;
use std::{fs::File, io::BufReader};

use crate::log::LogRecord;
use crate::router::RoutingServer;
use crate::{errors::SourceError, router::Timestep};

/// Different sources for write events
/// * `Simulate`: Take actual writes from processes.
/// * `Replay`: Use the timesteps writes were logged at from simulation.
/// * `Empty`: Stub. No messages get delivered.
#[derive(Debug)]
pub enum Source {
    /// Write events come from executing processes.
    Simulated { rx: mpsc::Receiver<fuse::FsMessage> },
    /// Write events come from a log.
    Replay {
        src: BufReader<File>,
        next_log: Option<LogRecord>,
    },
    /// No write events happen
    Empty,
}

impl Source {
    pub fn simulated(rx: mpsc::Receiver<fuse::FsMessage>) -> Result<Self, SourceError> {
        Ok(Self::Simulated { rx })
    }

    pub fn replay(log: impl AsRef<Path>) -> Result<Self, SourceError> {
        let src = BufReader::new(File::open(log).map_err(SourceError::ReplayLogOpen)?);
        Ok(Self::Replay {
            src,
            next_log: None,
        })
    }

    pub fn print_logs(log: impl AsRef<Path>) -> Result<(), SourceError> {
        let mut src = BufReader::new(File::open(log).map_err(SourceError::ReplayLogOpen)?);
        loop {
            let config = config::standard();
            match bincode::decode_from_reader::<LogRecord, _, _>(&mut src, config) {
                Ok(record) => {
                    println!("{record:?}");
                }
                Err(DecodeError::Io { inner, .. })
                    if inner.kind() == io::ErrorKind::UnexpectedEof =>
                {
                    break Ok(());
                }
                Err(e) => break Err(SourceError::ReplayLogRead(e)),
            }
        }?;
        Ok(())
    }

    fn poll_simulated(
        rx: &mut mpsc::Receiver<fuse::FsMessage>,
        router: &mut RoutingServer,
    ) -> Result<(), SourceError> {
        // Receive all write requests from FS then let router ingest them
        for msg in rx.try_iter() {
            match msg {
                fuse::FsMessage::Write(msg) => {
                    router
                        .receive_write(msg)
                        .map_err(SourceError::RouterError)?;
                }
                fuse::FsMessage::Read(msg) => {
                    router.request_read(msg).map_err(SourceError::RouterError)?;
                }
            }
        }
        router.step().map_err(SourceError::RouterError)?;
        Ok(())
    }

    fn poll_log(
        src: &mut BufReader<File>,
        ts: Timestep,
        router: &mut RoutingServer,
        next_log: &mut Option<LogRecord>,
    ) -> Result<(), SourceError> {
        // Only do this I/O if we either don't know when the next log
        // is or if we know there are logs ready to be sent.
        if next_log.as_ref().is_none_or(|rec| rec.timestep() <= ts) {
            // Queue the previously peeked record if it's due.
            if let Some(rec) = next_log.take() {
                if let Some(e) = Self::queue_record(router, rec).err() {
                    return Err(e);
                }
            }

            loop {
                let config = config::standard();
                match bincode::decode_from_reader::<LogRecord, _, _>(&mut *src, config) {
                    // Record scheduled for the future — save it and stop.
                    Ok(rec) if rec.timestep() > ts => {
                        *next_log = Some(rec);
                        break Ok(());
                    }
                    Ok(rec) => {
                        if let Err(e) = Self::queue_record(router, rec) {
                            break Err(e);
                        }
                    }
                    Err(DecodeError::Io { inner, .. })
                        if inner.kind() == io::ErrorKind::UnexpectedEof =>
                    {
                        break Ok(());
                    }
                    Err(e) => break Err(SourceError::ReplayLogRead(e)),
                }
            }?;
        }
        router.step().map_err(SourceError::RouterError)?;
        Ok(())
    }

    /// Queue a decoded log record on the router.  Battery records are skipped
    /// during replay (they are informational snapshots, not re-playable events).
    fn queue_record(router: &mut RoutingServer, rec: LogRecord) -> Result<(), SourceError> {
        match rec {
            LogRecord::Message {
                is_publisher: false,
                ..
            } => Err(SourceError::InvalidLogType),
            LogRecord::Message {
                node,
                channel,
                data,
                ..
            } => router
                .queue_message(node, channel, data)
                .map_err(SourceError::RouterError),
            // Battery snapshots are not replayed as events.
            LogRecord::Battery { .. } => Ok(()),
        }
    }

    pub(crate) fn poll(
        &mut self,
        router: &mut RoutingServer,
        ts: Timestep,
    ) -> Result<(), SourceError> {
        match self {
            Self::Empty => Ok(()),
            Self::Simulated { rx } => Self::poll_simulated(rx, router),
            Self::Replay { src, next_log } => Self::poll_log(src, ts, router, next_log),
        }
    }
}
