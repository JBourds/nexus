use std::num::{NonZeroU64, NonZeroUsize};

use crate::ast::{ChannelType, TimeUnit};

impl ChannelType {
    pub const MSG_MAX_DEFAULT: NonZeroUsize = NonZeroUsize::new(4096).unwrap();

    pub fn time_units(&self) -> TimeUnit {
        match self {
            ChannelType::Shared { unit, .. } => *unit,
            ChannelType::Exclusive { unit, .. } => *unit,
        }
    }

    pub fn ttl(&self) -> Option<NonZeroU64> {
        match self {
            ChannelType::Shared { ttl, .. } => *ttl,
            ChannelType::Exclusive { ttl, .. } => *ttl,
        }
    }

    pub fn max_buffered(&self) -> Option<NonZeroUsize> {
        match self {
            ChannelType::Shared { .. } => Some(NonZeroUsize::MAX),
            ChannelType::Exclusive { nbuffered, .. } => *nbuffered,
        }
    }

    pub fn max_buf_size(&self) -> NonZeroUsize {
        match self {
            ChannelType::Shared { max_size, .. } => *max_size,
            ChannelType::Exclusive { max_size, .. } => *max_size,
        }
    }

    pub fn delivers_to_self(&self) -> bool {
        match self {
            ChannelType::Shared {
                read_own_writes, ..
            } => *read_own_writes,
            ChannelType::Exclusive {
                read_own_writes, ..
            } => *read_own_writes,
        }
    }

    pub fn new_internal() -> Self {
        Self::Exclusive {
            ttl: None,
            unit: TimeUnit::Seconds,
            nbuffered: None,
            max_size: Self::MSG_MAX_DEFAULT,
            read_own_writes: true,
        }
    }
}

impl Default for ChannelType {
    fn default() -> Self {
        Self::Exclusive {
            ttl: None,
            unit: TimeUnit::Seconds,
            nbuffered: None,
            max_size: Self::MSG_MAX_DEFAULT,
            read_own_writes: false,
        }
    }
}
