//! Linux networking primitives for Nexus.
//!
//! This crate provides TAP interface creation, network namespace management,
//! and the TAP frame router that bridges simulated network channels.
//!
//! ## Capabilities Required
//!
//! - `CAP_NET_ADMIN` for TAP interface and namespace creation.
//! - Operations that require elevated privileges are gated behind
//!   runtime capability checks and return clear errors.

pub mod namespace;
pub mod router;
pub mod tap;

pub use namespace::Namespace;
pub use router::{TapRouter, TapRouterHandle};
pub use tap::TapDevice;
