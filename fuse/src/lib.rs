pub mod errors;
pub mod fs;
pub mod socket;
use std::{collections::HashMap, os::unix::net::UnixDatagram};

use config::ast;

pub type Mode = i32;
pub type PID = u32;
pub type Inode = u64;
pub type ChannelId = (PID, ast::LinkHandle);
pub type KernelLinks = HashMap<(PID, ast::LinkHandle), (ast::NodeHandle, UnixDatagram)>;
