pub mod errors;
pub mod fs;
pub mod socket;
use config::ast;

pub type Mode = i32;
pub type PID = u32;
pub type Inode = u64;
pub type LinkId = (PID, ast::LinkHandle);
