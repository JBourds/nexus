use std::num::{NonZeroU64, NonZeroUsize};

use crate::ast::{ChannelKind, ChannelType, TimeUnit};

impl ChannelType {
    pub const MSG_MAX_DEFAULT: NonZeroUsize = NonZeroUsize::new(4096).unwrap();

    pub fn time_units(&self) -> TimeUnit {
        self.unit
    }

    pub fn ttl(&self) -> Option<NonZeroU64> {
        self.ttl
    }

    pub fn max_buffered(&self) -> Option<NonZeroUsize> {
        match &self.kind {
            ChannelKind::Shared => Some(NonZeroUsize::MAX),
            ChannelKind::Exclusive { nbuffered } => *nbuffered,
        }
    }

    pub fn max_buf_size(&self) -> NonZeroUsize {
        self.max_size
    }

    pub fn delivers_to_self(&self) -> bool {
        self.read_own_writes
    }

    pub fn new_internal() -> Self {
        Self {
            ttl: None,
            unit: TimeUnit::Seconds,
            max_size: Self::MSG_MAX_DEFAULT,
            read_own_writes: true,
            kind: ChannelKind::Exclusive { nbuffered: None },
        }
    }
}

impl Default for ChannelType {
    fn default() -> Self {
        Self {
            ttl: None,
            unit: TimeUnit::Seconds,
            max_size: Self::MSG_MAX_DEFAULT,
            read_own_writes: false,
            kind: ChannelKind::Exclusive { nbuffered: None },
        }
    }
}
