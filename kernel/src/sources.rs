use std::os::fd::AsRawFd;
use std::time::Duration;
use std::{fs::File, io::BufReader, os::unix::net::UnixDatagram, path::PathBuf};

use fuse::KernelControlFile;
use fuse::fs::{ReadSignal, WriteSignal};
use mio::{Events, Interest, Poll, Token, unix::SourceFd};

use crate::{Readers, Writers};
use crate::{
    errors::{RouterError, SourceError},
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
        log: PathBuf,
        buf: BufReader<File>,
        readers: Readers,
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

    pub fn replay(log: PathBuf, readers: Readers) -> Result<Self, SourceError> {
        unimplemented!()
    }

    pub fn poll(&mut self, router: &mut Router, delta: Duration) -> Result<(), SourceError> {
        match self {
            Self::Simulated {
                poll,
                events,
                readers,
                writers,
            } => {
                // Check write events
                poll.poll(events, Some(delta))
                    .map_err(|_| SourceError::PollError)?;
                for event in events.iter() {
                    let Token(index) = event.token();
                    router
                        .receive_write(index)
                        .map_err(SourceError::RouterError)?;
                }
                for (index, reader) in readers.iter().enumerate() {
                    while reader.request.try_recv().is_ok() {
                        let _ = reader.ack.send(
                            router
                                .deliver_msg(index)
                                .map_err(SourceError::RouterError)?,
                        );
                    }
                }
                router.step().map_err(SourceError::RouterError)?;
                for (_, writer) in writers.iter().enumerate() {
                    while writer.request.try_recv().is_ok() {
                        let _ = writer.ack.send(WriteSignal::Done);
                    }
                }
                Ok(())
            }
            Self::Replay { log, buf, readers } => todo!(),
        }
    }
}
