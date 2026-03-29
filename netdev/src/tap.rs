//! TAP (Layer 2) virtual network interface.
//!
//! A TAP device presents one end as a normal network interface to the kernel
//! and the other end as a file descriptor for userspace to read/write raw
//! Ethernet frames.

use std::ffi::CString;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

use anyhow::{Context, Result, bail};

/// Maximum Ethernet frame size (MTU 1500 + 14-byte header + 4-byte VLAN tag).
pub const MAX_FRAME_SIZE: usize = 1518;

// ioctl constants for TUN/TAP
const TUNSETIFF: libc::c_ulong = 0x400454ca;
const IFF_TAP: libc::c_short = 0x0002;
const IFF_NO_PI: libc::c_short = 0x1000;

/// A TAP network device backed by `/dev/net/tun`.
#[derive(Debug)]
pub struct TapDevice {
    /// The file descriptor for reading/writing frames.
    fd: OwnedFd,
    /// The interface name (e.g., "nexus_node0").
    name: String,
}

#[repr(C)]
#[derive(Default)]
struct Ifreq {
    ifr_name: [u8; libc::IFNAMSIZ],
    ifr_flags: libc::c_short,
    _pad: [u8; 22],
}

impl TapDevice {
    /// Create a new TAP device with the given name.
    ///
    /// Requires `CAP_NET_ADMIN` capability.
    pub fn create(name: &str) -> Result<Self> {
        if name.len() >= libc::IFNAMSIZ {
            bail!("TAP interface name '{}' too long (max {} chars)", name, libc::IFNAMSIZ - 1);
        }

        // Open /dev/net/tun
        let tun_path = CString::new("/dev/net/tun")?;
        let fd = unsafe {
            let raw_fd = libc::open(tun_path.as_ptr(), libc::O_RDWR | libc::O_CLOEXEC);
            if raw_fd < 0 {
                return Err(io::Error::last_os_error())
                    .context("Failed to open /dev/net/tun. Is CAP_NET_ADMIN available?");
            }
            OwnedFd::from_raw_fd(raw_fd)
        };

        // Set up the ifreq struct
        let mut ifr = Ifreq::default();
        let name_bytes = name.as_bytes();
        ifr.ifr_name[..name_bytes.len()].copy_from_slice(name_bytes);
        ifr.ifr_flags = IFF_TAP | IFF_NO_PI;

        // Create the TAP interface via ioctl
        let ret = unsafe { libc::ioctl(fd.as_raw_fd(), TUNSETIFF, &ifr as *const Ifreq) };
        if ret < 0 {
            return Err(io::Error::last_os_error())
                .context(format!("ioctl TUNSETIFF failed for TAP '{name}'"));
        }

        tracing::info!("Created TAP interface: {name}");

        Ok(Self {
            fd,
            name: name.to_string(),
        })
    }

    /// Get the interface name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Get the raw file descriptor (for use with poll/epoll/mio).
    pub fn raw_fd(&self) -> i32 {
        self.fd.as_raw_fd()
    }

    /// Read a single Ethernet frame from the TAP device.
    ///
    /// Blocks until a frame is available. Returns the frame bytes.
    pub fn read_frame(&self, buf: &mut [u8]) -> Result<usize> {
        let mut file = unsafe { std::fs::File::from_raw_fd(self.fd.as_raw_fd()) };
        let n = file.read(buf).context("Failed to read from TAP")?;
        // Prevent the File from closing our fd on drop
        std::mem::forget(file);
        Ok(n)
    }

    /// Write a raw Ethernet frame to the TAP device.
    pub fn write_frame(&self, frame: &[u8]) -> Result<usize> {
        let mut file = unsafe { std::fs::File::from_raw_fd(self.fd.as_raw_fd()) };
        let n = file.write(frame).context("Failed to write to TAP")?;
        std::mem::forget(file);
        Ok(n)
    }

