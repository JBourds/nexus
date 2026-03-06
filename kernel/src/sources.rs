//! sources.rs
//! Abstraction for different ways which messages get delivered to the router.
use bincode::config;
use bincode::error::DecodeError;
use std::io;
use std::path::Path;
use std::sync::mpsc;
use std::{fs::File, io::BufReader};

use crate::log::{LogRecord, MessageRecord};
use crate::router::RoutingServer;
use crate::{errors::SourceError, router::Timestep};
use trace::format::{TraceEvent, TraceRecord};
use trace::reader::TraceReader;

/// Different sources for write events
/// * `Simulate`: Take actual writes from processes.
/// * `Replay`: Use the timesteps writes were logged at from simulation.
/// * `ReplayTrace`: Replay from the unified trace format.
/// * `Empty`: Stub. No messages get delivered.
#[derive(Debug)]
pub enum Source {
    /// Write events come from executing processes.
    Simulated { rx: mpsc::Receiver<fuse::FsMessage> },
    /// Write events come from a legacy binary log. Only `Message { tx: true }` records are
    /// replayed; RX, Movement, and Battery records are skipped.
    Replay {
        src: BufReader<File>,
        next_log: Option<MessageRecord>,
    },
    /// Write events come from a unified trace file.
    ReplayTrace {
        reader: TraceReader,
        next_record: Option<TraceRecord>,
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

    pub fn replay_trace(trace_path: impl AsRef<Path>) -> Result<Self, SourceError> {
        let reader = TraceReader::open(trace_path)
            .map_err(|e| SourceError::ReplayLogOpen(std::io::Error::other(e.to_string())))?;
        Ok(Self::ReplayTrace {
            reader,
            next_record: None,
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
        next_log: &mut Option<MessageRecord>,
    ) -> Result<(), SourceError> {
        // Only do this I/O if we either don't know when the next log
        // is or if we know there are logs ready to be sent.
        if next_log.as_ref().is_none_or(|rec| rec.timestep <= ts) {
            // Queue the previously peeked record if it's due.
            if let Some(rec) = next_log.take() {
                router
                    .queue_message(rec.node, rec.channel, rec.data)
                    .map_err(SourceError::RouterError)?;
            }

            loop {
                let config = config::standard();
                match bincode::decode_from_reader::<LogRecord, _, _>(&mut *src, config) {
                    // Skip RX message records during replay
                    Ok(LogRecord::Message(MessageRecord { tx: false, .. })) => continue,
                    // Skip movement records during replay (positions are recomputed)
                    Ok(LogRecord::Movement(_)) => continue,
                    // Skip battery records during replay (energy is recomputed)
                    Ok(LogRecord::Battery(_)) => continue,
                    // TX record scheduled for the future: buffer it
                    Ok(LogRecord::Message(rec)) if rec.timestep > ts => {
                        *next_log = Some(rec);
                        break Ok(());
                    }
                    // TX record ready to deliver now
                    Ok(LogRecord::Message(MessageRecord {
                        node,
                        channel,
                        data,
                        ..
                    })) => {
                        if let Err(e) = router.queue_message(node, channel, data) {
                            break Err(SourceError::RouterError(e));
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

    fn poll_trace(
        reader: &mut TraceReader,
        ts: Timestep,
        router: &mut RoutingServer,
        next_record: &mut Option<TraceRecord>,
    ) -> Result<(), SourceError> {
        // Deliver buffered record if ready
        if next_record.as_ref().is_none_or(|rec| rec.timestep <= ts) {
            if let Some(rec) = next_record.take()
                && let TraceEvent::MessageSent {
                    src_node,
                    channel,
                    data,
                } = rec.event
            {
                router
                    .queue_message(src_node as usize, channel as usize, data)
                    .map_err(SourceError::RouterError)?;
            }

            loop {
                match reader.next_record() {
                    Ok(Some(rec)) => {
                        // Only replay MessageSent events
                        if !matches!(rec.event, TraceEvent::MessageSent { .. }) {
                            continue;
                        }
                        if rec.timestep > ts {
                            *next_record = Some(rec);
                            break;
                        }
                        if let TraceEvent::MessageSent {
                            src_node,
                            channel,
                            data,
                        } = rec.event
                        {
                            router
                                .queue_message(src_node as usize, channel as usize, data)
                                .map_err(SourceError::RouterError)?;
                        }
                    }
                    Ok(None) => break,
                    Err(e) => {
                        return Err(SourceError::ReplayLogOpen(std::io::Error::other(
                            e.to_string(),
                        )));
                    }
                }
            }
        }
        router.step().map_err(SourceError::RouterError)?;
        Ok(())
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
            Self::ReplayTrace {
                reader,
                next_record,
            } => Self::poll_trace(reader, ts, router, next_record),
        }
    }
}
