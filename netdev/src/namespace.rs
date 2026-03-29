//! Linux network namespace management.
//!
//! Each simulated node can be placed in its own network namespace, giving it
//! an isolated network stack. Protocols inside the namespace see only the
//! interfaces assigned to that namespace.

use anyhow::{Context, Result, bail};

/// A handle to a Linux network namespace.
///
/// On drop, the namespace is deleted (best-effort).
pub struct Namespace {
    name: String,
    /// Whether we own this namespace (created it) and should delete on drop.
    owned: bool,
}

impl Namespace {
    /// Create a new network namespace with the given name.
    ///
    /// Requires `CAP_SYS_ADMIN` or root.
    pub fn create(name: &str) -> Result<Self> {
        let output = std::process::Command::new("ip")
            .args(["netns", "add", name])
            .output()
            .context("Failed to run 'ip netns add'")?;
        if !output.status.success() {
            bail!(
                "ip netns add {} failed: {}",
                name,
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        tracing::info!("Created network namespace: {name}");
        Ok(Self {
            name: name.to_string(),
            owned: true,
        })
    }

    /// Get the namespace name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Move a network interface into this namespace.
    pub fn move_interface(&self, ifname: &str) -> Result<()> {
        let output = std::process::Command::new("ip")
            .args(["link", "set", ifname, "netns", &self.name])
            .output()
            .context("Failed to run 'ip link set netns'")?;
        if !output.status.success() {
            bail!(
                "ip link set {} netns {} failed: {}",
                ifname,
                self.name,
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(())
    }

    /// Configure an IP address on an interface inside this namespace.
    pub fn configure_address(&self, ifname: &str, addr_cidr: &str) -> Result<()> {
        let output = std::process::Command::new("ip")
            .args([
                "netns", "exec", &self.name, "ip", "addr", "add", addr_cidr, "dev", ifname,
            ])
            .output()
            .context("Failed to configure address in namespace")?;
        if !output.status.success() {
            bail!(
                "ip addr add {} dev {} in netns {} failed: {}",
                addr_cidr,
                ifname,
                self.name,
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(())
    }

    /// Bring an interface up inside this namespace.
    pub fn set_interface_up(&self, ifname: &str) -> Result<()> {
        let output = std::process::Command::new("ip")
            .args([
                "netns", "exec", &self.name, "ip", "link", "set", ifname, "up",
            ])
            .output()
            .context("Failed to bring up interface in namespace")?;
        if !output.status.success() {
            bail!(
                "ip link set {} up in netns {} failed: {}",
                ifname,
                self.name,
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(())
    }

    /// Bring loopback up inside this namespace (required for localhost).
    pub fn set_loopback_up(&self) -> Result<()> {
        self.set_interface_up("lo")
    }

    /// Build a command that executes inside this namespace.
    ///
    /// Usage:
    /// ```ignore
    /// let mut cmd = ns.exec("bash");
    /// cmd.arg("-c").arg("echo hello");
    /// let child = cmd.spawn()?;
    /// ```
    pub fn exec(&self, program: &str) -> std::process::Command {
        let mut cmd = std::process::Command::new("ip");
        cmd.args(["netns", "exec", &self.name, program]);
        cmd
    }

    /// Build a bash command string prefix for entering this namespace.
    ///
    /// Returns a string like `"ip netns exec nexus_node0 "` that can be
    /// prepended to the existing bash wrapper in `runner/src/lib.rs`.
    pub fn bash_prefix(&self) -> String {
        format!("ip netns exec {} ", self.name)
    }

    /// Delete this namespace.
    pub fn delete(&self) -> Result<()> {
        let output = std::process::Command::new("ip")
            .args(["netns", "delete", &self.name])
            .output()
            .context("Failed to run 'ip netns delete'")?;
        if !output.status.success() {
            // May already be deleted
            tracing::debug!(
                "ip netns delete {} returned: {}",
                self.name,
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(())
    }

    /// Check if a namespace with the given name exists.
    pub fn exists(name: &str) -> bool {
        std::process::Command::new("ip")
            .args(["netns", "list"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).lines().any(|l| l.starts_with(name)))
            .unwrap_or(false)
    }
}

impl Drop for Namespace {
    fn drop(&mut self) {
        if self.owned {
            let _ = std::process::Command::new("ip")
                .args(["netns", "delete", &self.name])
                .output();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bash_prefix_format() {
        let ns = Namespace {
            name: "nexus_node0".to_string(),
            owned: false, // Don't actually create/delete
        };
        assert_eq!(ns.bash_prefix(), "ip netns exec nexus_node0 ");
    }

    #[test]
    fn name_accessor() {
        let ns = Namespace {
            name: "test_ns".to_string(),
            owned: false,
        };
        assert_eq!(ns.name(), "test_ns");
    }

    // Integration tests requiring CAP_SYS_ADMIN are gated with #[ignore].

    #[test]
    #[ignore = "requires CAP_SYS_ADMIN"]
    fn namespace_create_and_delete() {
        let ns = Namespace::create("nexus_test_ns").expect("Failed to create namespace");
        assert!(Namespace::exists("nexus_test_ns"));
        ns.delete().expect("Failed to delete namespace");
        assert!(!Namespace::exists("nexus_test_ns"));
    }

    #[test]
    #[ignore = "requires CAP_SYS_ADMIN + CAP_NET_ADMIN"]
    fn namespace_with_interface() {
        use crate::tap::TapDevice;

        let tap = TapDevice::create("nexus_testtap").expect("Failed to create TAP");
        let ns = Namespace::create("nexus_test_ns2").expect("Failed to create namespace");

        // Move TAP into namespace
        ns.move_interface("nexus_testtap")
            .expect("Failed to move interface");

        // Configure inside namespace
        ns.configure_address("nexus_testtap", "10.99.1.1/24")
            .expect("Failed to configure address");
        ns.set_interface_up("nexus_testtap")
            .expect("Failed to bring up");

        // Verify inside namespace
        let output = std::process::Command::new("ip")
            .args([
                "netns", "exec", "nexus_test_ns2", "ip", "addr", "show", "nexus_testtap",
            ])
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("10.99.1.1"),
            "Address should be visible inside namespace"
        );

        // Cleanup (drop order matters: namespace deletion removes contained interfaces)
        std::mem::forget(tap); // Don't try to delete interface from host (it's in the namespace)
        ns.delete().expect("Failed to delete namespace");
    }
}