    /// Bring the interface up using `ip link set <name> up`.
    pub fn set_up(&self) -> Result<()> {
        let output = std::process::Command::new("ip")
            .args(["link", "set", &self.name, "up"])
            .output()
            .context("Failed to run 'ip link set up'")?;
        if !output.status.success() {
            bail!(
                "ip link set {} up failed: {}",
                self.name,
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Ok(())
    }

    /// Configure an IP address on the interface.
    pub fn set_address(&self, addr_cidr: &str) -> Result<()> {
        let output = std::process::Command::new("ip")
            .args(["addr", "add", addr_cidr, "dev", &self.name])
            .output()
            .context("Failed to run 'ip addr add'")?;
        if !output.status.success() {
            bail!(
                "ip addr add {} dev {} failed: {}",
                addr_cidr,
                self.name,
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Ok(())
    }

    /// Set the interface to non-blocking mode for use with poll/epoll.
    pub fn set_nonblocking(&self) -> Result<()> {
        let flags = unsafe { libc::fcntl(self.fd.as_raw_fd(), libc::F_GETFL) };
        if flags < 0 {
            return Err(io::Error::last_os_error()).context("fcntl F_GETFL failed");
        }
        let ret = unsafe { libc::fcntl(self.fd.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) };
        if ret < 0 {
            return Err(io::Error::last_os_error()).context("fcntl F_SETFL O_NONBLOCK failed");
        }
        Ok(())
    }

    /// Delete the TAP interface.
    pub fn destroy(&self) -> Result<()> {
        let output = std::process::Command::new("ip")
            .args(["link", "delete", &self.name])
            .output()
            .context("Failed to run 'ip link delete'")?;
        if !output.status.success() {
            // Interface may already be gone (e.g., namespace was deleted first)
            tracing::debug!(
                "ip link delete {} returned: {}",
                self.name,
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(())
    }
}

impl Drop for TapDevice {
    fn drop(&mut self) {
        // Best-effort cleanup: delete the interface.
        // The fd is closed automatically by OwnedFd's drop.
        let _ = std::process::Command::new("ip")
            .args(["link", "delete", &self.name])
            .output();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tap_name_too_long_is_rejected() {
        let long_name = "a".repeat(libc::IFNAMSIZ);
        let result = TapDevice::create(&long_name);
        assert!(result.is_err());
        assert!(
            format!("{:?}", result.unwrap_err()).contains("too long"),
            "Expected 'too long' error"
        );
    }

    #[test]
    fn tap_constants_are_correct() {
        // IFF_TAP = 0x0002, IFF_NO_PI = 0x1000 per Linux kernel headers
        assert_eq!(IFF_TAP, 0x0002);
        assert_eq!(IFF_NO_PI, 0x1000);
    }

    #[test]
    fn max_frame_size_is_ethernet_max() {
        // Standard Ethernet MTU 1500 + 14-byte header + 4-byte VLAN tag
        assert_eq!(MAX_FRAME_SIZE, 1518);
    }

    // Integration tests that require CAP_NET_ADMIN are in tests/ directory
    // and gated with #[ignore].

    #[test]
    #[ignore = "requires CAP_NET_ADMIN"]
    fn tap_create_and_destroy() {
        let tap = TapDevice::create("nexus_test0").expect("Failed to create TAP");
        assert_eq!(tap.name(), "nexus_test0");

        // Verify interface exists
        let output = std::process::Command::new("ip")
            .args(["link", "show", "nexus_test0"])
            .output()
            .unwrap();
        assert!(output.status.success(), "Interface should exist");

        // Destroy
        tap.destroy().expect("Failed to destroy TAP");
    }

    #[test]
    #[ignore = "requires CAP_NET_ADMIN"]
    fn tap_set_address_and_up() {
        let tap = TapDevice::create("nexus_test1").expect("Failed to create TAP");
        tap.set_address("10.99.0.1/24").expect("Failed to set address");
        tap.set_up().expect("Failed to bring up");

        // Verify address
        let output = std::process::Command::new("ip")
            .args(["addr", "show", "nexus_test1"])
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("10.99.0.1"), "Address should be configured");
    }
}
