use bincode::config;
use bincode::error::DecodeError;
use std::io;
use std::os::fd::AsRawFd;
use std::path::Path;
use std::time::Duration;
use std::{fs::File, io::BufReader, os::unix::net::UnixDatagram};

use crate::log::BinaryLogRecord;
use fuse::fs::WriteSignal;
use mio::{Events, Interest, Poll, Token, unix::SourceFd};

use crate::{Readers, Writers};
use crate::{
    errors::SourceError,
    router::{Router, Timestep},
};

/// Different sources for write events
/// * `Simulate`: Take actual writes from processes.
/// * `Replay`: Use the timesteps writes were logged at from simulation.
pub enum Source {
    /// Write events come from executing processes.
    Simulated {
        poll: Poll,
        events: Events,
        readers: Readers,
        writers: Writers,
    },
    /// Write events come from a log.
    Replay {
        src: BufReader<File>,
        readers: Readers,
        next_log: Option<BinaryLogRecord>,
    },
}

impl Source {
    pub fn simulated(
        sockets: &[UnixDatagram],
        readers: Readers,
        writers: Writers,
    ) -> Result<Self, SourceError> {
        let poll = Poll::new().map_err(|_| SourceError::SimulatedEvents)?;
        let events = Events::with_capacity(sockets.len());
        for (index, sock) in sockets.iter().enumerate() {
            poll.registry()
                .register(
                    &mut SourceFd(&sock.as_raw_fd()),
                    Token(index),
                    Interest::READABLE,
                )
                .map_err(|_| SourceError::PollRegistration)?;
        }
        Ok(Self::Simulated {
            poll,
            events,
            readers,
            writers,
        })
    }

    pub fn replay(log: impl AsRef<Path>, readers: Readers) -> Result<Self, SourceError> {
        let src = BufReader::new(File::open(log).map_err(SourceError::ReplayLogOpen)?);
        Ok(Self::Replay {
            src,
            readers,
            next_log: None,
        })
    }

    pub fn print_logs(log: impl AsRef<Path>) -> Result<(), SourceError> {
        let mut src = BufReader::new(File::open(log).map_err(SourceError::ReplayLogOpen)?);
        loop {
            let config = config::standard();
            match bincode::decode_from_reader::<BinaryLogRecord, _, _>(&mut src, config) {
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
        poll: &mut Poll,
        events: &mut Events,
        readers: &Readers,
        writers: &Writers,
        router: &mut Router,
        delta: Duration,
    ) -> Result<(), SourceError> {
        // Check write events
        poll.poll(events, Some(delta))
            .map_err(|_| SourceError::PollError)?;
        for event in events.iter() {
            let Token(index) = event.token();
            router
                .receive_write(index)
                .map_err(SourceError::RouterError)?;
        }
        for writer in writers.iter() {
            while writer.request.try_recv().is_ok() {
                let _ = writer.ack.send(WriteSignal::Done);
            }
        }
        router.step().map_err(SourceError::RouterError)?;
        for (index, reader) in readers.iter().enumerate() {
            while reader.request.try_recv().is_ok() {
                let _ = reader.ack.send(
                    router
                        .deliver_msg(index)
                        .map_err(SourceError::RouterError)?,
                );
            }
        }
        Ok(())
    }

    fn poll_log(
        src: &mut BufReader<File>,
        ts: Timestep,
        readers: &Readers,
        router: &mut Router,
        next_log: &mut Option<BinaryLogRecord>,
    ) -> Result<(), SourceError> {
        // Only do this I/O if we either don't know when the next log
        // is or if we know there are logs ready to be sent.
        if next_log.as_ref().is_none_or(|rec| rec.timestep <= ts) {
            if let Some(Err(e)) = next_log
                .take()
                .map(|rec| router.post_to_mailboxes(rec.node, rec.channel, rec.data))
            {
                return Err(SourceError::RouterError(e));
            }

            loop {
                let config = config::standard();
                match bincode::decode_from_reader::<BinaryLogRecord, _, _>(&mut *src, config) {
                    Ok(BinaryLogRecord {
                        is_publisher: false,
                        ..
                    }) => break Err(SourceError::InvalidLogType),
                    // Record scheduled for the future
                    Ok(rec) if rec.timestep > ts => {
                        *next_log = Some(rec);
                        break Ok(());
                    }
                    Ok(BinaryLogRecord {
                        node,
                        channel,
                        data,
                        ..
                    }) => {
                        if let Err(e) = router.post_to_mailboxes(node, channel, data) {
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
        for (index, reader) in readers.iter().enumerate() {
            while reader.request.try_recv().is_ok() {
                let _ = reader.ack.send(
                    router
                        .deliver_msg(index)
                        .map_err(SourceError::RouterError)?,
                );
            }
        }
        Ok(())
    }

    pub(crate) fn poll(
        &mut self,
        router: &mut Router,
        ts: Timestep,
        delta: Duration,
    ) -> Result<(), SourceError> {
        match self {
            Self::Simulated {
                poll,
                events,
                readers,
                writers,
            } => Self::poll_simulated(poll, events, readers, writers, router, delta),
            Self::Replay {
                src,
                readers,
                next_log,
            } => Self::poll_log(src, ts, readers, router, next_log),
        }
    }
}
